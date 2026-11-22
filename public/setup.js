/* Desktop setup shell: pairing QR + configuration, all through the local API. */
const $ = (id) => document.getElementById(id);

/* Tabs keep the window short: only the active pane scrolls. */
function showTab(name) {
  document.querySelectorAll("[data-tab]").forEach((t) =>
    t.classList.toggle("active", t.dataset.tab === name)
  );
  document.querySelectorAll(".pane").forEach((p) =>
    p.classList.toggle("active", p.id === `pane-${name}`)
  );
  try { localStorage.setItem("ra.setupTab", name); } catch {}
}


document.querySelectorAll("[data-tab]").forEach((tab) =>
  tab.addEventListener("click", () => showTab(tab.dataset.tab))
);
// Always start on the main screen: a setup window that opens deep inside
// Settings looks broken to a first-time user.
showTab("connect");
const LOCAL_HEADERS = { "x-requested-with": "remote-agents" };

async function api(path, options = {}) {
  // Served loopback-only. The header is sent when the webview allows it; the
  // server also accepts a plain loopback request from the setup page.
  return fetch(path, {
    credentials: "same-origin",
    ...options,
    headers: { ...LOCAL_HEADERS, ...(options.headers || {}) },
  });
}

function setMsg(text, ok = false) {
  const el = $("cfgMsg");
  el.textContent = text;
  el.classList.toggle("ok", ok);
}

function row(label, value, extra) {
  const el = document.createElement("div");
  el.className = "row";
  const grow = document.createElement("div");
  grow.className = "grow";
  const b = document.createElement("b");
  b.textContent = label;
  const small = document.createElement("small");
  small.textContent = value ?? "—";
  grow.append(b, small);
  el.append(grow);
  if (extra) el.append(extra);
  return el;
}

function pill(text, kind = "") {
  const el = document.createElement("span");
  el.className = `pill ${kind}`.trim();
  el.textContent = text;
  return el;
}

let pairTimer = null;
let state = null;

async function load({ quiet = false } = {}) {
  let data;
  try {
    const res = await api("/api/setup");
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    data = await res.json();
  } catch (err) {
    if (!quiet) $("headSub").textContent = `Cannot load status: ${err.message}`;
    $("livePill").textContent = "offline";
    $("livePill").className = "pill warn";
    return;
  }
  state = data;
  render(data);
}

