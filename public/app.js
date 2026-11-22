"use strict";

const $ = (id) => document.getElementById(id);

// Phone-first: 8pt keeps a wide omp session readable without horizontal scroll.
const DEFAULT_PREFS = { fps: 5, font: 8.0, keybar: true, awake: false };

function loadPrefs() {
  try {
    const stored = JSON.parse(localStorage.getItem("ra.prefs") || "{}");
    // One-time migration: the old phone default (11pt) was too large for a
    // wide omp session, so move it to the new 8pt baseline.
    if (!stored.__fontV2 && stored.font === 11) {
      stored.font = DEFAULT_PREFS.font;
    }
    stored.__fontV2 = true;
    localStorage.setItem("ra.prefs", JSON.stringify(stored));
    return { ...DEFAULT_PREFS, ...stored };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

const state = {
  started: false,
  page: "home",
  view: (() => {
    const saved = localStorage.getItem("ra.view");
    // "split" was the default in older builds; CLI is the entry point now.
    return saved && saved !== "split" ? saved : "term";
  })(),
  status: null,
  prefs: loadPrefs(),
  minFrameMs: 200,

  screenWs: null,
  screenTimer: null,
  decoding: false,
  pendingFrame: null,
  frameTimer: null,
  imgRect: null,
  frameTimes: [],
  lastFrame: 0,
  lastDrawAt: 0,

  term: null,
  fit: null,
  termWs: null,
  termTimer: null,

  projects: [],
  projectId: localStorage.getItem("ra.project") || "",
  projectName: "",

  pointers: new Map(),
  dragPointer: null,
  /// Trackpad mode: the pointer moves relative to finger movement, so the
  /// finger never lands on top of the cursor it is trying to see.
  cursor: { x: 0.5, y: 0.5 },
  /// "pad" = drag moves the pointer; "scroll" = drag scrolls the remote view.
  inputMode: "pad",
  hoverPointer: null,
  hoverLast: null,
  hoverMovedAt: 0,
  longPressTimer: null,
  tapStart: null,
  twoFinger: false,
  lastPoint: null,
  lastScroll: null,
  scrollCenter: null,
  lastMouse: 0,
  lastTapAt: 0,

  ctrlArmed: false,
  mods: { ctrl: false, alt: false, shift: false, altgr: false, cmd: false },

  statsTimer: null,
  homeTimer: null,
  statsBusy: false,
  wakeLock: null,
};

state.minFrameMs = Math.max(120, Math.round(1000 / state.prefs.fps));
try {
  const savedMode = localStorage.getItem("ra.inputMode");
  if (savedMode === "scroll" && state.inputMode === "pad") $("btnInputMode")?.click();
} catch {}

const canvas = $("screen");
const ctx = canvas.getContext("2d", { alpha: false });
const kbd = $("kbd");

const PAGES = { home: "Home", remote: "Remote", system: "System", projects: "Projects", files: "Files", settings: "Settings" };

/* A paired device keeps its bearer key on this phone. Logging out ends the
   session; it must not force a second pairing from the same device. */
const TRUST_KEY = "ra.keys";
function trustedKeys() {
  try { return JSON.parse(localStorage.getItem(TRUST_KEY) || "{}"); } catch { return {}; }
}
function trustDevice(deviceId, clientKey) {
  if (!deviceId || !clientKey) return;
  const map = trustedKeys();
  map[String(deviceId)] = clientKey;
  try { localStorage.setItem(TRUST_KEY, JSON.stringify(map)); } catch {}
}
/// Re-install client cookies from the trusted keys. The relay reads them on the
/// very next request, so a returning phone never sees the pairing screen.
function restoreTrustedCookies() {
  for (const [deviceId, key] of Object.entries(trustedKeys())) {
    const name = `ra_c_${deviceId}_${String(key).slice(0, 10)}`;
    if (document.cookie.includes(`${name}=`)) continue;
    document.cookie = `${name}=${key}; path=/; max-age=31536000; SameSite=Lax; Secure`;
  }
}


/* ================= auth ================= */

async function api(path, options = {}) {
  const res = await fetch(path, { credentials: "same-origin", cache: "no-store", ...options });
  // The relay routes /api/* by the ra_device cookie. If that cookie was dropped
  // (clear-site-data, logout in another tab), recover it once instead of
  // showing "connecting" forever against an HTML error page.
  if (res.status === 401 && path.startsWith("/api/")) {
    let body = "";
    try { body = await res.clone().text(); } catch {}
    if (body.includes("no_device") || body === "") {
      if (await rememberRelayDevice("")) {
        return fetch(path, { credentials: "same-origin", ...options });
      }
    }
  }
  return res;
}

async function boot() {
  const wantsPicker = new URLSearchParams(location.search).get("picker") === "1";
  if (wantsPicker) {
    showLogin();
    return;
  }
  // Trusted keys stay dormant on boot: logout must actually end the session.
  // They are replayed only when the user taps that device in the picker.
  if (window.__RA_DEVICE && !/(^|;\s*)ra_device=/.test(document.cookie)) {
    document.cookie = `ra_device=${window.__RA_DEVICE}; path=/; max-age=31536000; SameSite=Lax; Secure`;
  }
  if (new URLSearchParams(location.search).get("offline") === "1") {
    showToast("Perangkat sedang offline. Nyalakan app desktop.", 5000);
  }
  // A QR link lands here (the relay redirects /pair and keeps #c=), so redeem
  // it in this same UI instead of rendering a second pairing page.
  const linked = pairInfoFromQr(location.href);
  if (linked.code) {
    showLogin();
    if (await claimPairingCode(linked.code, linked.key)) {
      history.replaceState(null, "", "/");
    }
    return;
  }
  try {
    const res = await api("/api/status");
    if (res.status === 409) {
      showMoved();
      return;
    }
    if (res.ok) {
      state.status = await res.json();
      await start();
      return;
    }
    if (res.status === 409) {
      showMoved();
      return;
    }
    showLogin();
  } catch {
    showLogin();
  }
}

// The relay allows one active device at a time. When another device takes over,
// this tab stops polling and offers the device picker instead of a login form.
function showMoved() {
  movedGateActive = true;
  $("login").classList.add("hidden");
  $("app").classList.add("hidden");
  let gate = $("movedGate");
  if (!gate) {
    gate = document.createElement("div");
    gate.id = "movedGate";
    gate.className = "login";
    gate.innerHTML = `
      <img class="login-mark" src="/icons/icon-192.png" alt="Remote Agents" width="56" height="56">
      <h1>Sesi pindah</h1>
      <p class="login-sub">Perangkat ini tidak lagi aktif. Satu waktu hanya satu perangkat yang bisa dipakai.</p>
      <a class="btn primary" href="/?picker=1">Pilih perangkat</a>`;
    document.body.append(gate);
  }
  gate.classList.remove("hidden");
}

function showLogin() {
  $("login").classList.remove("hidden");
  $("app").classList.add("hidden");
  renderRecentDevices();
  setTimeout(() => $("token").focus(), 40);
}

$("token")?.addEventListener("input", () => {
  const input = $("token");
  const value = input.value;
  // Pairing codes are alphanumeric (e.g. ABC-123); device IDs are 9 digits.
  if (/^[A-Za-z0-9-\s]*$/.test(value) && /[A-Za-z]/.test(value)) {
    const raw = value.replace(/[^A-Za-z0-9]/g, "").toUpperCase().slice(0, 6);
    input.value = raw.length > 3 ? `${raw.slice(0, 3)}-${raw.slice(3)}` : raw;
    return;
  }
  // 9-digit ID: format as 123 456 789.
  if (/^[\d\s]{1,11}$/.test(value)) {
    const raw = value.replace(/\D/g, "").slice(0, 9);
    input.value = raw.replace(/(\d{3})(?=\d)/g, "$1 ");
  }
});

$("loginForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  const raw = $("token").value.replace(/\s/g, "").trim();
  if (!raw) return;
  $("loginErr").textContent = "";

  // A 9-digit value is the device ID (or a long token); an alphanumeric code is
  // a one-time pairing code and must be redeemed instead.
  const looksLikeCode = /^[A-Za-z0-9-]{4,8}$/.test(raw) && /[A-Za-z]/.test(raw);
  if (looksLikeCode) {
    const code = raw.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
    if (!(await claimPairingCode(code))) {
      $("loginErr").textContent = "Kode salah atau kedaluwarsa.";
      return;
    }
    $("token").value = "";
    return;
  }

  const res = await api("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token: raw }),
  });
  if (!res.ok) {
    $("loginErr").textContent = res.status === 429 ? "Terlalu banyak percobaan." : "Token salah.";
    return;
  }
  $("token").value = "";
  await start();
});

function deviceLabel() {
  const ua = navigator.userAgent;
  return /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android" : "Browser";
}

async function start() {
  if (state.started) return;
  state.started = true;
  $("login").classList.add("hidden");
  $("app").classList.remove("hidden");
  await loadProjects();
  initSidebar();
  const initialPage = new URLSearchParams(location.search).get("page");
  if (initialPage && PAGES[initialPage]) go(initialPage);
  applyPrefs();
  try {
    const saved = localStorage.getItem(`ra.resume.${state.projectId || "default"}`);
    if (saved) state.resumeSession = saved;
  } catch {}
  applyView();
  go("home");
  refreshStatus();
  setInterval(refreshStatus, 5000);
}

/* ================= navigation ================= */

function go(page) {
  if (!PAGES[page]) return;
  state.page = page;
  document.querySelectorAll(".view").forEach((view) => {
    view.classList.toggle("active", view.id === `view-${page}`);
  });
  document.querySelectorAll(".bottom-nav .nav-btn, .side-nav .nav-btn").forEach((button) => {
    button.classList.toggle("active", button.dataset.page === page);
  });
  $("pageTitle").textContent = PAGES[page];

  if (page === "remote") {
    syncScreen();
    setTimeout(fitTerm, 80);
  } else {
    closeScreen();
  }
  $("filesPane")?.classList.toggle("hidden", !(page === "remote" && state.view === "files"));
  if (page === "home") startHome();
  else stopHome();
  if (page === "system") startStats();
  else stopStats();
  if (page === "projects") renderProjects();
  if (page === "files" && typeof loadFilesPage === "function") loadFilesPage();
  if (page === "settings") renderSettings();
  applyWake();
}

function bindNavigation() {
  document.querySelectorAll("[data-page]").forEach((button) => {
    button.addEventListener("click", () => go(button.dataset.page));
  });
}

/* ================= status ================= */

let movedGateActive = false;

async function refreshStatus() {
  if (movedGateActive) return;
  try {
    const res = await api("/api/status");
    if (!res.ok) {
      if (res.status === 401) showLogin();
      if (res.status === 409) showMoved();
      return;
    }
    state.status = await res.json();
    updateStatus();
  } catch {
    setStatus("offline", "warn");
  }
}

function updateStatus() {
  const s = state.status;
  if (!s) return;
  const capture = s.capture || {};
  const input = s.input || {};

  if (capture.ok) {
    if (state.page === "remote") setStatus("live", "ok");
    else setStatus("standby", "ok");
    $("liveDot").classList.add("on");
    const fps = state.frameTimes.length > 1
      ? (state.frameTimes.length - 1) / ((state.frameTimes[state.frameTimes.length - 1] - state.frameTimes[0]) / 1000)
      : 0;
    $("screenMeta").textContent = `${capture.width}×${capture.height} · ${fps > 0.2 ? fps.toFixed(1) : capture.fps}fps · q${capture.quality}`;
  } else {
    setStatus(capture.error ? "screen permission" : "waiting", "warn");
    $("liveDot").classList.remove("on");
    $("screenMeta").textContent = capture.error || "—";
  }

  $("sideNote").textContent = `${s.session || "omp"} · ${s.cwd || ""}`;
  $("homeProject").textContent = state.projectName || s.session || "omp";
  const uptime = s.uptime || 0;
  $("homeUptime").textContent = `up ${Math.floor(uptime / 3600)}h ${Math.floor((uptime % 3600) / 60)}m`;
  // Reaching this code means /api/status answered, so the device IS online.
  // capture.ok only says whether the screen pipeline is streaming right now,
  // which is lazy: it starts when a viewer connects.
  const captureIssue = !!capture.error;
  $("heroState").textContent = captureIssue ? "perlu izin" : "online";
  $("heroState").className = `pill ${captureIssue ? "warn" : "ok"}`;
  $("homeSub").textContent = capture.ok
    ? `omp ready · screen ${capture.width}×${capture.height} · ${s.screenClients || 0} viewer`
    : (capture.error || "menunggu screen capture");

  if (input.trusted === false) {
    showToast("Accessibility belum diizinkan: mouse/keyboard layar tidak aktif.", 8000);
  }
}