function render(data) {
  const { device, env, permissions, pairing, security, runtime } = data;
  $("headSub").textContent = `ID ${device.id} · ${device.hostname || device.platform}`;
  if ($("deviceId")) $("deviceId").textContent = device.id;
  if ($("connName")) $("connName").textContent = device.name || "—";
  if ($("connRelay")) $("connRelay").textContent = env.publicUrl || env.lanUrl || "—";
  if ($("connPill")) {
    const ready = permissions.screen.ok || permissions.input.ok;
    $("connPill").textContent = ready ? "ready" : "needs permission";
    $("connPill").className = `pill ${ready ? "ok" : "warn"}`;
  }
  const screenOk = permissions.screen.ok;
  const inputOk = permissions.input.ok;
  const sessionLive = typeof pairing.sessionPresence === "number" && pairing.sessionPresence < 45;
  const connected = (pairing.clients || []).filter((c) => c.online).length + (sessionLive ? 1 : 0);
  if (connected) {
    $("livePill").textContent = `${connected} mobile`;
    $("livePill").className = "pill ok";
  } else {
    $("livePill").textContent = screenOk ? "online" : "needs permission";
    $("livePill").className = `pill ${screenOk ? "ok" : "warn"}`;
  }

  // ---- pair ----
  const code = pairing.code || "";
  const expires = pairing.expiresIn || 0;
  if (code) {
    const pretty = `${code.slice(0, 3)}-${code.slice(3)}`;
    $("pairCode").textContent = pretty;
    $("pairHint").textContent =
      expires > 0 ? `Valid for ${Math.floor(expires / 60)}m ${expires % 60}s · single use` : "Expired";
    const img = $("qrBox").querySelector("img");
    const src = `/desktop/setup-qr.svg?c=${encodeURIComponent(code)}`;
    if (!img || img.getAttribute("src") !== src) {
      $("qrBox").classList.remove("is-empty");
      $("qrBox").innerHTML = "";
      const next = document.createElement("img");
      next.alt = "QR pairing";
      next.src = src;
      $("qrBox").append(next);
    }
  } else {
    $("pairCode").textContent = "— — — —";
    $("pairHint").textContent = "Press the button to create a code";
    $("qrBox").classList.add("is-empty");
    $("qrBox").innerHTML = '<img class="qr-ghost" src="/icons/logo-192.png" alt="" width="72" height="72"><span>No code yet</span>';
  }
  $("pairNew").disabled = runtime.headless === false && false;

  // ---- connect info (moved from the old Access tab) ----
  if ($("connLan")) $("connLan").textContent = env.lanUrl || "—";
  if ($("connOmp")) $("connOmp").textContent = env.ompVersion || "not detected";
  if ($("connCwd")) $("connCwd").textContent = data.settings.ompCwd || "—";

  // ---- permissions ----
  const perms = $("permRows");
  perms.textContent = "";
  const screenBtn = document.createElement("button");
  screenBtn.className = "link";
  screenBtn.textContent = "Open settings";
  screenBtn.addEventListener("click", () => openPane("screen"));
  perms.append(row("Screen Recording", screenOk ? "on" : permissions.screen.error || "not allowed",
    screenOk ? pill("ok", "ok") : screenBtn));
  const inputBtn = document.createElement("button");
  inputBtn.className = "link";
  inputBtn.textContent = "Open settings";
  inputBtn.addEventListener("click", () => openPane("input"));
  const inputRow = row("Accessibility (mouse/keyboard)", inputOk ? "on" : "not allowed",
    inputOk ? pill("ok", "ok") : inputBtn);
  if (!inputOk && permissions.input.live) {
    // Granted in System Settings but the running process still reports false.
    const retry = document.createElement("button");
    retry.className = "link";
    retry.textContent = "Reload permissions";
    retry.addEventListener("click", async () => {
      retry.disabled = true;
      const res = await api("/api/setup");
      if (res.ok) render(await res.json());
      else retry.disabled = false;
    });
    inputRow.append(retry);
  }
  perms.append(inputRow);

  // ---- clients ----
  const clients = $("clients");
  clients.textContent = "";

  // Keys the mobile app asked us to approve (via QR scan on this Mac).
  const pending = pairing.pendingApprovals || [];
  for (const item of pending) {
    const ok = document.createElement("button");
    ok.className = "link";
    ok.innerHTML = '<svg class="bi"><use href="#i-check"/></svg> approve';
    ok.addEventListener("click", async () => {
      ok.disabled = true;
      await api("/api/pair/approve", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: item.key }),
      });
      await load({ quiet: true });
    });
    clients.append(row(`Awaiting approval: ${item.name || "device"}`, item.purpose || "login", ok));
  }
  // A device connected with an older session cookie has no pairing entry.
  // Surface it so "nothing connected" is never shown while a viewer is active.
  const sessionSecs = pairing.sessionPresence;
  const sessionActive = typeof sessionSecs === "number" && sessionSecs < 45;
  if (sessionActive) {
    const sess = document.createElement("div");
    sess.className = "row";
    const grow = document.createElement("div");
    grow.className = "grow";
    grow.innerHTML = '<b>Old session</b><small>Connected without pairing data</small>';
    sess.append(grow, pill("connected", "ok"));
    clients.append(sess);
  }

  if (!pairing.clients.length && !sessionActive) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No devices yet. Scan the QR in the Connect tab.";
    clients.append(empty);
  }
  for (const client of pairing.clients) {
    const online = !!client.online;
    const actions = document.createElement("div");
    actions.style.cssText = "display:flex;gap:8px;align-items:center";

    if (online) {
      const cut = document.createElement("button");
      cut.className = "link danger";
      cut.textContent = "disconnect";
      cut.addEventListener("click", () => revoke(client.id, client.name));
      actions.append(cut);
    }

    const del = document.createElement("button");
    del.className = "link danger";
    del.textContent = "remove";
    del.addEventListener("click", () => revoke(client.id, client.name));
    actions.append(del);

    const seen = client.last_seen
      ? new Date(client.last_seen * 1000).toLocaleString("en-US")
      : "new";
    const label = `${client.platform || "web"} · ${online ? "now" : seen}`;
    const statusPill = pill(online ? "connected" : "offline", online ? "ok" : "");

    const rowEl = row(client.name || client.id, label, actions);
    rowEl.insertBefore(statusPill, actions);
    clients.append(rowEl);
  }
  if (pairing.clients.length) {
    const all = document.createElement("button");
    all.className = "link danger";
    all.textContent = `remove all (${pairing.clients.length})`;
    all.style.marginTop = "8px";
    all.addEventListener("click", () => revokeAll());
    clients.append(all);
  }

  loadAutostart();
  fillForm(data.settings);
  $("footInfo").textContent = `data ${device.dataDir} · session ${env.sessionTtlDays} days`;
  $("footVer").textContent = `v${runtime.version} · ${runtime.headless ? "headless" : "desktop"}`;
}

function fillForm(settings) {
  const form = $("cfgForm");
  const set = (name, value) => {
    const el = form.elements[name];
    if (!el || document.activeElement === el) return;
    el.value = value ?? "";
  };
  set("deviceName", settings.deviceName);
  set("port", settings.port);
  set("publicUrl", settings.publicUrl);
  set("captureFps", String(settings.captureFps));
  set("jpegQuality", String(settings.jpegQuality));
  set("resolution", settings.resolution);
  set("sessionTtlDays", settings.sessionTtlDays);
  set("ompCwd", settings.ompCwd);
  set("allowedRoots", (settings.allowedRoots || []).join(", "));
  set("ompBin", settings.ompBin);
  set("relayKey", settings.relayKey);
  const pairing = form.elements["requirePairing"];
  if (pairing && document.activeElement !== pairing) pairing.checked = !!settings.requirePairing;
}

function readForm() {
  const form = $("cfgForm");
  const value = (name) => form.elements[name]?.value?.trim() ?? "";
  return {
    deviceName: value("deviceName"),
    port: Number(value("port")) || undefined,
    publicUrl: value("publicUrl"),
    captureFps: Number(value("captureFps")) || undefined,
    jpegQuality: Number(value("jpegQuality")) || undefined,
    resolution: value("resolution"),
    sessionTtlDays: Number(value("sessionTtlDays")) || undefined,
    ompCwd: value("ompCwd"),
    ompBin: value("ompBin"),
    relayKey: value("relayKey"),
    allowedRoots: value("allowedRoots")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
    requirePairing: !!form.elements["requirePairing"]?.checked,
  };
}

async function save({ restart = false } = {}) {
  const payload = readForm();
  setMsg("saving…");
  try {
    const res = await api("/api/setup", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    const data = await res.json();
    if (!res.ok || !data.ok) {
      setMsg(data.error || `failed to save (${res.status})`);
      return false;
    }
    // One button: save, then restart only when the server says a change needs
    // it. Two save buttons with different behaviour was the confusing part.
    const needsRestart = Array.isArray(data.needsRestart) && data.needsRestart.length > 0;
    setMsg(needsRestart ? "saved · applying…" : "saved", true);
    $("restartHint")?.classList.toggle("hidden", !needsRestart);
    if (needsRestart) await doRestart();
    return true;
  } catch (err) {
    setMsg(`failed: ${err.message}`);
    return false;
  }
}

async function doRestart() {
  setMsg("restarting…");
  try {
    await api("/api/setup/restart", { method: "POST" });
  } catch {}
  // The server drops the connection; poll until it answers again.
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 700));
    try {
      const res = await api("/api/setup");
      if (res.ok) {
        setMsg("restart complete", true);
        await load();
        return;
      }
    } catch {}
  }
  setMsg("server did not come back — check the log", false);
}

async function pairNew() {
  $("pairNew").disabled = true;
  try {
    const res = await api("/api/pair/new", { method: "POST" });
    const data = await res.json();
    if (!res.ok || !data.ok) {
      $("pairHint").textContent = "failed to create code";
      return;
    }
    startPairCountdown(data.expiresIn || 300);
    await load({ quiet: true });
  } catch {
    $("pairHint").textContent = "failed to create code";
  } finally {
    $("pairNew").disabled = false;
  }
}