function setStatus(text, cls) {
  $("statusText").textContent = text;
  $("status").querySelector(".dot").className = `dot ${cls || ""}`;
  $("sideStatus").textContent = text;
}

function showToast(text, ms = 3800) {
  const el = $("toast");
  el.textContent = text;
  el.classList.remove("hidden");
  clearTimeout(showToast.timer);
  showToast.timer = setTimeout(() => el.classList.add("hidden"), ms);
}

/* ================= home ================= */

function startHome() {
  refreshHome();
  clearInterval(state.homeTimer);
  state.homeTimer = setInterval(refreshHome, 5000);
  renderHomeProjects();
}
function stopHome() {
  clearInterval(state.homeTimer);
  state.homeTimer = null;
}

async function refreshHome() {
  try {
    const res = await api("/api/system");
    if (!res.ok) return;
    const s = await res.json();
    if (s.host) $("homeHost").textContent = s.host.replace(".local", "");
    updateHomeStats(s);
  } catch {}
}

function updateHomeStats(s) {
  const cpu = Math.round(s.cpu || 0);
  const ram = s.mem_total ? (s.mem_used / s.mem_total) * 100 : 0;
  const disk = s.disk_total ? (s.disk_used / s.disk_total) * 100 : 0;
  $("homeCpu").textContent = `${cpu}%`;
  $("homeCpuBar").style.width = `${Math.min(100, cpu)}%`;
  $("homeRam").textContent = `${Math.round(ram)}%`;
  $("homeRamBar").style.width = `${Math.min(100, ram)}%`;
  $("homeDisk").textContent = `${Math.round(disk)}%`;
  $("homeDiskBar").style.width = `${Math.min(100, disk)}%`;
  $("homeNet").textContent = fmtRate((s.net_rx_bps || 0) + (s.net_tx_bps || 0));
  $("homeNetSub").textContent = `↓ ${fmtRate(s.net_rx_bps || 0)} · ↑ ${fmtRate(s.net_tx_bps || 0)}`;
}

function renderHomeProjects() {
  const host = $("homeProjects");
  host.textContent = "";
  for (const project of state.projects.slice(0, 4)) {
    host.append(projectCard(project, true));
  }
}

/* ================= projects ================= */

async function loadProjects() {
  try {
    const res = await api("/api/projects");
    if (!res.ok) return;
    const data = await res.json();
    state.projects = data.projects || [];
    if (!state.projectId || !state.projects.some((p) => p.id === state.projectId)) {
      state.projectId = data.default || state.projects[0]?.id || "";
    }
    renderProjects();
    renderHomeProjects();
    updateProjectLabels();
  } catch {}
}

function updateProjectLabels() {
  const project = state.projects.find((p) => p.id === state.projectId);
  state.projectName = project?.name || state.projectId || "omp";
  state.projectPath = project?.path || "";
  const label = $("remoteProjectName") || $("remoteProject");
  label.textContent = state.projectName || "—";
  $("remoteProject").title = project?.path || "";
  $("homeProject").textContent = state.projectName;
}

function projectCard(project, compact = false) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "project-card" + (project.id === state.projectId ? " current" : "");
  const name = document.createElement("span");
  name.className = "pc-name";
  name.textContent = project.name;
  const path = document.createElement("span");
  path.className = "pc-path";
  path.textContent = project.path;
  button.append(name, path);
  if (!compact) {
    const open = document.createElement("span");
    open.className = "meta";
    open.textContent = project.id === state.projectId ? "aktif · buka remote" : "tap untuk buka";
    button.append(open);
  }
  button.addEventListener("click", () => openProject(project.id));
  const canEdit = project.id && project.id !== "workspace";
  if (!canEdit) return button;
  return attachSwipe(button, [{
    label: "Hapus",
    danger: true,
    icon: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13M10 11v5M14 11v5"/></svg>',
    onTap: () => deleteProject(project),
  }]);
}

async function deleteProject(project) {
  const ok = await askConfirm({
    title: "Hapus dari daftar?",
    message: `"${project.name || project.id}" dihapus dari daftar project. Folder di laptop TIDAK dihapus.`,
    okLabel: "Hapus",
    danger: true,
  });
  if (!ok) return;
  const res = await api(`/api/projects/${encodeURIComponent(project.id)}`, { method: "DELETE" });
  if (!res.ok) {
    showToast((await res.text()) || "gagal menghapus", 4200);
    return;
  }
  if (state.projectId === project.id) {
    state.projectId = "";
    state.resumeSession = null;
    closeTerminal();
  }
  await loadProjects();
  showToast("Dihapus dari daftar", 2600);
}

/* ================= dialog ================= */

let dialogResolve = null;

function showDialog({ title, message, value, okLabel, cancelLabel, danger, withInput }) {
  const modal = $("dlgModal");
  $("dlgTitle").textContent = title || "";
  const msg = $("dlgMsg");
  msg.textContent = message || "";
  msg.classList.toggle("hidden", !message);
  const input = $("dlgInput");
  input.classList.toggle("hidden", !withInput);
  input.value = value || "";
  const ok = $("dlgOk");
  ok.textContent = okLabel || (withInput ? "Simpan" : "Lanjut");
  ok.classList.toggle("danger", !!danger);
  $("dlgCancel").textContent = cancelLabel || "Batal";
  modal.classList.remove("hidden");
  if (withInput) setTimeout(() => { input.focus(); input.select(); }, 60);

  return new Promise((resolve) => {
    dialogResolve = (result) => {
      dialogResolve = null;
      modal.classList.add("hidden");
      resolve(result);
    };
  });
}

function closeDialog(result) {
  if (dialogResolve) dialogResolve(result);
}

const askText = (opts) => showDialog({ ...opts, withInput: true });
const askConfirm = (opts) => showDialog({ ...opts, withInput: false }).then(Boolean);

$("dlgOk")?.addEventListener("click", () => {
  const withInput = !$("dlgInput").classList.contains("hidden");
  closeDialog(withInput ? $("dlgInput").value : true);
});
$("dlgCancel")?.addEventListener("click", () => closeDialog(null));
$("dlgModal")?.addEventListener("click", (event) => {
  if (event.target === $("dlgModal")) closeDialog(null);
});
$("dlgInput")?.addEventListener("keydown", (event) => {
  if (event.key === "Enter") { event.preventDefault(); closeDialog($("dlgInput").value); }
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && dialogResolve) closeDialog(null);
});

/* ================= swipe actions ================= */

/// Reveals destructive actions under a row. Horizontal drags only; vertical
/// stays with the page so the list still scrolls.
function attachSwipe(row, actions) {
  if (!actions?.length) return row;
  const wrap = document.createElement("div");
  wrap.className = "swipe";
  const bar = document.createElement("div");
  bar.className = "swipe-actions";
  const width = actions.length * 52;
  bar.style.width = `${width}px`;

  let open = false;
  const setOpen = (next) => {
    open = next;
    row.style.transform = `translateX(${next ? -width : 0}px)`;
  };
  for (const action of actions) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `swipe-btn${action.danger ? " danger" : ""}`;
    btn.setAttribute("aria-label", action.label);
    btn.title = action.label;
    btn.innerHTML = action.icon;
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      setOpen(false);
      action.onTap();
    });
    bar.append(btn);
  }

  let startX = 0, startY = 0, base = 0, dragging = false, locked = false;
  row.addEventListener("pointerdown", (event) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    startX = event.clientX;
    startY = event.clientY;
    base = open ? -width : 0;
    dragging = true;
    locked = false;
  });
  row.addEventListener("pointermove", (event) => {
    if (!dragging) return;
    const dx = event.clientX - startX;
    const dy = event.clientY - startY;
    if (!locked) {
      if (Math.abs(dx) < 6 && Math.abs(dy) < 6) return;
      if (Math.abs(dy) > Math.abs(dx)) { dragging = false; return; }
      locked = true;
      row.setPointerCapture?.(event.pointerId);
    }
    const next = Math.min(0, Math.max(-width, base + dx));
    row.style.transition = "none";
    row.style.transform = `translateX(${next}px)`;
  });
  const finish = () => {
    if (!dragging) return;
    dragging = false;
    row.style.transition = "";
    const current = Number((row.style.transform.match(/-?\d+/) || [0])[0]);
    setOpen(current < -width / 2);
    if (locked) {
      // Swallow the click that follows a drag so the row does not navigate.
      const swallow = (event) => { event.stopPropagation(); event.preventDefault(); };
      row.addEventListener("click", swallow, { capture: true, once: true });
    }
  };
  row.addEventListener("pointerup", finish);
  row.addEventListener("pointercancel", finish);

  wrap.append(bar, row);
  return wrap;
}

function renderProjects() {
  const list = $("projectList");
  if (!list) return;
  list.textContent = "";
  for (const project of state.projects) list.append(projectCard(project));

  renderHomeProjects();
}

function openProject(id) {
  state.projectId = id;
  localStorage.setItem("ra.project", id);
  updateProjectLabels();
  closeTerminal();
  state.resumeSession = null;
  go("remote");
  // The CLI view starts with a picker: resume a session or start a new one.
  if (state.view === "term" || state.view === "split") {
    showTermEmpty();
  }
}

/* ================= folder picker (add project) ================= */

let browseCurrent = "";
let browseParent = null;

async function openBrowsePicker() {
  await loadBrowse("~");
  $("browseModal").classList.remove("hidden");
}

async function loadBrowse(path) {
  const list = $("browseList");
  if (!list) return;
  list.textContent = "";
  const res = await api(`/api/browse?path=${encodeURIComponent(path)}`);
  if (!res.ok) {
    showToast("Folder tidak bisa dibuka", 3200);
    return;
  }
  const data = await res.json();
  browseCurrent = data.path || "";
  browseParent = data.parent || null;
  const shown = browseCurrent.replace(/^\/Users\/[^/]+/, "~");
  $("browsePath").textContent = shown;
  $("browseUp").disabled = !browseParent;

  if (!data.dirs?.length) {
    const empty = document.createElement("div");
    empty.className = "browse-empty";
    empty.textContent = "Tidak ada subfolder di sini.";
    list.append(empty);
    return;
  }
  for (const dir of data.dirs) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "browse-row";
    const icon = document.createElement("span");
    icon.textContent = "📁";
    const label = document.createElement("span");
    label.textContent = dir.name;
    const chev = document.createElement("span");
    chev.textContent = "›";
    chev.style.color = "var(--dim)";
    row.append(icon, label, chev);
    row.addEventListener("click", () => loadBrowse(dir.abs));
    list.append(row);
  }
}

$("addProjectBtn")?.addEventListener("click", openBrowsePicker);
$("browseClose")?.addEventListener("click", () => $("browseModal").classList.add("hidden"));
$("browseUp")?.addEventListener("click", () => { if (browseParent) loadBrowse(browseParent); });
// Picking IS the submit: the folder name becomes the project name on disk.
$("browsePick")?.addEventListener("click", async () => {
  if (!browseCurrent) return;
  const folder = browseCurrent;
  $("browseModal").classList.add("hidden");
  const res = await api("/api/projects", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path: folder }),
  });
  if (!res.ok) {
    showToast((await res.text()) || "gagal menambah project", 5000);
    return;
  }
  const project = await res.json();
  await loadProjects();
  openProject(project.id);
  showToast(`Ditambahkan: ${project.name}`, 2600);
});

$("remoteProject").addEventListener("click", () => go("projects"));

/* ================= screen stream ================= */

function wsUrl(path) {
  return `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}${path}`;
}

function wantScreen() {
  return state.page === "remote" && state.view !== "term" && !document.hidden;
}