function startPairCountdown(seconds) {
  clearInterval(pairTimer);
  let left = seconds;
  pairTimer = setInterval(() => {
    left -= 1;
    if (left <= 0) {
      clearInterval(pairTimer);
      $("pairHint").textContent = "Expired — create a new code";
      $("qrBox").innerHTML = '<div class="qr-empty">Code expired</div>';
      return;
    }
    $("pairHint").textContent = `Valid for ${Math.floor(left / 60)}m ${left % 60}s · single use`;
  }, 1000);
}

/// Ask inside the page instead of using window.confirm (unreliable in WKWebView).
function askConfirm(message) {
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.className = "ask-back";
    const card = document.createElement("div");
    card.className = "ask-card-ui";
    const text = document.createElement("p");
    text.textContent = message;
    const row = document.createElement("div");
    row.className = "ask-actions-ui";
    const no = document.createElement("button");
    no.className = "btn ghost";
    no.textContent = "Cancel";
    const yes = document.createElement("button");
    yes.className = "btn primary";
    yes.textContent = "Yes, continue";
    row.append(no, yes);
    card.append(text, row);
    back.append(card);
    document.body.append(back);
    const done = (value) => { back.remove(); resolve(value); };
    no.addEventListener("click", () => done(false));
    yes.addEventListener("click", () => done(true));
    back.addEventListener("click", (e) => { if (e.target === back) done(false); });
  });
}

async function revoke(id, name) {
  const ok = await askConfirm(`Disconnect “${name || id}”? That device must pair again to sign in.`);
  if (!ok) return;
  try {
    const res = await api("/api/pair/revoke", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id }),
    });
    const out = await res.json().catch(() => ({}));
    if (!res.ok || !out.ok) {
      setMsg(`failed to disconnect: ${out.error || res.status}`);
      return;
    }
    setMsg(`${name || id} disconnected`, true);
  } catch (err) {
    setMsg(`failed: ${err.message}`);
    return;
  }
  await load({ quiet: true });
}

async function revokeAll() {
  const ok = await askConfirm("Remove all trusted devices? Every device must pair again.");
  if (!ok) return;
  try {
    const res = await api("/api/pair/revoke", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ all: true }),
    });
    const out = await res.json().catch(() => ({}));
    if (!res.ok || !out.ok) {
      setMsg(`failed to remove: ${out.error || res.status}`);
      return;
    }
    setMsg("All devices removed", true);
  } catch (err) {
    setMsg(`failed: ${err.message}`);
    return;
  }
  await load({ quiet: true });
}

async function openPane(pane) {
  await api("/api/setup/open", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ pane }),
  });
}

async function loadAutostart() {
  try {
    const res = await api("/api/setup/autostart");
    if (!res.ok) return;
    const data = await res.json();
    const box = $("prefAutostart");
    box.checked = !!data.enabled;
    box.disabled = !data.supported;
    box.closest(".checkrow").title = data.supported ? "" : "macOS app only";
  } catch {}
}

$("prefAutostart").addEventListener("change", async (event) => {
  const enabled = event.target.checked;
  const res = await api("/api/setup/autostart", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ enabled }),
  });
  if (!res.ok) {
    event.target.checked = !enabled;
    setMsg("Failed to change autostart");
    return;
  }
  setMsg(enabled ? "Autostart enabled" : "Autostart disabled", true);
});

$("copyId")?.addEventListener("click", async () => {
  const id = $("deviceId")?.textContent?.trim();
  if (!id || id === "—") return;
  try {
    await navigator.clipboard.writeText(id);
    setMsg("ID copied", true);
  } catch {}
});

$("pairNew").addEventListener("click", pairNew);

$("cfgForm").addEventListener("submit", (event) => {
  event.preventDefault();
  save();
});


load();
setInterval(() => {
  if (document.hidden) return;
  if (document.activeElement?.closest?.("#cfgForm")) return;
  load({ quiet: true });
}, 6000);

// A normal browser must not get the setup shell; fail visibly instead of
// rendering a dead page. Guarded so a missing node can never break the rest of
// this file (that is exactly how the folder picker disappeared once).
if (!window.__RA_DESKTOP__ && !location.search.includes("debug=1")) {
  // Fixed overlay: inserting it into .app would become a grid item and break
  // the whole layout.
  const banner = document.createElement("div");
  banner.className = "banner-warn";
  banner.textContent =
    "This page is for the desktop app. Open it from the Remote Agents menu bar item, or add ?debug=1.";
  document.body.append(banner);
}