function syncScreen() {
  if (wantScreen()) {
    if (!state.screenWs || state.screenWs.readyState > WebSocket.OPEN) connectScreen();
  } else {
    closeScreen();
  }
  applyWake();
}

function closeScreen() {
  clearTimeout(state.screenTimer);
  if (state.screenWs) {
    state.screenWs.onclose = null;
    try { state.screenWs.close(); } catch {}
    state.screenWs = null;
  }
  state.pendingFrame = null;
  $("liveDot").classList.remove("on");
}

function connectScreen() {
  clearTimeout(state.screenTimer);
  if (state.screenWs && state.screenWs.readyState <= WebSocket.OPEN) return;
  try {
    const ws = new WebSocket(wsUrl("/ws/screen"));
    ws.binaryType = "arraybuffer";
    state.screenWs = ws;
    ws.onopen = () => setStatus("live", "ok");
    ws.onclose = () => {
      state.screenWs = null;
      if (wantScreen()) state.screenTimer = setTimeout(connectScreen, 1200);
    };
    ws.onerror = () => ws.close();
    ws.onmessage = (event) => {
      if (typeof event.data === "string") return;
      state.lastFrame = performance.now();
      state.frameTimes.push(state.lastFrame);
      if (state.frameTimes.length > 30) state.frameTimes.shift();
      queueFrame(event.data);
    };
  } catch {
    state.screenTimer = setTimeout(connectScreen, 1500);
  }
}

function queueFrame(buffer) {
  const now = performance.now();
  if (state.decoding || now - state.lastDrawAt < state.minFrameMs) {
    state.pendingFrame = buffer;
    scheduleFrame();
    return;
  }
  decodeFrame(buffer);
}

function scheduleFrame() {
  clearTimeout(state.frameTimer);
  const wait = Math.max(30, state.minFrameMs - (performance.now() - state.lastDrawAt));
  state.frameTimer = setTimeout(() => {
    if (state.decoding || !state.pendingFrame) return;
    const next = state.pendingFrame;
    state.pendingFrame = null;
    decodeFrame(next);
  }, wait);
}

async function decodeFrame(buffer) {
  state.decoding = true;
  // Watchdog: if a decode hangs, the stream would freeze forever. Release the
  // lock and let the next frame through instead.
  const watchdog = setTimeout(() => {
    state.decoding = false;
    state.lastDrawAt = performance.now();
    if (state.pendingFrame) scheduleFrame();
  }, 1500);
  try {
    const blob = new Blob([buffer], { type: "image/jpeg" });
    let source;
    if (typeof createImageBitmap === "function") source = await createImageBitmap(blob);
    else source = await loadImage(blob);
    drawFrame(source);
    if (source.close) source.close();
    state.framesDrawn = (state.framesDrawn || 0) + 1;
  } catch {
  } finally {
    clearTimeout(watchdog);
    state.decoding = false;
    state.lastDrawAt = performance.now();
    // Keep only the newest pending frame; older ones are already stale.
    if (state.pendingFrame) scheduleFrame();
  }
}

function loadImage(blob) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => { URL.revokeObjectURL(img.src); resolve(img); };
    img.onerror = reject;
    img.src = URL.createObjectURL(blob);
  });
}

function drawFrame(source) {
  const wrap = $("screenWrap");
  const cw = wrap.clientWidth;
  const ch = wrap.clientHeight;
  if (cw < 2 || ch < 2) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const pw = Math.round(cw * dpr);
  const ph = Math.round(ch * dpr);
  if (canvas.width !== pw || canvas.height !== ph) {
    canvas.width = pw;
    canvas.height = ph;
  }
  const iw = source.width;
  const ih = source.height;
  // Fit width first on phones so text stays legible; allow vertical scroll/letterbox.
  const fitWidth = cw / iw <= ch / ih;
  const scale = fitWidth ? cw / iw : Math.min(cw / iw, ch / ih);
  const w = iw * scale;
  const h = ih * scale;
  const x = (cw - w) / 2;
  const y = fitWidth ? 0 : (ch - h) / 2;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = "#04060a";
  ctx.fillRect(0, 0, cw, ch);
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(source, x, y, w, h);
  state.imgRect = { x, y, w, h };
}

/* ================= pointer input ================= */

function normalized(event) {
  const rect = canvas.getBoundingClientRect();
  const px = event.clientX - rect.left;
  const py = event.clientY - rect.top;
  const ir = state.imgRect;
  if (!ir) return null;
  const nx = (px - ir.x) / ir.w;
  const ny = (py - ir.y) / ir.h;
  if (nx < 0 || nx > 1 || ny < 0 || ny > 1) return null;
  return { x: nx, y: ny };
}

function sendControl(payload) {
  if (state.screenWs && state.screenWs.readyState === WebSocket.OPEN) {
    state.screenWs.send(JSON.stringify(payload));
  }
}

function activeButton() {
  const active = document.querySelector("[data-btn].active");
  if (active?.dataset.btn === "right") return "right";
  return "left";
}

function centroid() {
  let x = 0, y = 0;
  for (const p of state.pointers.values()) { x += p.x; y += p.y; }
  const n = Math.max(1, state.pointers.size);
  return { x: x / n, y: y / n };
}

document.querySelectorAll("[data-btn]").forEach((button) => {
  button.addEventListener("click", () => {
    document.querySelectorAll("[data-btn]").forEach((b) => b.classList.remove("active"));
    button.classList.add("active");
  });
});

/// Move the virtual pointer by a finger delta. The gain is "screen fraction per
/// pixel of finger travel": 0.55 gives fine control on a phone, where a small
/// drag used to fling the cursor across the display.
const TRACKPAD_GAIN = 0.55;

function cursorToScreen() {
  const ir = state.imgRect;
  if (!ir) return null;
  return { x: state.cursor.x, y: state.cursor.y };
}

function moveCursorBy(dxPx, dyPx) {
  const wrap = $("screenWrap");
  const ir = state.imgRect;
  if (!ir || ir.w < 4 || ir.h < 4) return false;
  // Normalise so a full swipe across the panel crosses the whole screen.
  const nx = (dxPx / Math.max(ir.w, 1)) * TRACKPAD_GAIN;
  const ny = (dyPx / Math.max(ir.h, 1)) * TRACKPAD_GAIN;
  const before = { x: state.cursor.x, y: state.cursor.y };
  state.cursor.x = Math.min(1, Math.max(0, state.cursor.x + nx));
  state.cursor.y = Math.min(1, Math.max(0, state.cursor.y + ny));
  const moved = Math.abs(before.x - state.cursor.x) > 1e-4 || Math.abs(before.y - state.cursor.y) > 1e-4;
  if (moved) sendControl({ t: "move", x: state.cursor.x, y: state.cursor.y });
  return moved;
}

canvas.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  $("hint")?.classList.add("gone");
  state.pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });

  // Two fingers: scroll, never move the pointer.
  if (state.pointers.size === 2) {
    state.twoFinger = true;
    clearTimeout(state.longPressTimer);
    state.hoverPointer = null;
    state.scrollCenter = centroid();
    return;
  }
  if (state.twoFinger) return;

  if (document.querySelector("[data-btn].active")?.dataset.btn === "scroll") {
    state.dragPointer = event.pointerId;
    state.lastScroll = { x: event.clientX, y: event.clientY };
    canvas.setPointerCapture(event.pointerId);
    return;
  }

  // Scroll mode: one finger scrolls the remote view; the pointer stays put.
  if (state.inputMode === "scroll") {
    state.dragPointer = event.pointerId;
    state.lastScroll = { x: event.clientX, y: event.clientY };
    canvas.setPointerCapture(event.pointerId);
    return;
  }

  // One finger: the pointer moves relatively, so the finger can rest anywhere.
  state.hoverPointer = event.pointerId;
  state.hoverLast = { x: event.clientX, y: event.clientY };
  state.hoverMovedAt = performance.now();
  state.tapStart = { x: event.clientX, y: event.clientY, t: performance.now() };
  canvas.setPointerCapture(event.pointerId);

  // Long press = right click at the current pointer position.
  clearTimeout(state.longPressTimer);
  state.longPressTimer = setTimeout(() => {
    if (state.dragPointer !== null) return;
    const p = cursorToScreen();
    if (!p) return;
    state.longPressTimer = null;
    state.tapStart = null;
    sendControl({ t: "down", ...p, button: "right" });
    setTimeout(() => sendControl({ t: "up", ...p, button: "right" }), 60);
    showToast("klik kanan", 900);
  }, 550);
});

canvas.addEventListener("pointermove", (event) => {
  if (state.pointers.has(event.pointerId)) {
    state.pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  }
  if (state.twoFinger && state.pointers.size >= 2) {
    const c = centroid();
    if (state.scrollCenter) {
      sendControl({ t: "scroll", dx: (state.scrollCenter.x - c.x) * 1.6, dy: (state.scrollCenter.y - c.y) * 1.6 });
    }
    state.scrollCenter = c;
    return;
  }
  if (state.dragPointer === event.pointerId) {
    const prev = state.lastScroll || { x: event.clientX, y: event.clientY };
    sendControl({ t: "scroll", dx: (prev.x - event.clientX) * 2, dy: (prev.y - event.clientY) * 2 });
    state.lastScroll = { x: event.clientX, y: event.clientY };
    return;
  }

  // Trackpad hover: translate finger travel into pointer travel.
  if (state.hoverPointer === event.pointerId && state.hoverLast) {
    const dx = event.clientX - state.hoverLast.x;
    const dy = event.clientY - state.hoverLast.y;
    if (Math.abs(dx) + Math.abs(dy) > 0.5) {
      if (moveCursorBy(dx, dy)) {
        state.hoverMovedAt = performance.now();
        clearTimeout(state.longPressTimer);
      }
      state.hoverLast = { x: event.clientX, y: event.clientY };
    }
    return;
  }

  if (event.pointerType === "mouse") {
    const now = performance.now();
    if (now - state.lastMouse < 33) return;
    state.lastMouse = now;
    const p = normalized(event);
    if (p) {
      state.cursor = p;   // a real mouse maps 1:1
      sendControl({ t: "move", ...p });
    }
  }
});

function endPointer(event) {
  state.pointers.delete(event.pointerId);
  if (state.pointers.size < 2) {
    state.twoFinger = false;
    state.scrollCenter = null;
  }

  if (state.hoverPointer === event.pointerId) {
    clearTimeout(state.longPressTimer);
    const tap = state.tapStart;
    const moved = tap ? Math.hypot(event.clientX - tap.x, event.clientY - tap.y) : 999;
    const quick = tap ? performance.now() - tap.t < 300 : false;
    // A stationary touch is a click at the pointer, not at the finger.
    if (tap && moved < 8 && quick) {
      const p = cursorToScreen();
      if (p) {
        const now = performance.now();
        const doubleTap = state.lastTapAt && now - state.lastTapAt < 320;
        state.lastTapAt = now;
        const count = doubleTap ? 2 : 1;
        sendControl({ t: "click", ...p, button: activeButton(), count });
      }
    }
    state.hoverPointer = null;
    state.hoverLast = null;
    state.tapStart = null;
  }

  if (state.dragPointer === event.pointerId) {
    if (state.lastPoint) sendControl({ t: "up", ...state.lastPoint, button: activeButton() });
    state.dragPointer = null;
    state.lastScroll = null;
  }
}

canvas.addEventListener("pointerup", endPointer);
canvas.addEventListener("pointercancel", endPointer);
canvas.addEventListener("wheel", (event) => {
  event.preventDefault();
  sendControl({ t: "scroll", dx: event.deltaX, dy: event.deltaY });
}, { passive: false });
canvas.addEventListener("contextmenu", (event) => event.preventDefault());

/* ================= desktop keyboard (screen) ================= */

$("btnKbd").addEventListener("click", () => {
  const on = $("btnKbd").classList.toggle("active");
  $("btnKbd").textContent = on ? "⌨ ON" : "⌨";
  if (on) kbd.focus();
  else kbd.blur();
  updateCkbdTarget();
});

kbd.addEventListener("input", () => {
  const text = kbd.value;
  if (text) sendControl({ t: "text", text });
  kbd.value = "";
});

kbd.addEventListener("paste", (event) => {
  const text = event.clipboardData?.getData("text") || "";
  if (text) sendControl({ t: "text", text });
  event.preventDefault();
});

const SPECIAL = new Set([
  "Enter", "Tab", "Backspace", "Escape", "Delete",
  "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown",
  "Home", "End", "PageUp", "PageDown",
  "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
]);

kbd.addEventListener("keydown", (event) => {
  const mods = [];
  if (event.metaKey) mods.push("cmd");
  if (event.ctrlKey) mods.push("ctrl");
  if (event.altKey) mods.push("alt");
  if (event.shiftKey) mods.push("shift");
  if (mods.length || SPECIAL.has(event.key)) {
    sendControl({ t: "key", key: event.key, mods });
    event.preventDefault();
  }
});

/* ================= terminal ================= */

function fitTerm() {
  if (!state.fit) return;
  if ($("termView").classList.contains("hidden")) return;
  if ($("term").clientHeight < 36) return;
  try {
    state.fit.fit();
    state.term?.refresh(0, Math.max(0, (state.term.rows || 1) - 1));
  } catch {}
}

function initTerminal() {
  if (state.term) return;
  const term = new Terminal({
    cursorBlink: true,
    cursorStyle: "bar",
    fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
    fontSize: state.prefs.font,
    lineHeight: 1.2,
    scrollback: 6000,
    allowProposedApi: true,
    theme: {
      background: "#080a0f",
      foreground: "#e6e9ef",
      cursor: "#7ee2b8",
      selectionBackground: "#334155",
      black: "#1e2430",
      red: "#fb7185",
      green: "#7ee2b8",
      yellow: "#fbbf24",
      blue: "#66b6ff",
      magenta: "#c084fc",
      cyan: "#22d3ee",
      white: "#e6e9ef",
    },
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open($("term"));
  bindTermScrollGestures();
  state.term = term;
  state.fit = fit;

  // A pane that was hidden or zero-sized when xterm opened keeps a 0-row
  // canvas. Retry until it has real dimensions, then fit once more.
  const settle = (n = 0) => {
    const h = $("term").clientHeight;
    const w = $("term").clientWidth;
    if (w > 40 && h > 40) {
      try { fit.fit(); } catch {}
      term.refresh(0, Math.max(0, (term.rows || 1) - 1));
      if (term.rows < 3 && n < 8) setTimeout(() => settle(n + 1), 90);
      return;
    }
    if (n < 10) setTimeout(() => settle(n + 1), 90);
  };
  requestAnimationFrame(() => settle(0));

  term.onData((data) => {
    if (state.ctrlArmed) {
      state.ctrlArmed = false;
      $("btnCtrl")?.classList.remove("active");
      if (data.length === 1) data = String.fromCharCode(data.toUpperCase().charCodeAt(0) & 0x1f);
    } else if ((state.mods.alt || state.mods.altgr) && data.length === 1) {
      data = "\x1b" + data;
      ckClearMods();
    }
    termSend({ type: "input", data });
  });
  term.onResize(({ cols, rows }) => termSend({ type: "resize", cols, rows }));

  new ResizeObserver(() => {
    if ($("termPanel").clientHeight < 40) return;
    fitTerm();
  }).observe($("term"));
}

/* Terminal scrolling.
   omp runs in the alternate screen buffer (like vim/htop), so xterm keeps no
   scrollback of its own: scrollLines() would do nothing. Scrolling therefore has
   two paths, chosen at runtime:
     1. normal buffer  -> move xterm's own viewport
     2. alternate buffer -> send arrow keys / PageUp to the remote TUI        */
function termUsesAltBuffer() {
  const buf = state.term?.buffer?.active;
  return buf?.type === "alternate";
}

function sendTermKey(seq) {
  termSend({ type: "input", data: seq });
}

function termScroll(lines) {
  const term = state.term;
  if (!term) return;

  if (!termUsesAltBuffer()) {
    // xterm has real scrollback here.
    term.scrollLines?.(lines);
    termScrollState();
    return;
  }

  // Scrollback lives in tmux, so ask the server to move its viewport.
  termSend({ type: "scroll", dir: lines < 0 ? "up" : "down", lines: Math.abs(lines) });
  termScrollState();
}

function termPage(dir) {
  if (!termUsesAltBuffer()) {
    state.term?.scrollPages?.(dir);
    termScrollState();
    return;
  }
  termSend({ type: "scroll", dir: dir < 0 ? "up" : "down", lines: 12 });
  termScrollState();
}

function termScrollTop() {
  if (!termUsesAltBuffer()) {
    state.term?.scrollToTop?.();
  } else {
    termSend({ type: "scroll", dir: "top" });
  }
  termScrollState();
}

function termScrollEnd() {
  if (!termUsesAltBuffer()) {
    state.term?.scrollToBottom?.();
  } else {
    termSend({ type: "scroll", dir: "bottom" });
  }
  termScrollState();
}

function termScrollState() {
  const term = state.term;
  const label = $("termSizeLabel");
  if (!term || !label) return;
  const buf = term.buffer?.active;
  if (!buf) return;
  if (buf.type === "alternate") {
    label.textContent = `tui · ${state.prefs.font.toFixed(1)}pt · alt`;
    return;
  }
  const atBottom = buf.viewportY >= buf.baseY;
  label.textContent = atBottom
    ? `tui · ${state.prefs.font.toFixed(1)}pt`
    : `tui · ${buf.viewportY}/${buf.baseY}`;
}

function bindTermScrollGestures() {
  const box = $("termView");
  if (!box) return;
  box.dataset.scrollBound = "1";

  let startY = 0;
  let dragging = false;
  let accum = 0;

  box.addEventListener("touchstart", (event) => {
    if (event.touches.length !== 1) return;
    startY = event.touches[0].clientY;
    dragging = true;
    accum = 0;
  }, { passive: true });

  box.addEventListener("touchmove", (event) => {
    if (!dragging || event.touches.length !== 1) return;
    const y = event.touches[0].clientY;
    const dy = startY - y;
    // ~12px of finger travel per buffer line feels like a normal terminal.
    if (Math.abs(dy) > 12) {
      const lines = Math.round(dy / 12);
      if (lines !== 0) {
        accum += lines;
        termScroll(lines);
        startY = y;
      }
    }
    event.preventDefault();
  }, { passive: false });

  box.addEventListener("touchend", (event) => {
    // A tap (no drag) opens the keyboard for direct typing.
    if (dragging && Math.abs((event.changedTouches?.[0]?.clientY ?? 0) - startY) < 10) {
      state.term?.focus();
    }
    dragging = false;
  }, { passive: true });

  // Keyboard-friendly: PageUp/PageDown while the terminal has focus.
  box.addEventListener("keydown", (event) => {
    if (event.key === "PageUp") { termScroll(-10); event.preventDefault(); }
    if (event.key === "PageDown") { termScroll(10); event.preventDefault(); }
  });
}

$("termScrollUp")?.addEventListener("click", () => termPage(-1));
$("termScrollDown")?.addEventListener("click", () => termPage(1));
$("termScrollEnd")?.addEventListener("click", termScrollEnd);

$("termNew")?.addEventListener("click", () => startTerminalFromPicker(null));

function closeTerminal() {
  clearTimeout(state.termTimer);
  if (state.termWs) {
    state.termWs.onclose = null;
    try { state.termWs.close(); } catch {}
    state.termWs = null;
  }
}

function termSend(payload) {
  if (state.termWs && state.termWs.readyState === WebSocket.OPEN) {
    state.termWs.send(JSON.stringify(payload));
  }
}

/// Show "no session yet" with the project's existing sessions to resume.
let termSwitcher = false;

async function showTermEmpty(switching = termSwitcher) {
  const box = $("termEmpty");
  const list = $("termSessions");
  if (!box || !list) return;
  termSwitcher = !!switching;
  box.classList.remove("hidden");
  list.textContent = "";

  if ($("teProject")) {
    $("teProject").textContent = state.projectPath || state.projectName || state.projectId || "";
  }

  let sessions = [];
  try {
    const res = await api(`/api/project-sessions?path=${encodeURIComponent(state.projectId || "")}`);
    if (res.ok) {
      const data = await res.json();
      sessions = data.sessions || [];
    }
  } catch {}

  if (!sessions.length) {
    const none = document.createElement("div");
    none.className = "te-hint";
    none.textContent = "Belum ada sesi tersimpan di project ini.";
    list.append(none);
    return;
  }
  for (const session of sessions.slice(0, 8)) {
    const row = document.createElement("div");
    row.className = "te-session";

    const open = document.createElement("button");
    open.type = "button";
    open.className = "te-open";
    const name = document.createElement("span");
    name.className = "ts-name";
    name.textContent = session.title || session.id;
    const date = document.createElement("span");
    date.className = "ts-date";
    date.textContent = new Date((session.mtime || 0) * 1000).toLocaleDateString("id-ID");
    open.append(name, date);
    open.addEventListener("click", () => resumeTermSession(session));

    const rename = document.createElement("button");
    rename.type = "button";
    rename.className = "te-rename";
    rename.title = "Ganti nama sesi";
    rename.setAttribute("aria-label", "Ganti nama sesi");
    rename.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 20h4l10-10-4-4L4 16v4zM13 6l4 4"/></svg>';
    rename.addEventListener("click", (event) => {
      event.stopPropagation();
      renameSession(session);
    });

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "te-del";
    remove.title = "Hapus sesi";
    remove.setAttribute("aria-label", "Hapus sesi");
    remove.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13M10 11v5M14 11v5"/></svg>';
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      deleteSession(session);
    });

    const actions = document.createElement("div");
    actions.className = "te-actions";
    actions.append(rename, remove);
    row.append(open, actions);
    list.append(row);
  }
}

/// Rename is a label only: the .jsonl file keeps its name so omp can still
/// resume it. Stored server-side so every device sees the same title.
async function renameSession(session) {
  const current = session.title || session.id;
  const next = await askText({
    title: "Ganti nama sesi",
    message: "Nama tersimpan di file sesi, jadi omp juga memakai nama ini.",
    value: current,
    okLabel: "Simpan",
  });
  if (next === null) return;
  const label = String(next).trim();
  if (!label || label === current) return;
  const res = await api("/api/session-label", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path: session.path, label }),
  });
  if (!res.ok) {
    showToast("Gagal mengganti nama sesi", 3600);
    return;
  }
  showTermEmpty();
  showToast("Nama sesi diperbarui", 2400);
}

/// Deletes the session file itself, so omp no longer lists it either.
async function deleteSession(session) {
  const ok = await askConfirm({
    title: "Hapus sesi ini?",
    message: `"${session.title || session.id}" dihapus permanen dari riwayat omp.`,
    okLabel: "Hapus",
    danger: true,
  });
  if (!ok) return;
  const res = await api("/api/session-delete", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path: session.path }),
  });
  if (!res.ok) {
    showToast("Gagal menghapus sesi", 3600);
    return;
  }
  showTermEmpty();
  showToast("Sesi dihapus", 2400);
}

function hideTermEmpty() {
  $("termEmpty")?.classList.add("hidden");
  termSwitcher = false;
}

// Toolbar shortcut: opens the session list over a running terminal, so
// switching does not require leaving the project or using /resume in omp.
// Tapping it again closes the sheet, which is why it needs no ✕ button.
$("btnSessions")?.addEventListener("click", () => {
  const open = !$("termEmpty").classList.contains("hidden");
  if (open) hideTermEmpty();
  else showTermEmpty(true);
});

/// Start (or resume) the terminal from the session picker.
function startTerminalFromPicker(resumePath) {
  hideTermEmpty();
  state.resumeSession = resumePath || null;
  try {
    // Remember the chosen session per project: a dropped socket, a reload or a
    // layout switch must reattach it instead of dumping the user on the picker.
    const key = `ra.resume.${state.projectId || "default"}`;
    if (resumePath) localStorage.setItem(key, resumePath);
    else localStorage.removeItem(key);
  } catch {}
  closeTerminal();
  requestAnimationFrame(() => {
    initTerminal();
    // xterm needs a real size before connecting, or it reports 0 columns.
    const kick = (n = 0) => {
      fitTerm();
      if (n < 6 && (!state.term?.cols || state.term.cols < 10 || $("term").clientHeight < 36)) {
        setTimeout(() => kick(n + 1), 60 + n * 40);
        return;
      }
      connectTerminal();
      state.term?.focus();
    };
    setTimeout(() => kick(0), 40);
  });
}