/* ---------- folder chooser ---------- */
const picker = {
  field: null,
  path: null,
  parent: null,
};

function pickerOpen(fieldName) {
  picker.field = fieldName;
  $("pickerTitle").textContent =
    fieldName === "allowedRoots" ? "Choose allowed folder" : "Choose project folder";
  $("picker").classList.remove("hidden");
  const current = $("cfgForm")?.elements[fieldName]?.value?.split(",")[0]?.trim();
  pickerGo(current || "");
}

function pickerClose() {
  $("picker").classList.add("hidden");
  picker.field = null;
}

async function pickerGo(path) {
  const list = $("pickerList");
  list.innerHTML = '<div class="picker-empty">loading…</div>';
  try {
    const res = await api(`/api/fs?browse=1&path=${encodeURIComponent(path)}`);
    if (!res.ok) {
      list.innerHTML = `<div class="picker-empty">Cannot open (${res.status})</div>`;
      return;
    }
    const data = await res.json();
    picker.path = data.path;
    picker.parent = data.parent;
    picker.home = data.home || picker.home;
    $("pickerPath").textContent = data.path;
    renderCrumbs(data.path);
    list.textContent = "";

    const dirs = (data.entries || []).filter((e) => e.is_dir);
    if (!dirs.length) {
      list.innerHTML = '<div class="picker-empty">No subfolders.</div>';
      return;
    }

    // Two distinct affordances per row: the name opens the folder, the
    // "Choose" button picks it. Tapping the row no longer silently selects.
    for (const entry of dirs) {
      const row = document.createElement("div");
      row.className = "picker-item";

      const openBtn = document.createElement("button");
      openBtn.type = "button";
      openBtn.className = "pi-open";
      openBtn.innerHTML = '<svg class="ico" width="14" height="14"><use href="#i-folder"/></svg>';
      const label = document.createElement("span");
      label.textContent = entry.name;
      openBtn.append(label);
      openBtn.addEventListener("click", () => pickerGo(entry.path));

      const pick = document.createElement("button");
      pick.type = "button";
      pick.className = "pi-pick";
      pick.textContent = "Choose";
      pick.title = `Choose ${entry.name}`;
      pick.addEventListener("click", () => pickerTake(entry.path));

      row.append(openBtn, pick);
      list.append(row);
    }
  } catch (err) {
    list.innerHTML = `<div class="picker-empty">${err.message}</div>`;
  }
}

/// Clickable breadcrumb: every segment jumps straight to that folder.
function renderCrumbs(fullPath) {
  const bar = $("pickerPath");
  bar.textContent = "";
  if (!fullPath) return;

  const parts = fullPath.split("/").filter(Boolean);
  const segments = [];
  let built = "";
  for (const part of parts) {
    built += "/" + part;
    segments.push([part, built]);
  }

  segments.forEach(([name, full], index) => {
    if (index > 0) {
      const sep = document.createElement("span");
      sep.className = "crumb-sep";
      sep.textContent = "/";
      bar.append(sep);
    }
    const crumb = document.createElement("button");
    crumb.type = "button";
    crumb.className = "crumb";
    crumb.textContent = name;
    crumb.title = full;
    crumb.addEventListener("click", () => pickerGo(full));
    bar.append(crumb);
  });
  bar.scrollLeft = bar.scrollWidth;
}

function pickerTake(path) {
  if (!picker.field || !path) return;
  const input = $("cfgForm")?.elements[picker.field];
  if (input) input.value = path;
  pickerClose();
  setMsg("Folder selected — press Save", true);
}

function pickerChoose() {
  if (!picker.field || !picker.path) return;
  pickerTake(picker.path);
}

document.querySelectorAll("[data-pick]").forEach((btn) =>
  btn.addEventListener("click", () => pickerOpen(btn.dataset.pick))
);
$("pickerClose").addEventListener("click", pickerClose);
$("pickerChoose").addEventListener("click", pickerChoose);
$("pickerUp").addEventListener("click", () => { if (picker.parent) pickerGo(picker.parent); });
$("pickerHome").addEventListener("click", () => pickerGo(picker.home || ""));
$("picker").addEventListener("click", (event) => {
  if (event.target === $("picker")) pickerClose();
});