/// Resume by reattaching tmux to the project's session (omp --resume).
function resumeTermSession(session) {
  startTerminalFromPicker(session.path);
}

function connectTerminal() {
  clearTimeout(state.termTimer);
  const term = state.term;
  if (!term) return;
  // One live terminal per page: a second socket means two tmux clients on the
  // same session, and closing either one used to tear down the other.
  const existing = state.termWs;
  if (existing && existing.readyState <= WebSocket.OPEN) return;
  if (existing) {
    existing.onclose = null;
    try { existing.close(); } catch {}
    state.termWs = null;
  }
  if (!term.cols || term.cols < 10 || !term.rows || term.rows < 3) {
    clearTimeout(state.termTimer);
    state.termTimer = setTimeout(connectTerminal, 150);
    return;
  }
  const cols = term.cols;
  const rows = term.rows;
  const project = encodeURIComponent(state.projectId || "");
  const resume = state.resumeSession ? `&resume=${encodeURIComponent(state.resumeSession)}` : "";
  const ws = new WebSocket(wsUrl(`/ws/term?cols=${cols}&rows=${rows}&project=${project}${resume}`));
  ws.binaryType = "arraybuffer";
  state.termWs = ws;
  ws.onopen = () => {
    ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
    term.focus();
  };
  ws.onmessage = (event) => {
    if (typeof event.data === "string") term.write(event.data);
    else term.write(new Uint8Array(event.data));
  };
  ws.onclose = () => {
    if (state.termWs !== ws) return;   // superseded by a newer socket
    state.termWs = null;
    term.write("\r\n\x1b[38;5;245m[terminal disconnected]\x1b[0m\r\n");
    state.termTimer = setTimeout(connectTerminal, 1500);
  };
  ws.onerror = () => ws.close();
}

/* ================= quick key bar ================= */

const TERM_KEYS = {
  esc: "\x1b", tab: "\t", enter: "\r", up: "\x1b[A", down: "\x1b[B",
  right: "\x1b[C", left: "\x1b[D", home: "\x1b[H", end: "\x1b[F",
  pgup: "\x1b[5~", pgdn: "\x1b[6~", del: "\x1b[3~", ins: "\x1b[2~",
  ctrlc: "\x03", ctrld: "\x04", ctrlz: "\x1a", ctrll: "\x0c",
  ctrlr: "\x12", ctrlu: "\x15", ctrla: "\x01", ctrle: "\x05",
};

const SCREEN_KEYS = {
  esc: "Escape", tab: "Tab", enter: "Enter", up: "ArrowUp", down: "ArrowDown",
  left: "ArrowLeft", right: "ArrowRight", home: "Home", end: "End",
  pgup: "PageUp", pgdn: "PageDown", del: "Delete",
};

function sendKeyToTarget(key) {
  if ($("btnKbd").classList.contains("active")) {
    if (key.startsWith("ctrl") && key.length === 5) {
      sendControl({ t: "key", key: key.slice(4), mods: ["ctrl"] });
    } else if (SCREEN_KEYS[key]) {
      sendControl({ t: "key", key: SCREEN_KEYS[key] });
    }
    return;
  }
  const seq = TERM_KEYS[key];
  if (seq) termSend({ type: "input", data: seq });
}

document.querySelectorAll("#keybar [data-k]").forEach((button) => {
  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    const key = button.dataset.k;
    if (key === "ctrl") {
      state.ctrlArmed = !state.ctrlArmed;
      button.classList.toggle("active", state.ctrlArmed);
      if (state.ctrlArmed) state.term?.focus();
      return;
    }
    sendKeyToTarget(key);
  });
});

/* ================= custom keyboard popup ================= */

const CK_BASE = {
  esc: "\x1b", tab: "\t", enter: "\r", space: " ", backspace: "\x7f",
  up: "\x1b[A", down: "\x1b[B", right: "\x1b[C", left: "\x1b[D",
  home: "\x1b[H", end: "\x1b[F", pgup: "\x1b[5~", pgdn: "\x1b[6~",
  ins: "\x1b[2~", del: "\x1b[3~",
  f1: "\x1bOP", f2: "\x1bOQ", f3: "\x1bOR", f4: "\x1bOS",
  f5: "\x1b[15~", f6: "\x1b[17~", f7: "\x1b[18~", f8: "\x1b[19~",
  f9: "\x1b[20~", f10: "\x1b[21~", f11: "\x1b[23~", f12: "\x1b[24~",
  ctrlc: "\x03", ctrld: "\x04", ctrlz: "\x1a", ctrll: "\x0c",
  ctrlr: "\x12", ctrlu: "\x15", ctrla: "\x01", ctrle: "\x05",
  ctrlw: "\x17", ctrlk: "\x0b",
};
const CK_CSI = { up: "A", down: "B", right: "C", left: "D", home: "H", end: "F" };
const CK_TILDE = { ins: 2, del: 3, pgup: 5, pgdn: 6, f5: 15, f6: 17, f7: 18, f8: 19, f9: 20, f10: 21, f11: 23, f12: 24 };
const CK_F = { f1: "P", f2: "Q", f3: "R", f4: "S" };
const CK_SCREEN = {
  esc: "Escape", tab: "Tab", enter: "Enter", space: "Space", backspace: "Backspace",
  up: "ArrowUp", down: "ArrowDown", left: "ArrowLeft", right: "ArrowRight",
  home: "Home", end: "End", pgup: "PageUp", pgdn: "PageDown", del: "Delete", ins: "Insert",
  f1: "F1", f2: "F2", f3: "F3", f4: "F4", f5: "F5", f6: "F6",
  f7: "F7", f8: "F8", f9: "F9", f10: "F10", f11: "F11", f12: "F12",
};

function ckMods() {
  const m = { ...state.mods };
  if (m.altgr) m.alt = true;
  return m;
}

function ckScreenMode() {
  return true;
}

function ckModsList() {
  const m = ckMods();
  const out = [];
  if (m.cmd) out.push("cmd");
  if (m.ctrl) out.push("ctrl");
  if (m.alt) out.push("alt");
  if (m.shift) out.push("shift");
  return out;
}

function ckClearMods() {
  for (const key of Object.keys(state.mods)) state.mods[key] = false;
  document.querySelectorAll("#ckbd [data-m]").forEach((b) => b.classList.remove("active"));
  state.ctrlArmed = false;
  $("btnCtrl")?.classList.remove("active");
}

function sendTextToTerminal(text) {
  if (!text) return;
  let data = text;
  if (state.mods.ctrl && text.length === 1) data = String.fromCharCode(text.toUpperCase().charCodeAt(0) & 0x1f);
  else if (state.mods.alt || state.mods.altgr) data = "\x1b" + data;
  termSend({ type: "input", data });
  ckClearMods();
}

function ckSendText(text) {
  if (!text) return;
  if (ckScreenMode()) {
    sendControl({ t: "text", text });
    return;
  }
  sendTextToTerminal(text);
}

function ckSendKey(key) {
  if (ckScreenMode()) {
    if (key.startsWith("ctrl") && key.length === 5) {
      sendControl({ t: "key", key: key.slice(4), mods: ["ctrl"] });
    } else if (CK_SCREEN[key]) {
      sendControl({ t: "key", key: CK_SCREEN[key], mods: ckModsList() });
    }
    ckClearMods();
    return;
  }
  const base = CK_BASE[key];
  if (base === undefined) return;
  const mods = ckMods();
  const any = mods.shift || mods.alt || mods.ctrl;
  const m = 1 + (mods.shift ? 1 : 0) + (mods.alt ? 2 : 0) + (mods.ctrl ? 4 : 0);
  let data = base;
  if (key === "tab" && mods.shift && !mods.ctrl && !mods.alt) data = "\x1b[Z";
  else if (key === "enter" && mods.alt && !mods.ctrl) data = "\x1b\r";
  else if (any && CK_CSI[key]) data = `\x1b[1;${m}${CK_CSI[key]}`;
  else if (any && CK_TILDE[key]) data = `\x1b[${CK_TILDE[key]};${m}~`;
  else if (any && CK_F[key]) data = `\x1b[1;${m}${CK_F[key]}`;
  else if (any && key === "backspace" && mods.ctrl) data = "\x08";
  termSend({ type: "input", data });
  ckClearMods();
}

function updateCkbdTarget() {
  $("ckbdTarget").textContent = "screen";
}

function ckFit() {
  const vv = window.visualViewport;
  if (!vv) return;
  const bottom = Math.max(0, window.innerHeight - (vv.offsetTop + vv.height));
  $("ckbd").style.bottom = `${bottom}px`;
}

function ckOpen() {
  $("ckbd").classList.remove("hidden");
  updateCkbdTarget();
  ckFit();
}
function ckClose() {
  $("ckbd").classList.add("hidden");
  $("ckbdInput").blur();
  $("ckbdNative").classList.remove("active");
}

document.querySelectorAll("#ckbd [data-k]").forEach((button) => {
  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    ckSendKey(button.dataset.k);
  });
});

document.querySelectorAll("#ckbd [data-m]").forEach((button) => {
  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    const mod = button.dataset.m;
    state.mods[mod] = !state.mods[mod];
    if (mod === "altgr") state.mods.alt = state.mods.altgr ? true : state.mods.alt;
    button.classList.toggle("active", state.mods[mod]);
    if (mod === "ctrl") {
      state.ctrlArmed = state.mods.ctrl;
      $("btnCtrl")?.classList.toggle("active", state.ctrlArmed);
    }
  });
});

$("btnCkbd").addEventListener("click", () => {
  if ($("ckbd").classList.contains("hidden")) ckOpen();
  else ckClose();
});
$("ckbdClose").addEventListener("click", ckClose);

$("ckbdNative").addEventListener("click", () => {
  if (document.activeElement === $("ckbdInput")) {
    $("ckbdInput").blur();
    $("ckbdNative").classList.remove("active");
  } else {
    $("ckbdInput").focus();
    $("ckbdNative").classList.add("active");
  }
});

$("ckbdInput").addEventListener("input", () => {
  const value = $("ckbdInput").value;
  if (value) {
    ckSendText(value);
    $("ckbdInput").value = "";
  }
});

$("ckbdInput").addEventListener("keydown", (event) => {
  if (event.key === "Enter") { event.preventDefault(); ckSendKey("enter"); }
  else if (event.key === "Escape") { event.preventDefault(); ckSendKey("esc"); }
  else if (event.key === "Tab") { event.preventDefault(); ckSendKey("tab"); }
});

$("ckbdSend").addEventListener("pointerdown", (event) => {
  event.preventDefault();
  ckSendText($("ckbdInput").value);
  $("ckbdInput").value = "";
});

window.visualViewport?.addEventListener("resize", ckFit);
window.visualViewport?.addEventListener("scroll", ckFit);

/* ================= system stats ================= */

function fmtBytes(n) {
  if (!n) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}
function fmtRate(n) { return `${fmtBytes(n)}/s`; }

async function refreshStats() {
  if (state.statsBusy) return;
  state.statsBusy = true;
  try {
    const res = await api("/api/system");
    if (!res.ok) return;
    const s = await res.json();
    updateHomeStats(s);
    const cpu = Math.round(s.cpu || 0);
    $("gCpu").textContent = `${cpu}%`;
    $("bCpu").style.width = `${Math.min(100, cpu)}%`;
    $("bCpu").classList.toggle("hot", cpu > 80);
    const cores = $("gCores");
    cores.textContent = "";
    for (const value of s.cores || []) {
      const bar = document.createElement("i");
      bar.style.height = `${Math.max(8, Math.min(100, value))}%`;
      bar.title = `${Math.round(value)}%`;
      cores.append(bar);
    }
    const ramPct = s.mem_total ? (s.mem_used / s.mem_total) * 100 : 0;
    $("gRam").textContent = `${Math.round(ramPct)}% · ${fmtBytes(s.mem_used)}`;
    $("bRam").style.width = `${Math.min(100, ramPct)}%`;
    $("bRam").classList.toggle("hot", ramPct > 85);
    $("gSwap").textContent = s.swap_total ? `swap ${fmtBytes(s.swap_used)} / ${fmtBytes(s.swap_total)}` : "swap —";
    const diskPct = s.disk_total ? (s.disk_used / s.disk_total) * 100 : 0;
    $("gDisk").textContent = `${Math.round(diskPct)}%`;
    $("bDisk").style.width = `${Math.min(100, diskPct)}%`;
    $("bDisk").classList.toggle("hot", diskPct > 90);
    $("gDiskAbs").textContent = `${fmtBytes(s.disk_used)} / ${fmtBytes(s.disk_total)}`;
    $("gNet").textContent = fmtRate((s.net_rx_bps || 0) + (s.net_tx_bps || 0));
    $("gNetAbs").textContent = `↓ ${fmtRate(s.net_rx_bps || 0)} · ↑ ${fmtRate(s.net_tx_bps || 0)}`;
    const procs = $("statsProcs");
    procs.textContent = "";
    for (const p of s.procs || []) {
      const row = document.createElement("div");
      row.className = "proc-row";
      const name = document.createElement("span");
      name.className = "pn";
      name.textContent = `${p.name} · ${p.pid}`;
      const pc = document.createElement("span");
      pc.className = "pc";
      pc.textContent = `${(p.cpu || 0).toFixed(1)}%`;
      const pm = document.createElement("span");
      pm.className = "pm";
      pm.textContent = fmtBytes(p.mem || 0);
      row.append(name, pc, pm);
      procs.append(row);
    }
    $("statsHost").textContent = s.host || "—";
    const up = s.uptime || 0;
    $("statsFoot").textContent = `load ${s.load1?.toFixed(2)} ${s.load5?.toFixed(2)} ${s.load15?.toFixed(2)} · uptime ${Math.floor(up / 3600)}h ${Math.floor((up % 3600) / 60)}m`;
  } catch {
  } finally {
    state.statsBusy = false;
  }
}

function startStats() {
  refreshStats();
  clearInterval(state.statsTimer);
  state.statsTimer = setInterval(refreshStats, 2000);
}
function stopStats() {
  clearInterval(state.statsTimer);
  state.statsTimer = null;
}
$("statsRefresh").addEventListener("click", refreshStats);

/* ================= settings ================= */

function savePrefs() {
  try { localStorage.setItem("ra.prefs", JSON.stringify(state.prefs)); } catch {}
}

function applyPrefs() {
  state.minFrameMs = Math.max(120, Math.round(1000 / state.prefs.fps));
try {
  const savedMode = localStorage.getItem("ra.inputMode");
  if (savedMode === "scroll" && state.inputMode === "pad") $("btnInputMode")?.click();
} catch {}
  if (state.term) {
    state.term.options.fontSize = state.prefs.font;
    if (!$("termView").classList.contains("hidden")) fitTerm();
    else state.term.refresh(0, Math.max(0, (state.term.rows || 1) - 1));
  }
  $("keybar").classList.toggle("hidden", !state.prefs.keybar);
  $("prefKeybar").checked = state.prefs.keybar;
  const label = $("termSizeLabel");
  if (label) label.textContent = `tui · ${state.prefs.font.toFixed(1)}pt`;
  applyWake();
}

async function applyWake() {
  const want = state.prefs.awake && state.page === "remote" && !document.hidden;
  if (want && !state.wakeLock && navigator.wakeLock) {
    try {
      state.wakeLock = await navigator.wakeLock.request("screen");
      state.wakeLock.addEventListener("release", () => { state.wakeLock = null; });
    } catch {}
  } else if (!want && state.wakeLock) {
    try { await state.wakeLock.release(); } catch {}
    state.wakeLock = null;
  }
}

function renderSettings() {
  $("prefFps").value = String(state.prefs.fps);
  $("prefFont").value = String(state.prefs.font);
  $("prefFontVal").textContent = state.prefs.font.toFixed(1);
  if ($("tuiHint")) $("tuiHint").textContent = `${state.prefs.font.toFixed(1)}pt`;
  $("prefKeybar").checked = state.prefs.keybar;
  $("prefAwake").checked = state.prefs.awake;
  const capture = state.status?.capture;
  $("prefQuality").textContent = capture
    ? `${capture.resolution} · q${capture.quality} · ${capture.width}×${capture.height}`
    : "—";
  const s = state.status || {};
  $("settingsInfo").textContent =
    `session ${s.session || "omp"} · cwd ${s.cwd || "—"} · input ${s.input?.trusted ? "ok" : "no permission"}`;
  const publicUrl = s.network?.publicUrl;
  const ttlDays = s.network?.sessionTtlDays;
  $("settingsPublic").textContent = publicUrl || "belum ada tunnel";
  if ($("ttlInfo")) {
    $("ttlInfo").textContent = ttlDays
      ? `Cookie berlaku ${ttlDays} hari sejak login; diperbarui saat login ulang.`
      : "Cookie berlaku selama konfigurasi SESSION_TTL_DAYS.";
  }
  if ($("ttlPill")) $("ttlPill").textContent = ttlDays ? `${ttlDays}d` : "—";
  $("settingsTunnelState").textContent = publicUrl ? "online" : "LAN only";
  $("settingsTunnelState").className = `pill ${publicUrl ? "ok" : ""}`;
  loadConfigInfo();
}

$("prefFps").addEventListener("change", () => {
  state.prefs.fps = Number($("prefFps").value) || 5;
  savePrefs();
  applyPrefs();
  showToast(`target ${state.prefs.fps} fps`, 1800);
});
$("prefFont").addEventListener("input", () => {
  state.prefs.font = Number($("prefFont").value) || 12.5;
  $("prefFontVal").textContent = state.prefs.font.toFixed(1);
  savePrefs();
  applyPrefs();
});
$("prefKeybar").addEventListener("change", () => {
  state.prefs.keybar = $("prefKeybar").checked;
  savePrefs();
  applyPrefs();
});
// Back/forward can restore the login DOM from bfcache long after a successful
// pairing. Re-run the session check instead of showing a page the server would
// no longer agree with.
window.addEventListener("pageshow", (event) => {
  if (event.persisted) location.reload();
});

$("switchDeviceBtn")?.addEventListener("click", () => {
  // Keep this device active until another one is actually chosen; the picker
  // lives in the app so there is one UI to keep up to date.
  location.href = "/?picker=1";
});

$("prefAwake").addEventListener("change", () => {
  state.prefs.awake = $("prefAwake").checked;
  savePrefs();
  applyWake();
});

async function loadConfigInfo() {
  try {
    const res = await api("/api/config");
    if (!res.ok) return;
    const c = await res.json();
    const grid = $("configGrid");
    if (!grid) return;
    grid.innerHTML = "";
    const rows = [
      ["bind", `${c.host}:${c.port}`],
      ["omp cwd", c.ompCwd],
      ["omp bin", c.ompBin],
      ["allowed roots", (c.allowedRoots || []).join(", ")],
      ["capture", `${c.resolution} q${c.jpegQuality} @ ${c.captureFps}fps`],
      ["token", c.tokenSource],
      ["public url", c.publicUrl || "—"],
    ];
    for (const [key, value] of rows) {
      const line = document.createElement("div");
      const strong = document.createElement("b");
      strong.textContent = `${key}: `;
      line.append(strong, document.createTextNode(String(value)));
      grid.append(line);
    }
  } catch {}
}

/* ================= layout ================= */

function applyView() {
  const grid = $("remoteGrid");
  grid.classList.remove("view-screen", "view-term", "view-files", "view-split");
  if (state.view === "screen") grid.classList.add("view-screen");
  if (state.view === "term") grid.classList.add("view-term");
  if (state.view === "split") grid.classList.add("view-split");
  if (state.view === "files") {
    grid.classList.add("view-files");
    $("filesPane")?.classList.remove("hidden");
    loadFilesPage();
  } else {
    $("filesPane")?.classList.add("hidden");
  }
  document.querySelectorAll("#segLayout button").forEach((button) => {
    button.classList.toggle("active", button.dataset.layout === state.view);
  });
  localStorage.setItem("ra.view", state.view);

  // Focus modes: stop the terminal session entirely so the screen owns the
  // viewport. Coming back re-initialises it with a correct size.
  if (state.view === "screen") {
    closeTerminal();
  } else if (state.view === "term" || state.view === "split") {
    if (state.termWs && state.termWs.readyState <= WebSocket.OPEN) {
      requestAnimationFrame(() => {
        initTerminal();
        setTimeout(fitTerm, 60);
      });
    } else if (state.resumeSession) {
      requestAnimationFrame(() => startTerminalFromPicker(state.resumeSession));
    } else {
      // Nothing running: let the user pick a session instead of a blank pane.
      requestAnimationFrame(() => showTermEmpty());
    }
  }

  syncScreen();
  setTimeout(() => { fitTerm(); }, 120);
  setTimeout(() => { fitTerm(); }, 320);
}

document.querySelectorAll("#segLayout button").forEach((button) => {
  button.addEventListener("click", () => {
    state.view = button.dataset.layout;
    applyView();
  });
});

function toggleFullscreen(el) {
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  } else {
    el.requestFullscreen?.().then(() => {
      if (screen.orientation?.lock) screen.orientation.lock("landscape").catch(() => {});
    }).catch(() => {});
  }
}

$("btnInputMode")?.addEventListener("click", () => {
  state.inputMode = state.inputMode === "pad" ? "scroll" : "pad";
  const pad = state.inputMode === "pad";
  const btn = $("btnInputMode");
  btn.textContent = pad ? "pad" : "scroll";
  btn.classList.toggle("active", pad);
  btn.title = pad
    ? "Mode touchpad: geser = gerakkan kursor"
    : "Mode scroll: geser = gulir layar";
  localStorage.setItem("ra.inputMode", state.inputMode);
  showToast(pad ? "Mode touchpad" : "Mode scroll", 1200);
});

$("btnClick")?.addEventListener("click", () => {
  const p = cursorToScreen();
  if (!p) return;
  sendControl({ t: "down", ...p, button: activeButton() });
  setTimeout(() => sendControl({ t: "up", ...p, button: activeButton() }), 40);
});

$("btnScreenFull").addEventListener("click", () => toggleFullscreen($("screenPanel")));
$("btnTermFull")?.addEventListener("click", () => toggleFullscreen($("termPanel")));
$("fsExitScreen").addEventListener("click", () => { document.exitFullscreen?.().catch(() => {}); });
$("fsExitTerm").addEventListener("click", () => { document.exitFullscreen?.().catch(() => {}); });
$("fsKeys").addEventListener("click", () => {
  document.exitFullscreen?.().then(() => setTimeout(ckOpen, 150)).catch(() => ckOpen());
});
document.addEventListener("fullscreenchange", () => {
  setTimeout(() => { fitTerm(); }, 120);
});

let dividerActive = false;
function dividerWide() {
  return matchMedia("(orientation: landscape) and (min-width: 820px)").matches;
}
$("divider").addEventListener("pointerdown", (event) => {
  dividerActive = true;
  $("divider").setPointerCapture(event.pointerId);
});
$("divider").addEventListener("pointermove", (event) => {
  if (!dividerActive) return;
  const rect = $("remoteGrid").getBoundingClientRect();
  const pct = dividerWide()
    ? ((event.clientX - rect.left) / rect.width) * 100
    : ((event.clientY - rect.top) / rect.height) * 100;
  const clamped = Math.max(18, Math.min(84, pct));
  $("remoteGrid").style.setProperty("--split", `${clamped}%`);
});
$("divider").addEventListener("pointerup", (event) => {
  dividerActive = false;
  $("divider").releasePointerCapture(event.pointerId);
  fitTerm();
});

window.addEventListener("resize", fitTerm);
window.addEventListener("orientationchange", () => setTimeout(fitTerm, 250));
document.addEventListener("gesturestart", (event) => event.preventDefault());
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && document.fullscreenElement) document.exitFullscreen();
});
document.addEventListener("visibilitychange", () => {
  syncScreen();
  applyWake();
});

/* ================= project file explorer (Remote > Files) ================= */

const filesState = { path: "", file: null, preview: true, hidden: false };

async function loadFilesPage() {
  if (!$("treeList")) return;
  const project = state.projectId || "";
  if (!filesState.path) {
    try {
      const res = await api(`/api/files?project=${encodeURIComponent(project)}&path=`);
      if (res.ok) {
        const data = await res.json();
        filesState.path = data.root || "";
      }
    } catch {}
  }
  renderTree(filesState.path);
}

async function renderTree(path) {
  const list = $("treeList");
  if (!list) return;
  list.textContent = "";
  const project = state.projectId || "";
  try {
    const res = await api(`/api/files?project=${encodeURIComponent(project)}&path=${encodeURIComponent(path)}`);
    if (!res.ok) {
      list.innerHTML = `<div class="hint-line">${await res.text()}</div>`;
      return;
    }
    const data = await res.json();
    filesState.path = data.path || "";
    if ($("filesBreadcrumb")) $("filesBreadcrumb").textContent = data.path || "—";
    for (const entry of data.entries || []) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = `tree-row ${entry.is_dir ? "dir" : "file"}`;
      const icon = document.createElement("span");
      icon.className = "ti";
      icon.textContent = entry.is_dir ? "▸" : "·";
      const name = document.createElement("span");
      name.className = "tn";
      name.textContent = entry.name;
      row.append(icon, name);
      row.addEventListener("click", () => {
        if (entry.is_dir) renderTree(entry.abs || entry.path);
        else openFile(entry);
      });
      list.append(row);
    }
  } catch (err) {
    list.innerHTML = `<div class="hint-line">${err.message}</div>`;
  }
}

async function openFile(entry) {
  filesState.file = entry;
  filesState.preview = true;
  $("filesBody")?.classList.add("code-open");
  if ($("fileName")) $("fileName").textContent = entry.name;
  if ($("fileMeta")) $("fileMeta").textContent = fmtBytes(entry.size || 0);
  $("mdToggle")?.classList.toggle("hidden", !entry.markdown);
  const body = $("codeBody");
  if (!body) return;
  body.className = "code-body";

  const project = state.projectId || "";
  const res = await api(`/api/files/read?project=${encodeURIComponent(project)}&path=${encodeURIComponent(entry.abs || entry.path)}`);
  if (!res.ok) {
    body.textContent = (await res.text()) || "gagal membaca file";
    return;
  }
  const data = await res.json();
  filesState.file = { ...entry, ...data };
  renderOpenedFile();
}

function renderOpenedFile() {
  const file = filesState.file;
  const body = $("codeBody");
  if (!file || !body) return;
  $("mdToggle")?.classList.toggle("hidden", !file.markdown);
  if (file.markdown && filesState.preview) {
    if ($("mdToggle")) $("mdToggle").textContent = "source";
    const html = window.marked ? marked.parse(file.content || "") : (file.content || "");
    body.className = "code-body md-body";
    body.innerHTML = window.DOMPurify ? DOMPurify.sanitize(html) : html;
    return;
  }
  if ($("mdToggle")) $("mdToggle").textContent = "preview";
  body.className = "code-body";
  renderCode(file.content || "");
}

function renderCode(content) {
  const body = $("codeBody");
  if (!body) return;
  body.textContent = "";
  const lines = content.split("\n");
  const limit = Math.min(lines.length, 12000);
  const frag = document.createDocumentFragment();
  for (let i = 0; i < limit; i++) {
    const row = document.createElement("div");
    row.className = "code-line";
    const ln = document.createElement("span");
    ln.className = "ln";
    ln.textContent = String(i + 1);
    const lc = document.createElement("span");
    lc.className = "lc";
    lc.textContent = lines[i];
    row.append(ln, lc);
    frag.append(row);
  }
  body.append(frag);
}

$("mdToggle")?.addEventListener("click", () => {
  filesState.preview = !filesState.preview;
  renderOpenedFile();
});
$("fileBack")?.addEventListener("click", () => $("filesBody")?.classList.remove("code-open"));
$("filesRoot")?.addEventListener("click", () => renderTree(""));
$("fileOpenRaw")?.addEventListener("click", () => {
  const f = filesState.file;
  if (!f) return;
  const project = state.projectId || "";
  window.open(`/api/files/raw?project=${encodeURIComponent(project)}&path=${encodeURIComponent(f.abs || f.path)}`, "_blank");
});

/* ================= terminal zoom ================= */

function setTermFont(size) {
  // 5pt is the practical floor for xterm; 8pt was an arbitrary older limit.
  state.prefs.font = Math.max(5, Math.min(20, Math.round(size * 2) / 2));
  savePrefs();
  if (state.term) {
    state.term.options.fontSize = state.prefs.font;
    setTimeout(fitTerm, 30);
  } else {
    applyPrefs();
  }
  $("prefFont").value = String(state.prefs.font);
  $("prefFontVal").textContent = state.prefs.font.toFixed(1);
  if ($("tuiHint")) $("tuiHint").textContent = `${state.prefs.font.toFixed(1)}pt`;
}

function termZoom(delta) {
  setTermFont(state.prefs.font + delta);
  showToast(`terminal ${state.prefs.font.toFixed(1)}pt`, 1200);
}

$("termMinus")?.addEventListener("click", () => termZoom(-0.5));
$("termPlus")?.addEventListener("click", () => termZoom(0.5));

document.addEventListener("keydown", (event) => {
  if (!(event.ctrlKey || event.metaKey)) return;
  if (event.key === "-" || event.key === "_") {
    event.preventDefault();
    termZoom(-0.5);
  } else if (event.key === "=" || event.key === "+") {
    event.preventDefault();
    termZoom(0.5);
  } else if (event.key === "0") {
    event.preventDefault();
    setTermFont(window.innerWidth < 600 ? 11 : 12.5);
  }
});

/* ================= sidebar collapse ================= */

function applySidebar(collapsed) {
  $("app").classList.toggle("collapsed", collapsed);
  const button = $("sideToggle");
  if (button) {
    button.textContent = collapsed ? "»" : "«";
    button.title = collapsed ? "Expand sidebar" : "Collapse sidebar";
  }
}

function initSidebar() {
  const stored = localStorage.getItem("ra.sidebar");
  const collapsed = stored ? stored === "collapsed" : true;
  applySidebar(collapsed);
  $("sideToggle")?.addEventListener("click", () => {
    const next = !$("app").classList.contains("collapsed");
    applySidebar(next);
    localStorage.setItem("ra.sidebar", next ? "collapsed" : "expanded");
  });
}

/* ================= PWA ================= */

let installPrompt = null;

window.addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  installPrompt = event;
  $("btnInstall")?.classList.remove("hidden");
});

window.addEventListener("appinstalled", () => {
  installPrompt = null;
  $("btnInstall")?.classList.add("hidden");
  showToast("Remote Agents dipasang sebagai app", 2600);
});

$("btnInstall")?.addEventListener("click", async () => {
  if (!installPrompt) {
    showToast("Gunakan menu browser → Add to Home Screen / Install app", 5000);
    return;
  }
  installPrompt.prompt();
  const result = await installPrompt.userChoice.catch(() => null);
  if (result?.outcome === "accepted") {
    installPrompt = null;
    $("btnInstall")?.classList.add("hidden");
  }
});

if ("serviceWorker" in navigator && location.protocol === "https:") {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").then((reg) => {
      // A PWA has no browser reload button, so surface updates explicitly and
      // reload once the new worker takes control.
      reg.addEventListener("updatefound", () => {
        const worker = reg.installing;
        worker?.addEventListener("statechange", () => {
          if (worker.state === "installed" && navigator.serviceWorker.controller) {
            showUpdateBar(reg);
          }
        });
      });
      // A PWA can stay "running" for days, so poll for a new build instead of
      // trusting focus events alone. Reloading happens on controllerchange.
      const checkUpdate = () => reg.update().catch(() => {});
      document.addEventListener("visibilitychange", () => {
        if (!document.hidden) checkUpdate();
      });
      window.addEventListener("online", checkUpdate);
      setInterval(checkUpdate, 60_000);
    }).catch(() => {});

    let reloading = false;
    navigator.serviceWorker.addEventListener("controllerchange", () => {
      if (reloading) return;
      // Never reload under a live terminal: it would drop the tmux attach and
      // look like the session "ended by itself". Offer the update instead.
      const busy = state.termWs && state.termWs.readyState <= WebSocket.OPEN;
      if (busy) {
        showUpdateBar();
        return;
      }
      reloading = true;
      location.reload();
    });
  });
}

/* Explicit reload controls — a PWA has no browser refresh button. */
async function reloadApp({ clearCache = false } = {}) {
  try {
    if (clearCache) {
      if ("caches" in window) {
        const keys = await caches.keys();
        await Promise.all(keys.map((k) => caches.delete(k)));
      }
      const reg = await navigator.serviceWorker?.getRegistration();
      const keys = await reg?.cacheKeys?.().catch(() => []);
      if (keys?.length) await caches.delete(keys[0]);
    }
    const reg = await navigator.serviceWorker?.getRegistration();
    await reg?.update?.();
  } catch {}
  // Give the new worker a moment to claim, then reload from the network.
  setTimeout(() => location.reload(), clearCache ? 350 : 120);
}

$("btnFiles")?.addEventListener("click", () => go("files"));
$("btnReloadApp")?.addEventListener("click", () => reloadApp());
$("reloadAppBtn")?.addEventListener("click", () => reloadApp());
$("clearCacheBtn")?.addEventListener("click", () => reloadApp({ clearCache: true }));

function showUpdateBar(reg) {
  if (document.getElementById("updateBar")) return;
  const bar = document.createElement("div");
  bar.id = "updateBar";
  bar.className = "update-bar";
  bar.innerHTML = '<span>Versi baru tersedia</span>';
  const go = document.createElement("button");
  go.textContent = "Muat ulang";
  go.addEventListener("click", () => {
    reg?.waiting?.postMessage({ type: "SKIP_WAITING" });
    setTimeout(() => location.reload(), 250);
  });
  const later = document.createElement("button");
  later.textContent = "nanti";
  later.className = "ghost";
  later.addEventListener("click", () => bar.remove());
  bar.append(go, later);
  document.body.append(bar);
}

bindNavigation();
window.__RA_BOOT_REACHED = true;
boot().catch((err) => {
  // Never leave the shell hidden without a trace.
  console.error("boot failed", err);
  showLogin();
});

/* ================= recent devices (login) ================= */

const RD_KEY = "ra.devices";

function loadDevices() {
  try {
    const list = JSON.parse(localStorage.getItem(RD_KEY) || "[]");
    return Array.isArray(list) ? list.filter((d) => d && d.url) : [];
  } catch {
    return [];
  }
}

function saveDevices(list) {
  try { localStorage.setItem(RD_KEY, JSON.stringify(list.slice(0, 12))); } catch {}
}

function deviceKeyOf(device) {
  return `${device.url || ""}|${device.id || device.name || ""}`;
}

function rememberDevice(device) {
  // One hostname now serves every device, so the ID (not the URL) identifies
  // an entry in the recent list.
  const key = deviceKeyOf(device);
  const list = loadDevices().filter((d) => deviceKeyOf(d) !== key);
  list.unshift({ ...device, lastUsed: Date.now() });
  saveDevices(list);
}

async function probeDevice(device) {
  // Presence probe: /api/identity is public and CORS-enabled.
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 2500);
  try {
    const res = await fetch(`${device.url.replace(/\/$/, "")}/api/identity`, {
      signal: controller.signal,
      cache: "no-store",
    });
    if (!res.ok) return null;
    const data = await res.json();
    return data.id === device.id ? data : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/// One device list. Prefer the relay's map (it has live status); fall back to
/// entries saved in this browser so LAN-only setups still work.
let deviceRenderToken = 0;

/* One entry point for "open this device": pin the relay cookie, remember it,
   and skip the /auth hop when this browser already holds a session for it. */
function openSavedDevice(device) {
  if (device.key) {
    try {
      document.cookie = `ra_device=${device.key}; path=/; max-age=31536000; SameSite=Lax; Secure`;
      document.cookie = `ra_device=${device.key}; path=/; max-age=31536000; SameSite=Lax`;
    } catch {}
  }
  if (device.id) {
    rememberDevice({ id: device.id, name: device.name, url: location.origin, key: device.key });
  }
  restoreTrustedCookies();
  // Always resolve to the root page. When we are already on it, a plain href
  // is a no-op and leaves a stale login screen in place; reload instead so the
  // session check runs again with the cookie we just set.
  if (location.pathname === "/" && !location.search) {
    location.reload();
  } else {
    location.href = "/";
  }
}

async function renderRecentDevices() {
  const wrap = $("recentDevices");
  if (!wrap) return;
  // boot() and showLogin() can both ask for this list; without a token the
  // slower fetch appends a second copy of every device after the first render.
  const token = ++deviceRenderToken;
  wrap.textContent = "";

  let devices = [];
  let fromRelay = false;
  try {
    const res = await fetch("/api/devices", { cache: "no-store" });
    if (res.ok) {
      const data = await res.json();
      devices = (data.devices || []).filter((d) => d && d.key);
      fromRelay = devices.length > 0;
    }
  } catch {}

  if (!devices.length) {
    devices = loadDevices().map((d) => ({
      key: d.key || "",
      name: d.name || d.id,
      id: d.id,
      saved: true,
    }));
  } else {
    // Relay is the source of truth: prune saved entries that no longer exist
    // there, so a deleted device cannot linger in this browser.
    const live = new Set(devices.map((d) => d.id).filter(Boolean));
    const pruned = loadDevices().filter((d) => !d.id || live.has(d.id));
    if (pruned.length !== loadDevices().length) saveDevices(pruned);
  }

  if (token !== deviceRenderToken) return;

  if (!devices.length) {
    const empty = document.createElement("div");
    empty.className = "dl-empty";
    empty.textContent = "Belum ada perangkat. Scan QR dari app desktop.";
    wrap.append(empty);
    return;
  }

  const head = document.createElement("div");
  head.className = "dl-head";
  head.textContent = fromRelay ? "Perangkat" : "Tersimpan di perangkat ini";
  wrap.append(head);

  for (const device of devices) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "dl-item";

    const dot = document.createElement("span");
    dot.className = "dl-dot";

    const main = document.createElement("span");
    main.className = "dl-main";
    const name = document.createElement("b");
    name.textContent = device.name || device.id || device.key;
    const sub = document.createElement("span");
    const trusted = device.id && trustedKeys()[String(device.id)];
    sub.textContent = `${device.id || device.key || ""}${trusted ? " · tersambung" : " · perlu pairing"}`;
    main.append(name, sub);

    const chev = document.createElement("span");
    chev.className = "dl-chev";
    chev.textContent = "›";

    row.append(dot, main, chev);
    row.addEventListener("click", async () => {
      row.classList.add("busy");
      if (device.key) {
        try { await fetch(`/switch?key=${encodeURIComponent(device.key)}`, { cache: "no-store" }); } catch {}
      }
      openSavedDevice(device);
    });
    // Swipe actions belong to the project list only; pairing is managed from
    // Settings so a stray swipe cannot silently drop a device.
    wrap.append(row);

    if (fromRelay && device.key) {
      fetch(`/probe/${encodeURIComponent(device.key)}`, { cache: "no-store" })
        .then((r) => dot.classList.add(r.ok ? "online" : "offline"))
        .catch(() => dot.classList.add("offline"));
    } else {
      dot.classList.add("online");
    }
  }
}

renderRecentDevices();

// Decode QR from a camera photo/upload, then exchange the token.
// Uses BarcodeDetector when available; falls back to a bundled minimal decoder.
async function decodeQrFromImage(file) {
  if (!file) return null;
  if ("BarcodeDetector" in window) {
    try {
      const bitmap = await createImageBitmap(file);
      const detector = new BarcodeDetector({ formats: ["qr_code"] });
      const codes = await detector.detect(bitmap);
      bitmap.close?.();
      if (codes.length) return codes[0].rawValue;
    } catch {}
  }
  // jsQR (vendored) fallback.
  if (window.jsQR) {
    const bitmap = await createImageBitmap(file);
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const ctx = canvas.getContext("2d");
    ctx.drawImage(bitmap, 0, 0);
    bitmap.close?.();
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const result = window.jsQR(data.data, data.width, data.height);
    return result?.data || null;
  }
  return null;
}

/// The QR carries a pairing link like
///   https://relay.example.com/pair?k=a#c=ABC123&d=000000000
/// The one-time code lives in the *fragment*, which URLSearchParams on
/// `location.search` never sees. Parse all three shapes.
function pairInfoFromQr(payload) {
  const text = String(payload || "").trim();
  try {
    const url = new URL(text);
    const hash = new URLSearchParams(url.hash.replace(/^#/, ""));
    const code = (hash.get("c") || url.searchParams.get("c") || url.searchParams.get("code") || "")
      .toUpperCase()
      .replace(/[^A-Z0-9]/g, "");
    const legacyToken = url.searchParams.get("id") || url.searchParams.get("token");
    const key = url.searchParams.get("k") || "";
    return { code, token: legacyToken, key, href: text };
  } catch {
    // Plain code typed/scanned without a URL wrapper.
    const plain = text.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
    if (plain.length >= 4 && plain.length <= 8) return { code: plain, token: null, key: "", href: "" };
    const match = text.match(/[A-Za-z0-9_-]{16,}/);
    return { code: "", token: match ? match[0] : null, key: "", href: "" };
  }
}

function tokenFromQrPayload(payload) {
  const info = pairInfoFromQr(payload);
  return info.token || (info.code ? `code:${info.code}` : null);
}

/// Exchange a one-time pairing code for a session on this device.
/// The relay needs `?k=<relay key>` to pick the backend before this browser has
/// a device cookie; without it nginx rejects the request and the UI wrongly
/// reported "code expired".
async function rememberRelayDevice(relayKey) {
  let key = (relayKey || "").trim();
  // The relay inlines the key for single-device deployments, so a cookie-less
  // client can recover without an authenticated /api/setup call.
  if (!key) key = String(window.__RA_DEVICE || "").trim();
  if (!key) {
    try {
      const res = await fetch("/api/setup", { cache: "no-store" });
      if (res.ok) {
        const setup = await res.json();
        key = String(setup?.settings?.relayKey || "").trim();
      }
    } catch {}
  }
  if (!key) return false;
  document.cookie = `ra_device=${key}; path=/; max-age=31536000; SameSite=Lax; Secure`;
  return true;
}

async function claimPairingCode(code, relayKey = "") {
  const q = relayKey ? `?k=${encodeURIComponent(relayKey)}` : "";
  const res = await api(`/api/pair/claim${q}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ code, name: deviceLabel(), platform: "web" }),
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    console.warn("pair/claim failed", res.status, body.slice(0, 120));
    // Surface the real reason instead of always blaming expiry.
    if ($("loginErr")) {
      $("loginErr").textContent =
        res.status === 429 ? "Terlalu banyak percobaan. Tunggu sebentar."
        : res.status === 401 ? "Kode salah atau sudah dipakai."
        : res.status === 418 ? "Relay tidak mengenali perangkat ini."
        : `Gagal menghubungi relay (${res.status}).`;
    }
    return false;
  }
  const paired = await res.json().catch(() => ({}));
  trustDevice(paired.deviceId, paired.clientKey);
  await rememberRelayDevice(relayKey);
  restoreTrustedCookies();
  await start();
  return true;
}

async function loginWithToken(token) {
  if (!token) return false;
  $("loginErr").textContent = "";
  const res = await api("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token }),
  });
  if (!res.ok) {
    $("loginErr").textContent = res.status === 429 ? "Terlalu banyak percobaan." : "QR/token tidak valid.";
    return false;
  }
  stopScanner();
  const device = {
    id: token,
    name: $("deviceHint")?.textContent?.startsWith("Menghubungkan ke ")
      ? $("deviceHint").textContent.replace(/^Menghubungkan ke |…$/g, "")
      : (state.status?.deviceName || "Perangkat"),
    url: location.origin,
  };
  rememberDevice(device);
  await start();
  return true;
}

let scanStream = null;
let scanTimer = null;
let scanBusy = false;

function stopScanner() {
  clearInterval(scanTimer);
  scanTimer = null;
  scanBusy = false;
  if (scanStream) {
    for (const track of scanStream.getTracks()) track.stop();
    scanStream = null;
  }
  const video = $("scanVideo");
  if (video) video.srcObject = null;
  $("scanModal")?.classList.add("hidden");
}

async function openScanner() {
  const modal = $("scanModal");
  const video = $("scanVideo");
  const hint = $("scanHint");
  if (!modal || !video) return;
  modal.classList.remove("hidden");
  hint.textContent = "Meminta izin kamera…";

  if (!window.isSecureContext) {
    hint.textContent = "Kamera hanya bisa di HTTPS (tunnel) atau localhost.";
    return;
  }
  if (!navigator.mediaDevices?.getUserMedia) {
    hint.textContent = "Browser ini tidak mendukung kamera langsung. Gunakan “Pilih foto QR”.";
    return;
  }

  try {
    scanStream = await navigator.mediaDevices.getUserMedia({
      video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 }, height: { ideal: 1280 } },
      audio: false,
    });
    video.srcObject = scanStream;
    await video.play().catch(() => {});
    hint.textContent = "Arahkan kamera ke QR di app desktop.";
    scanTimer = setInterval(scanTick, 300);
  } catch (err) {
    hint.textContent = "Kamera tidak bisa dibuka. Gunakan “Pilih foto QR” atau izinkan kamera.";
  }
}

async function scanTick() {
  if (scanBusy) return;
  const video = $("scanVideo");
  if (!video || video.readyState < 2 || !video.videoWidth) return;
  scanBusy = true;
  try {
    let payload = null;
    if ("BarcodeDetector" in window) {
      try {
        const detector = new BarcodeDetector({ formats: ["qr_code"] });
        const codes = await detector.detect(video);
        if (codes.length) payload = codes[0].rawValue;
      } catch {}
    }
    if (!payload && window.jsQR) {
      const canvas = document.createElement("canvas");
      canvas.width = video.videoWidth;
      canvas.height = video.videoHeight;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const result = window.jsQR(data.data, data.width, data.height);
      if (result?.data) payload = result.data;
    }
    if (payload) {
      const info = pairInfoFromQr(payload);
      if (info.code) {
        stopScanner();
        $("token").value = info.code.slice(0, 3) + (info.code.length > 3 ? "-" + info.code.slice(3) : "");
        if (!(await claimPairingCode(info.code, info.key))) {
          $("loginErr").textContent = "Kode QR sudah kedaluwarsa. Buat kode baru di app desktop.";
        }
        return;
      }
      if (info.token) {
        stopScanner();
        $("token").value = info.token;
        if (!(await loginWithToken(info.token))) {
          $("loginErr").textContent = "QR ditolak.";
        }
        return;
      }
      // Recognisable QR, but nothing usable in it.
      if ($("scanHint")) $("scanHint").textContent = "QR tidak dikenali. Pastikan itu QR dari app desktop.";
    }
  } finally {
    scanBusy = false;
  }
}

$("scanBtn")?.addEventListener("click", openScanner);
$("scanClose")?.addEventListener("click", stopScanner);
$("scanModal")?.addEventListener("click", (event) => {
  if (event.target === $("scanModal")) stopScanner();
});

// File-picker fallback only when the user asks for it.
$("scanFallback")?.addEventListener("click", () => {
  stopScanner();
  $("qrFile").click();
});

$("qrFile")?.addEventListener("change", async () => {
  const file = $("qrFile").files?.[0];
  $("qrFile").value = "";
  if (!file) return;
  $("loginErr").textContent = "Membaca QR…";
  const payload = await decodeQrFromImage(file);
  const token = tokenFromQrPayload(payload);
  if (!token) {
    $("loginErr").textContent = "QR tidak terbaca. Coba foto ulang atau ketik token.";
    return;
  }
  if (!(await loginWithToken(token))) {
    $("loginErr").textContent = "QR terbaca, tetapi token ditolak.";
  }
});
