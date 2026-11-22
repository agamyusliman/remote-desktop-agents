mod auth;
mod capture;
mod config;
mod desktop;
mod identity;
mod input;
mod pairing;
mod projects;
mod rpc;
mod sysmon;
mod terminal;

use anyhow::{Context, Result};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use serde::Deserialize;
use serde_json::json;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    cfg: Arc<config::Config>,
    auth: Arc<auth::Auth>,
    capture: Arc<capture::Capture>,
    input: Arc<input::Input>,
    projects: Arc<projects::Projects>,
    pairing: Arc<pairing::Pairing>,
    rpc: Arc<rpc::RpcHub>,
    sysmon: Arc<sysmon::SysMon>,
    started: Instant,
    /// Incremented on "log out all"; sockets watching a stale value close.
    epoch: Arc<std::sync::atomic::AtomicU64>,
}

/// Returns a lock file handle when this is the only instance for this port.
fn single_instance(cfg: &config::Config) -> Option<std::fs::File> {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("remot-agent-{}.lock", cfg.port));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .ok()?;
    // A live instance keeps the port busy; if the port is free the old lock is
    // stale, so overwrite it.
    let in_use = std::net::TcpListener::bind((cfg.host.as_str(), cfg.port)).is_err();
    if in_use {
        return None;
    }
    let _ = file.set_len(0);
    let _ = writeln!(file, "{}", std::process::id());
    Some(file)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "remot_agent=info,tower_http=warn".into()),
        )
        .init();

    // A bundled .app cannot rely on a shell wrapper: macOS TCC attaches the
    // permission to the Mach-O identity, and a wrapper script makes that
    // identity unstable. The binary therefore applies the bundle environment
    // itself — before config is read.
    config::apply_bundle_defaults();
    let cfg = config::Config::load()?;

    // Single instance: a second copy cannot bind the port anyway, and if it
    // does (different data dir) it silently forks omp sessions into duplicates.
    if let Some(guard) = single_instance(&cfg) {
        // Keep the lock for the whole process lifetime.
        std::mem::forget(guard);
    } else {
        eprintln!(
            "remot-agent already running on port {}; open the existing window instead.",
            cfg.port
        );
        return Ok(());
    }
    if std::env::args().any(|a| a == "--print-identity") {
        let identity = identity::Identity::load(&cfg.data_dir)?;
        println!("id={}", identity.id);
        println!("public_url={}", cfg.public_url.clone().unwrap_or_default());
        return Ok(());
    }
    let headless = desktop::is_headless();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (url_tx, url_rx) = std::sync::mpsc::channel::<String>();
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cfg_thread = cfg.clone();
    let shutdown_thread = shutdown.clone();
    let server = runtime.spawn(async move {
        if let Err(err) = run_server(cfg_thread, url_tx).await {
            eprintln!("server: {err}");
            shutdown_thread.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });

    let ready_url = url_rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}/", cfg.port));

    if headless {
        runtime.block_on(server)?;
        return Ok(());
    }

    let result = desktop::run(cfg, ready_url, shutdown.clone());
    shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    runtime.block_on(server)?;
    result
}

async fn run_server(
    cfg: config::Config,
    url_tx: std::sync::mpsc::Sender<String>,
) -> Result<()> {
    let identity = identity::Identity::load(&cfg.data_dir)?;
    let mut cfg = cfg;
    cfg.device_id = identity.id.clone();
    let cfg = cfg;
    let auth = Arc::new(auth::Auth::new(
        identity.id.clone(),
        identity.token.clone(),
        cfg.secret()?,
        cfg.session_ttl_days,
        !cfg.require_pairing,
    ));
    let input = Arc::new(input::Input::new());
    let capture = capture::Capture::start(cfg.clone());
    let project_library = Arc::new(projects::Projects::load(&cfg)?);
    let rpc = Arc::new(rpc::RpcHub::new(cfg.omp_bin.clone()));
    let sysmon = sysmon::SysMon::start();
    let pairing = Arc::new(pairing::Pairing::load(&cfg.data_dir));

    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        auth,
        capture,
        input,
        projects: project_library,
        pairing,
        rpc,
        sysmon,
        started: Instant::now(),
        epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };

    let csp = HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; connect-src 'self' ws: wss:; font-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    );

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/logout-all", post(logout_all))
        .route("/api/session-delete", post(session_delete))
        .route("/api/status", get(status))
        .route("/api/projects", get(projects_list).post(projects_add))
        .route("/api/projects/{id}", axum::routing::delete(projects_delete))
        .route("/api/system", get(system_info))
        .route("/api/fs", get(fs_list))
        .route("/api/config", get(api_config))
        .route("/api/setup", get(api_setup).post(save_setup))
        .route("/api/setup/restart", post(restart_server))
        .route("/api/setup/open", post(open_pane))
        .route("/api/setup/autostart", get(autostart_state).post(set_autostart))
        .route("/api/pair/new", post(pair_new))
        .route("/api/pair/claim", post(pair_claim))
        .route("/api/pair/clients", get(pair_clients))
        .route("/api/pair/revoke", post(pair_revoke))
        .route("/api/pair/request", post(pair_request))
        .route("/api/pair/approve", post(pair_approve))
        .route("/api/identity", get(api_identity))
        .route("/api/ping", get(api_ping))
        .route("/api/sessions", get(api_sessions))
        .route("/api/project-sessions", get(api_project_sessions))
        .route("/api/tmux", get(api_tmux))
        .route("/api/session-label", post(session_label))
        .route("/api/files", get(files_list))
        .route("/api/browse", get(api_browse))
        .route("/api/files/read", get(files_read))
        .route("/api/files/raw", get(files_raw))
        .route("/auth", get(auth_link))
        .route("/desktop", get(desktop_page))
        .route("/desktop/qr.svg", get(desktop_qr))
        .route("/desktop/setup-qr.svg", get(setup_qr))
        .route("/ws/screen", get(ws_screen))
        .route("/ws/term", get(ws_term))
        .route("/ws/rpc", get(ws_rpc))
        .route("/", get(app_shell))
        .route("/index.html", get(app_shell))
        .fallback_service({
            let public = config::Config::public_dir();
            ServeDir::new(&public).fallback(ServeFile::new(public.join("index.html")))
        })
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            csp,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("service-worker-allowed"),
            HeaderValue::from_static("/"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(self), microphone=(), geolocation=()"),
        ))
        // A cached API body from an earlier build is indistinguishable from a
        // live one: it made the app boot into the wrong screen. API answers are
        // never reusable, including the 401/409 paths.
        .layer(axum::middleware::from_fn(no_store_api))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .with_context(|| "invalid HOST/PORT")?;
    let listener = TcpListener::bind(addr).await?;
    print_banner(&cfg);
    let _ = url_tx.send(format!("http://127.0.0.1:{}/", cfg.port));

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn print_banner(cfg: &config::Config) {
    let base = cfg.base_url();
    let login = format!("{base}/auth");
    println!();
    println!("  remot-agent");
    println!("  ├─ local:  {base}");
    println!("  ├─ omp:    tmux session '{}' in {}", cfg.session, cfg.cwd.display());
    println!("  ├─ capture: {} @ {} fps, jpeg q{}", cfg.resolution.label(), cfg.fps, cfg.quality);
    println!("  └─ device: {}  (ID or scan QR)", cfg.device_id);
    println!();
    println!("  open: {login}");
    println!("  login: enter the device ID on the login page, or open the URL containing it.");
    println!();
    if let Ok(code) = QrCode::new(login.as_bytes()) {
        let qr = code
            .render::<Dense1x2>()
            .quiet_zone(true)
            .module_dimensions(1, 1)
            .build();
        println!("{qr}");
    }
    println!("  Treat this URL as a password. It grants full control.");
    println!();
}

#[derive(Deserialize)]
struct LoginReq {
    token: String,
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Response {
    let ip = addr.ip();
    if !state.auth.allow_attempt(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many attempts").into_response();
    }
    if !state.auth.check(&req.token) {
        tokio::time::sleep(Duration::from_millis(400)).await;
        warn!("failed login from {ip}");
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }
    state.auth.reset_attempts(ip);
    info!("login from {ip}");
    let mut res = Json(json!({ "ok": true })).into_response();
    attach_cookie(&mut res, &state, &headers);
    res
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut res = Json(json!({"ok": true})).into_response();
    let secure = if auth::request_is_https(&headers) { "; Secure" } else { "" };
    let name = if headers.contains_key("x-forwarded-proto") {
        state.auth.relay_cookie_name()
    } else {
        state.auth.cookie_name()
    };
    let mut clears: Vec<HeaderValue> = Vec::new();
    let clear = format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}");
    if let Ok(value) = HeaderValue::from_str(&clear) {
        clears.push(value);
    }
    // Paired-client cookies are HttpOnly, so the app cannot delete them itself.
    // Expire every one this request carried or the "logout" leaves the session
    // fully authenticated.
    if let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in raw.split(';') {
            let cookie_name = part.splitn(2, '=').next().unwrap_or("").trim();
            if !cookie_name.starts_with("ra_c_") {
                continue;
            }
            let clear = format!("{cookie_name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}");
            if let Ok(value) = HeaderValue::from_str(&clear) {
                clears.push(value);
            }
        }
    }
    res.headers_mut().remove(header::SET_COOKIE);
    for value in clears {
        res.headers_mut().append(header::SET_COOKIE, value);
    }
    res
}

async fn logout_all(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.auth.check_headers(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Rotate token + secret on disk; all cookies become invalid immediately.
    let dir = state.cfg.data_dir.clone();
    let new_token = match auth::rotate(&dir) {
        Ok(token) => token,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    // Revoking cookies is not enough: paired keys live in clients.json and
    // existing sockets keep streaming. Clear both.
    let _ = state.pairing.revoke_all();
    state.pairing.clear_presence();
    state
        .epoch
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    info!("logged out every device: cookies rotated, pairings revoked, sockets closed");
    Json(json!({"ok": true, "token": new_token})).into_response()
}


fn attach_cookie(res: &mut Response, state: &AppState, headers: &HeaderMap) {
    let (cookie, max_age) = state.auth.cookie();
    let secure = if auth::request_is_https(headers) {
        "; Secure"
    } else {
        ""
    };
    let name = if headers.contains_key("x-forwarded-proto") {
        state.auth.relay_cookie_name()
    } else {
        state.auth.cookie_name()
    };
    let value = format!(
        "{name}={cookie}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        res.headers_mut().insert(header::SET_COOKIE, value);
    }
}

/// Local onboarding/setup shell. Loopback only, and the page must send the
/// desktop header so a random website cannot read it through the browser.
/// The setup UI is loopback-only. `is_loopback` checks the `Host` header, so a
/// DNS-rebinding page (which sends its own hostname) is refused.
fn is_local_ui(headers: &HeaderMap) -> bool {
    is_loopback(headers) && !is_cross_site(headers)
}

/// Same-origin check for state-changing calls coming from the desktop shell.
fn is_cross_site(headers: &HeaderMap) -> bool {
    match headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
    {
        Some(value) => value != "same-origin" && value != "none",
        None => false,
    }
}

fn local_or_auth(state: &AppState, headers: &HeaderMap) -> bool {
    state.auth.check_headers(headers) || is_local_ui(headers) || paired(state, headers)
}

/// True when the request carries a client key minted by pairing on this Mac.
async fn no_store_api(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    if is_api {
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
    }
    res
}

fn paired(state: &AppState, headers: &HeaderMap) -> bool {
    // Check every client cookie: re-pairing adds a new one and leaves the old
    // (now unknown) cookie in place, which used to shadow the live key.
    for key in state.auth.client_keys_from_headers(headers) {
        if state.pairing.is_known(&key) {
            // Record activity so the desktop can show which phone is connected.
            state.pairing.touch(&key);
            return true;
        }
    }
    false
}

fn allow(state: &AppState, headers: &HeaderMap) -> bool {
    state.auth.check_headers(headers) || paired(state, headers)
}

async fn desktop_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return Redirect::to("/").into_response();
    }
    let dir = config::Config::public_dir();
    let Ok(bytes) = tokio::fs::read(dir.join("setup.html")).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "setup ui missing").into_response();
    };
    let mut html = String::from_utf8_lossy(&bytes).to_string();
    // Tag every local asset with its mtime so an updated build cannot render
    // with a cached stylesheet (which broke the layout once already).
    for asset in ["setup.css", "setup.js"] {
        let stamp = std::fs::metadata(dir.join(asset))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let needle = format!("/{}", asset) + "\u{22}";
        let replacement = format!("/{}?v={}", asset, stamp) + "\u{22}";
        html = html.replace(&needle, &replacement);
    }
    let mut res = axum::response::Html(html).into_response();
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    res
}

const AUTOSTART_LABEL: &str = "dev.remoteagents.app";

fn autostart_plist_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{AUTOSTART_LABEL}.plist")))
}

fn autostart_command() -> Option<PathBuf> {
    // Prefer the launcher inside the bundle; fall back to the current binary.
    let exe = std::env::current_exe().ok()?;
    let launcher = exe.parent()?.join("remot-agent-launch");
    if launcher.is_file() {
        return Some(launcher);
    }
    Some(exe)
}

async fn autostart_state(headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let enabled = autostart_plist_path().map(|p| p.is_file()).unwrap_or(false);
    let headless = crate::desktop::is_headless();
    Json(json!({ "enabled": enabled, "supported": cfg!(target_os = "macos") && !headless })).into_response()
}

#[derive(Deserialize)]
struct AutostartReq {
    enabled: bool,
}

async fn set_autostart(headers: HeaderMap, Json(req): Json<AutostartReq>) -> Response {
    if !is_local_ui(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(path) = autostart_plist_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no home").into_response();
    };
    if !req.enabled {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", "-w", &path.display().to_string()])
            .output();
        let _ = std::fs::remove_file(&path);
        return Json(json!({ "ok": true, "enabled": false })).into_response();
    }
    let Some(cmd) = autostart_command() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no executable").into_response();
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{AUTOSTART_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>StandardOutPath</key><string>/tmp/remote-agents.log</string>
  <key>StandardErrorPath</key><string>/tmp/remote-agents.log</string>
</dict>
</plist>
"#,
        html_escape(&cmd.display().to_string())
    );
    if let Err(err) = std::fs::write(&path, plist) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w", &path.display().to_string()])
        .output();
    Json(json!({ "ok": true, "enabled": true })).into_response()
}

/// The app shell, with asset URLs stamped by file mtime. Browsers and the
/// service worker then always pick up a new build after a redeploy.
async fn app_shell() -> Response {
    let dir = config::Config::public_dir();
    let Ok(bytes) = tokio::fs::read(dir.join("index.html")).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "ui missing").into_response();
    };
    let mut html = String::from_utf8_lossy(&bytes).to_string();
    for asset in ["style.css", "app.js"] {
        let stamp = std::fs::metadata(dir.join(asset))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let quote = char::from(34).to_string();
        let needle = format!("/{}{}", asset, quote);
        let replacement = format!("/{}?v={}{}", asset, stamp, quote);
        html = html.replace(&needle, &replacement);
    }
    let mut res = axum::response::Html(html).into_response();
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    res
}


/// One payload for the onboarding screen: effective settings, live diagnostics
/// and the paths a user may need to copy.
async fn api_setup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let capture = state.capture.status.lock().unwrap().clone();
    let settings = state.cfg.settings();
    let lan = state
        .cfg
        .lan_ip()
        .map(|ip| format!("http://{ip}:{}", state.cfg.port));
    let omp_version = std::process::Command::new(&state.cfg.omp_bin)
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty());
    Json(json!({
        "device": {
            "id": state.auth.public_id(),
            "name": state.cfg.device_name,
            "hostname": hostname(),
            "platform": std::env::consts::OS,
            "dataDir": state.cfg.data_dir.display().to_string(),
        },
        "settings": settings,
        "env": {
            "host": state.cfg.host,
            "port": state.cfg.port,
            "lanUrl": lan,
            "publicUrl": state.cfg.public_url,
            "sessionTtlDays": state.auth.ttl_days(),
            "ompVersion": omp_version,
            "tokenSource": if std::env::var("REMOTE_TOKEN").is_ok() { "env" } else { "data/token" },
            "settingsFile": config::Settings::path(&state.cfg.data_dir).display().to_string(),
        },
        "permissions": {
            "screen": { "ok": capture.ok, "error": capture.error },
            "input": {
                "ok": state.input.trusted(),
                "live": state.input.permission_state(),
            },
        },
        "pairing": {
            "clients": state.pairing.list_with_presence(),
            "code": state.pairing.pending_code(),
            "expiresIn": state.pairing.pending_expires_in(),
            "pendingApprovals": state.pairing.pending_approvals(),
            "sessionPresence": state.pairing.session_presence_secs(),
        },
        "security": {
            "mode": if state.cfg.public_url.is_some() { "relay-bootstrap" } else { "lan" },
            "credentials": "local",
            "relayStores": ["device key", "device id"],
            "requirePairing": state.cfg.require_pairing,
            "idLogin": !state.cfg.require_pairing,
        },
        "runtime": {
            "uptime": state.started.elapsed().as_secs(),
            "version": env!("CARGO_PKG_VERSION"),
            "headless": crate::desktop::is_headless(),
        }
    }))
    .into_response()
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::process::Command::new("scutil")
                .args(["--get", "LocalHostName"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default()
}

async fn save_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(next): Json<config::Settings>,
) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match config::Settings::save(&state.cfg.data_dir, &next) {
        Ok(restart) => Json(json!({ "ok": true, "needsRestart": restart })).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Restart in place so saved settings take effect. The child waits briefly for
/// this process to release the listening socket.
async fn restart_server(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(exe) = std::env::current_exe() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no executable").into_response();
    };
    let quoted = format!("'{}'", exe.display().to_string().replace('\'', "'\\''"));
    let spawned = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("sleep 1; exec {quoted}"))
        .env("REMOT_AGENT_RESTARTED", "1")
        .spawn();
    match spawned {
        Ok(_) => {
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                std::process::exit(0);
            });
            Json(json!({ "ok": true })).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct PaneReq {
    pane: String,
}

/// Open a macOS privacy pane so the user can grant a permission.
async fn open_pane(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<PaneReq>) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Trigger the system prompt once, then open the exact settings pane.
    if req.pane == "screen" {
        let _ = scap::request_permission();
    }
    let target = match req.pane.as_str() {
        "screen" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
        "input" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        _ => return (StatusCode::BAD_REQUEST, "unknown pane").into_response(),
    };
    let ok = std::process::Command::new("open").arg(target).spawn().is_ok();
    Json(json!({ "ok": ok })).into_response()
}

/// QR for the pairing payload. Deliberately rendered server-side so the code
/// never has to travel through a third-party QR service.
async fn setup_qr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !is_local_ui(&headers) && !state.auth.check_headers(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(active) = state.pairing.pending_code() else {
        return StatusCode::GONE.into_response();
    };
    let submitted: String = q
        .get("c")
        .cloned()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if submitted != active {
        return StatusCode::GONE.into_response();
    }
    let payload = state.cfg.pair_link(&active);
    let Ok(code) = QrCode::new(payload.as_bytes()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .quiet_zone(true)
        .min_dimensions(320, 320)
        .build();
    let mut res = svg.into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn desktop_qr(State(state): State<AppState>, Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let id = q.get("id").cloned().unwrap_or_default();
    if !state.auth.token_matches(&id) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let base = state.cfg.public_url.clone().unwrap_or_else(|| {
        format!("http://{}:{}", state.cfg.lan_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "127.0.0.1".into()), state.cfg.port)
    });
    let login = format!("{base}/auth?id={id}");
    let Ok(code) = qrcode::QrCode::new(login.as_bytes()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    let mut res = svg.into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    res
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn is_loopback(headers: &HeaderMap) -> bool {
    headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .map(|h| h.starts_with("127.0.0.1") || h.starts_with("localhost") || h.starts_with("[::1]"))
        .unwrap_or(false)
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
    id: Option<String>,
    next: Option<String>,
}

async fn auth_link(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let next = query
        .next
        .as_deref()
        .filter(|n| n.starts_with('/') && !n.starts_with("//"))
        .unwrap_or("/");
    let candidate = query.token.or(query.id);
    if let Some(token) = candidate {
        if state.auth.check(&token) {
            let mut res = Redirect::to(next).into_response();
            attach_cookie(&mut res, &state, &headers);
            return res;
        }
    }
    Redirect::to("/").into_response()
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !local_or_auth(&state, &headers) {
        if let Some(key) = state.auth.client_key_from_headers(&headers) {
            if state.pairing.is_known(&key) {
                state.pairing.touch(&key);
            }
        }
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let capture = state.capture.status.lock().unwrap().clone();
    let (screen_w, screen_h) = state.input.screen_size();
    Json(json!({
        "capture": capture,
        "input": {
            "trusted": state.input.trusted(),
            "width": screen_w,
            "height": screen_h,
        },
        "network": {
            "address": state.cfg.lan_ip().map(|ip| ip.to_string()),
            "port": state.cfg.port,
            "publicUrl": state.cfg.public_url,
            "sessionTtlDays": state.auth.ttl_days(),
        },
        "deviceId": state.auth.public_id(),
        "deviceUrl": state.cfg.public_url.clone().unwrap_or_default(),
        "deviceName": state.cfg.device_name,
        "session": state.cfg.session,
        "cwd": state.cfg.cwd.display().to_string(),
        "screenClients": state.capture.client_count(),
        "uptime": state.started.elapsed().as_secs(),
    }))
    .into_response()
}

async fn ws_screen(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Note the credential used. Sessions minted before pairing existed still
    // work, but they used to be invisible in the Devices list.
    let known = state
        .auth
        .client_keys_from_headers(&headers)
        .into_iter()
        .find(|key| state.pairing.is_known(key));
    match known {
        Some(key) => state.pairing.touch(&key),
        None => state.pairing.note_session(),
    }
    ws.on_upgrade(move |socket| screen_socket(socket, state))
}

async fn screen_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.capture.tx.subscribe();
    state.capture.add_client();
    // This socket *is* the connection: keep presence fresh while it lives.
    state.pairing.note_session();
    let presence = state.pairing.clone();
    let epoch = state.epoch.clone();
    let socket_epoch = epoch.load(std::sync::atomic::Ordering::Relaxed);
    let presence_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // "Log out all" bumps the epoch: stop pretending this session lives.
            if epoch.load(std::sync::atomic::Ordering::Relaxed) != socket_epoch {
                break;
            }
            presence.note_session();
        }
    });
    let initial = { state.capture.latest.lock().unwrap().clone() };
    if let Some(frame) = initial {
        if socket
            .send(Message::Binary(frame.to_vec().into()))
            .await
            .is_err()
        {
            state.capture.remove_client();
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                            state.input.handle(&value);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            frame = rx.recv() => {
                match frame {
                    Ok(frame) => {
                        if socket.send(Message::Binary(frame.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }
    }
    presence_task.abort();
    state.capture.remove_client();
}

async fn projects_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if state.projects.remove(&id) {
        Json(json!({ "ok": true })).into_response()
    } else {
        (StatusCode::NOT_FOUND, "unknown project").into_response()
    }
}

async fn projects_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({
        "projects": state.projects.list(),
        "roots": state.projects.roots(),
        "default": state.projects.default_id(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct FileQuery {
    project: Option<String>,
    path: Option<String>,
    hidden: Option<bool>,
}

fn project_root(state: &AppState, id: Option<&str>) -> Option<(projects::Project, PathBuf)> {
    let project = match id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => state.projects.get(id),
        None => state.projects.default_project(),
    }?;
    let root = std::fs::canonicalize(&project.path).ok()?;
    Some((project, root))
}

fn safe_join(root: &std::path::Path, rel: &str) -> Option<PathBuf> {
    // Accept both project-relative paths ("src/main.rs") and absolute paths
    // ("/Users/me/project/src/main.rs"). The UI always sends absolute paths,
    // which used to be joined onto the root and rejected as "outside project".
    let candidate = if rel.starts_with('/') {
        PathBuf::from(rel)
    } else {
        root.join(rel.trim_start_matches('/'))
    };
    let canonical = std::fs::canonicalize(&candidate).ok()?;
    if canonical.starts_with(root) {
        Some(canonical)
    } else {
        None
    }
}

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", ".venv", "venv", "dist", "build", ".next",
    ".turbo", "__pycache__", ".cache", "vendor", "Pods", ".idea", ".DS_Store",
];

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

/// Directory browser for "add project". Limited to allowed roots, which is the
/// same boundary the rest of the app enforces; it exists because the project
/// file explorer is scoped to an existing project and cannot be used to pick a
/// new one.
async fn api_browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BrowseQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let roots: Vec<PathBuf> = state
        .cfg
        .allowed_roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    if roots.is_empty() {
        return (StatusCode::FORBIDDEN, "no allowed roots").into_response();
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let requested = query.path.unwrap_or_default();
    let requested = requested.trim();
    let candidate = if requested.is_empty() {
        PathBuf::from(&home)
    } else if requested == "~" {
        PathBuf::from(&home)
    } else if let Some(rest) = requested.strip_prefix("~/") {
        PathBuf::from(&home).join(rest)
    } else {
        PathBuf::from(requested)
    };

    let dir = match std::fs::canonicalize(&candidate) {
        Ok(dir) if dir.is_dir() => dir,
        _ => return (StatusCode::BAD_REQUEST, "not a directory").into_response(),
    };
    if !roots.iter().any(|root| dir.starts_with(root)) {
        return (StatusCode::FORBIDDEN, "outside allowed roots").into_response();
    }

    let mut dirs = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                dirs.push(json!({
                    "name": name,
                    "abs": path.display().to_string(),
                }));
            }
        }
    }
    dirs.sort_by(|a, b| {
        a["name"].as_str().unwrap_or("").to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    let parent = dir
        .parent()
        .filter(|p| roots.iter().any(|root| p.starts_with(root)))
        .map(|p| p.display().to_string());

    Json(json!({
        "path": dir.display().to_string(),
        "parent": parent,
        "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
        "dirs": dirs,
    }))
    .into_response()
}

async fn files_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((project, root)) = project_root(&state, query.project.as_deref()) else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let rel = query.path.unwrap_or_default();
    let Some(dir) = safe_join(&root, &rel) else {
        return (StatusCode::FORBIDDEN, "path outside project").into_response();
    };
    if !dir.is_dir() {
        return (StatusCode::BAD_REQUEST, "not a directory").into_response();
    }
    let show_hidden = query.hidden.unwrap_or(false);
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten().take(1000) {
            let name = entry.file_name().to_string_lossy().to_string();
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let is_dir = path.is_dir();
            let size = if is_dir { 0 } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
            let rel_path = path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            entries.push(serde_json::json!({
                "name": name,
                "path": rel_path,
                "abs": path.display().to_string(),
                "is_dir": is_dir,
                "size": size,
                "ext": ext,
                "markdown": ext == "md" || ext == "mdx",
                "image": matches!(ext.as_str(), "png"|"jpg"|"jpeg"|"gif"|"webp"|"svg"|"bmp"|"ico"),
                "code": !is_dir && !matches!(ext.as_str(), "png"|"jpg"|"jpeg"|"gif"|"webp"|"svg"|"bmp"|"ico"|"pdf"|"zip"|"gz"|"tar"|"mp4"|"mov"|"mp3"|"wav"),
            }));
        }
    }
    entries.sort_by(|a, b| {
        let ad = a.get("is_dir").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let bd = b.get("is_dir").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let an = a.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
        let bn = b.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
        bd.cmp(&ad).then(an.to_lowercase().cmp(&bn.to_lowercase()))
    });
    let parent = dir
        .parent()
        .filter(|p| p.starts_with(&root) && *p != root)
        .map(|p| {
            p.strip_prefix(&root)
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    let rel_dir = dir
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Json(serde_json::json!({
        "project": project,
        "root": root.display().to_string(),
        "path": rel_dir,
        "parent": parent,
        "entries": entries,
    }))
    .into_response()
}

async fn files_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((_, root)) = project_root(&state, query.project.as_deref()) else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let rel = query.path.unwrap_or_default();
    let Some(file) = safe_join(&root, &rel) else {
        return (StatusCode::FORBIDDEN, "path outside project").into_response();
    };
    if !file.is_file() {
        return (StatusCode::BAD_REQUEST, "not a file").into_response();
    }
    let meta = match std::fs::metadata(&file) {
        Ok(meta) => meta,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    if meta.len() > 2 * 1024 * 1024 {
        return (StatusCode::PAYLOAD_TOO_LARGE, "file too large (>2MB)").into_response();
    }
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    let binary = bytes.iter().take(8192).any(|b| *b == 0);
    let content = if binary {
        String::new()
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let markdown = matches!(ext.as_str(), "md" | "markdown" | "mdx" | "mdown" | "mkd");
    let image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif");
    Json(serde_json::json!({
        "path": rel,
        "name": file.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        "ext": ext,
        "size": meta.len(),
        "binary": binary,
        "markdown": markdown,
        "image": image,
        "content": content,
    }))
    .into_response()
}

async fn files_raw(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((_, root)) = project_root(&state, query.project.as_deref()) else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let rel = query.path.unwrap_or_default();
    let Some(file) = safe_join(&root, &rel) else {
        return (StatusCode::FORBIDDEN, "path outside project").into_response();
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };
    let mime = mime_guess::from_path(&file).first_or_octet_stream();
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Deserialize)]
struct SessionsQuery {
    project: Option<String>,
}

async fn api_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SessionsQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let project = match query.project.as_deref() {
        Some(id) => state.projects.get(id),
        None => state.projects.default_project(),
    };
    let Some(project) = project else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let path = PathBuf::from(&project.path);
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = rpc::session_dir(&path);
    Json(serde_json::json!({
        "home": home,
        "dir": dir.display().to_string(),
        "dirExists": dir.exists(),
        "sessions": rpc::list_sessions(&path),
    }))
    .into_response()
}

async fn api_identity(State(state): State<AppState>) -> Response {
    // Public onboarding hint: the short ID is safe to show; the token is not.
    // CORS is intentionally open so the login page of one device can probe
    // another device's presence. No secret is returned here.
    let mut res = Json(serde_json::json!({
        "id": state.auth.public_id(),
        "name": state.cfg.device_name,
        "hostname": std::env::var("HOSTNAME").unwrap_or_default(),
    }))
    .into_response();
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    res
}

#[derive(Deserialize)]
struct ClaimBody {
    code: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    platform: String,
}

/// Desktop-only: mint a short-lived pairing code (shown as QR).
async fn pair_new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let issued = state.pairing.issue();
    let code_for_qr = issued.code.replace('-', "");
    // One source of truth for the link the QR encodes.
    let payload = state.cfg.pair_link(&code_for_qr);
    let base = state.cfg.external_base();
    Json(json!({
        "ok": true,
        "code": issued.code,
        "codePlain": code_for_qr,
        "expiresIn": issued.expires_in,
        "payload": payload,
        "pairUrl": format!("{base}/pair#c={code_for_qr}&d={}", state.auth.public_id()),
        "deviceId": state.auth.public_id(),
        "deviceName": state.cfg.device_name,
    }))
    .into_response()
}

/// Phone-side: exchange a pairing code for a client key. Rate limited by IP.
async fn pair_claim(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Response {
    let ip = addr.ip();
    if !state.auth.allow_attempt(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many attempts").into_response();
    }
    let req = pairing::ClaimRequest {
        code: body.code,
        name: body.name,
        platform: body.platform,
    };
    let Some(claim) = state
        .pairing
        .claim(&req, state.auth.public_id(), &state.cfg.device_name)
    else {
        tokio::time::sleep(Duration::from_millis(400)).await;
        return (StatusCode::UNAUTHORIZED, Json(json!({"ok": false}))).into_response();
    };
    state.auth.reset_attempts(ip);
    info!("paired new client {} ({})", claim.client_id, claim.device_name);
    let mut res = Json(json!({
        "ok": true,
        "clientKey": claim.client_key,
        "clientId": claim.client_id,
        "deviceId": claim.device_id,
        "deviceName": claim.device_name,
    }))
    .into_response();
    // Drop the client key in an HttpOnly cookie too, so the plain web app can
    // use the pairing without keeping the secret in localStorage.
    let secure = if auth::request_is_https(&headers) { "; Secure" } else { "" };
    let name = auth::Auth::client_cookie_name(&claim.device_id, &claim.client_key);
    let value = format!(
        "{name}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000{secure}",
        claim.client_key
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        res.headers_mut().insert(header::SET_COOKIE, value);
    }
    res
}

/// Mobile side: ask this Mac to approve a freshly minted key.
async fn pair_request(State(state): State<AppState>, Json(body): Json<RejectBody>) -> Response {
    let req = pairing::ApproveRequest {
        key: body.key,
        purpose: body.purpose,
        name: body.name,
    };
    if req.key.trim().len() < 16 {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": "key too short"}))).into_response();
    }
    let out = state
        .pairing
        .request(&req, state.auth.public_id(), &state.cfg.device_name);
    Json(json!({ "ok": true, "deviceId": out.device_id, "deviceName": out.device_name })).into_response()
}

/// Desktop side: approve a key scanned from the phone's QR.
async fn pair_approve(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<RejectBody>) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state.pairing.approve(&body.key) {
        return (StatusCode::NOT_FOUND, Json(json!({"ok": false, "error": "unknown code"}))).into_response();
    }
    info!("approved client key {}", &body.key.chars().take(8).collect::<String>());
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize, Default)]
struct RejectBody {
    #[serde(default)]
    key: String,
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    name: String,
}

async fn pair_clients(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({
        "clients": state.pairing.list_with_presence(),
        "pending": state.pairing.pending_expires_in(),
        "pendingApprovals": state.pairing.pending_approvals(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct RevokeBody {
    id: Option<String>,
    all: Option<bool>,
}

async fn pair_revoke(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<RevokeBody>) -> Response {
    if !is_local_ui(&headers) && !paired(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let result = if body.all.unwrap_or(false) {
        state.pairing.revoke_all().map(|_| true)
    } else if let Some(id) = body.id {
        state.pairing.revoke(&id)
    } else {
        return (StatusCode::BAD_REQUEST, "id or all required").into_response();
    };
    match result {
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

/// Presence only. Used by the relay so it can show "online" without learning
/// anything about this machine (no name, no ID, no hostname).
async fn api_ping() -> Response {
    let mut res = Json(json!({"ok": true})).into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn api_config(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !local_or_auth(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({
        "host": state.cfg.host,
        "port": state.cfg.port,
        "ompCwd": state.cfg.cwd.display().to_string(),
        "ompBin": state.cfg.omp_bin.display().to_string(),
        "allowedRoots": state.projects.roots(),
        "home": state.cfg.home.display().to_string(),
        "captureFps": state.cfg.fps,
        "jpegQuality": state.cfg.quality,
        "resolution": state.cfg.resolution.label(),
        "publicUrl": state.cfg.public_url,
        "tokenSource": if std::env::var("REMOTE_TOKEN").is_ok() { "env" } else { "data/token" },
        "sessionTtlDays": state.auth.ttl_days(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct FsQuery {
    path: Option<String>,
    browse: Option<String>,
}

async fn fs_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FsQuery>,
) -> Response {
    if !local_or_auth(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let roots = state.projects.roots();
    let root_paths: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    let Some(root) = root_paths.first() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no allowed roots").into_response();
    };
    // The setup window runs on this machine, so it may browse the whole home
    // directory to pick a workspace folder. Remote clients stay restricted.
    let local = is_local_ui(&headers);
    let browse_home = local || query.browse.as_deref() == Some("1");
    let requested = match query.path.as_deref() {
        None | Some("") | Some("~") | Some("home") => state.cfg.home.clone(),
        Some(p) => PathBuf::from(p),
    };
    let requested = if requested.is_file() {
        requested.parent().map(PathBuf::from).unwrap_or_else(|| root.clone())
    } else {
        requested
    };
    let Ok(path) = std::fs::canonicalize(&requested) else {
        return (StatusCode::NOT_FOUND, "path not found").into_response();
    };
    let within_home = path.starts_with(&state.cfg.home);
    if !root_paths.iter().any(|r| path.starts_with(r)) && !(browse_home && within_home) {
        return (StatusCode::FORBIDDEN, "path outside allowed roots").into_response();
    }

    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(&path) {
        for entry in read.flatten().take(600) {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry_path.is_dir();
            if !is_dir && entry_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                continue;
            }
            entries.push(serde_json::json!({
                "name": name,
                "path": entry_path.display().to_string(),
                "is_dir": is_dir,
                "project": is_dir && projects::looks_like_project(&entry_path),
            }));
        }
    }
    entries.sort_by(|a, b| {
        let ad = a.get("is_dir").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let bd = b.get("is_dir").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let an = a.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
        let bn = b.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
        bd.cmp(&ad).then(an.to_lowercase().cmp(&bn.to_lowercase()))
    });

    // One level of lazily-loaded children: the client can prefetch the folders
    // the user is about to open, which removes the perceived delay.
    let mut children: Vec<serde_json::Value> = Vec::new();
    if entries.len() < 400 {
        for entry in entries.iter().take(40) {
            if !entry.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            let Some(dir) = entry.get("path").and_then(|v| v.as_str()) else { continue };
            let mut names: Vec<String> = Vec::new();
            if let Ok(read) = std::fs::read_dir(dir) {
                for child in read.flatten().take(120) {
                    let name = child.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    names.push(name);
                }
            }
            names.sort_by_key(|n| n.to_lowercase());
            names.truncate(80);
            children.push(serde_json::json!({ "path": dir, "names": names }));
        }
    }

    let parent = path
        .parent()
        .filter(|p| {
            root_paths.iter().any(|r| p.starts_with(r)) || (browse_home && p.starts_with(&state.cfg.home))
        })
        .map(|p| p.display().to_string());

    Json(serde_json::json!({
        "path": path.display().to_string(),
        "parent": parent,
        "roots": roots,
        "home": state.cfg.home.display().to_string(),
        "entries": entries,
        "children": children,
    }))
    .into_response()
}

async fn system_info(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(state.sysmon.snapshot()).into_response()
}

#[derive(Deserialize)]
struct AddProject {
    /// Optional: the folder name is used when it is absent or blank.
    #[serde(default)]
    name: String,
    path: String,
}

async fn projects_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AddProject>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.projects.add(&req.name, &req.path) {
        Ok(project) => Json(project).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct RpcQuery {
    project: Option<String>,
}

async fn ws_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RpcQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let project = match query.project.as_deref() {
        Some(id) => state.projects.get(id),
        None => state.projects.default_project(),
    };
    let Some(project) = project else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    let path = PathBuf::from(&project.path);
    let process = match state.rpc.get(&project.id, &path).await {
        Ok(process) => process,
        Err(err) => return (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
    };
    let sessions = rpc::list_sessions(&path);
    ws.on_upgrade(move |socket| rpc_socket(socket, process, project, sessions))
}

async fn rpc_socket(
    mut socket: WebSocket,
    process: Arc<rpc::RpcProcess>,
    project: projects::Project,
    sessions: Vec<rpc::SessionInfo>,
) {
    let mut events = process.events.subscribe();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(64);

    let snapshot = serde_json::json!({
        "type": "rpc_snapshot",
        "project": project,
        "state": process.snapshot(),
        "sessions": sessions,
    });
    if socket.send(Message::Text(snapshot.to_string().into())).await.is_err() {
        return;
    }

    {
        let process = process.clone();
        let out = out_tx.clone();
        tokio::spawn(async move {
            if let Ok(value) = process
                .request(
                    serde_json::json!({"type":"get_messages_page","limit":200}),
                    Duration::from_secs(25),
                )
                .await
            {
                let _ = out
                    .send(serde_json::json!({"type":"transcript","data":value}).to_string())
                    .await;
            }
        });
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(text.as_str()) else { continue };
                        handle_rpc_command(&value, &process, &project, &out_tx).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(value) => {
                        if value.get("type").and_then(serde_json::Value::as_str) == Some("extension_ui_request") {
                            tracing::info!("rpc ui request: {}", value);
                        }
                        if socket.send(Message::Text(value.to_string().into())).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            Some(line) = out_rx.recv() => {
                if socket.send(Message::Text(line.into())).await.is_err() { break; }
            }
        }
    }
}

async fn handle_rpc_command(
    value: &serde_json::Value,
    process: &Arc<rpc::RpcProcess>,
    project: &projects::Project,
    out: &tokio::sync::mpsc::Sender<String>,
) {
    let kind = value.get("type").and_then(serde_json::Value::as_str).unwrap_or_default();
    match kind {
        "prompt" => {
            let message = value.get("text").and_then(serde_json::Value::as_str).unwrap_or_default();
            if !message.is_empty() {
                process.send_command(serde_json::json!({"type":"prompt","message":message}));
            }
        }
        "abort" => process.send_command(serde_json::json!({"type":"abort"})),
        "model" => {
            process.send_command(serde_json::json!({
                "type": "set_model",
                "provider": value.get("provider"),
                "modelId": value.get("modelId"),
            }));
        }
        "new_session" => process.send_command(serde_json::json!({"type":"new_session"})),
        "resume" => {
            if let Some(path) = value.get("sessionPath").and_then(serde_json::Value::as_str) {
                process.send_command(serde_json::json!({"type":"switch_session","sessionPath":path}));
            }
        }
        "ui_response" => {
            let mut response = serde_json::json!({
                "type": "extension_ui_response",
                "id": value.get("id"),
            });
            let obj = response.as_object_mut().unwrap();
            for key in ["value", "confirmed", "answers", "cancelled", "timedOut"] {
                if let Some(v) = value.get(key) {
                    obj.insert(key.to_string(), v.clone());
                }
            }
            process.send_command(response);
        }
        "sessions" => {
            let sessions = rpc::list_sessions(&PathBuf::from(&project.path));
            let _ = out
                .send(serde_json::json!({"type":"sessions","sessions":sessions}).to_string())
                .await;
        }
        "models" => {
            let process = process.clone();
            let out = out.clone();
            tokio::spawn(async move {
                if let Ok(models) = rpc::available_models(&process).await {
                    let _ = out
                        .send(serde_json::json!({"type":"models","models":models}).to_string())
                        .await;
                }
            });
        }
        "messages" => {
            let process = process.clone();
            let out = out.clone();
            tokio::spawn(async move {
                if let Ok(value) = process
                    .request(
                        serde_json::json!({"type":"get_messages_page","limit":200}),
                        Duration::from_secs(25),
                    )
                    .await
                {
                    let _ = out
                        .send(serde_json::json!({"type":"transcript","data":value}).to_string())
                        .await;
                }
            });
        }
        "get_state" => process.send_command(serde_json::json!({"type":"get_state"})),
        _ => {}
    }
}

#[derive(Deserialize)]
struct TermQuery {
    cols: Option<u16>,
    rows: Option<u16>,
    project: Option<String>,
    resume: Option<String>,
    /// Attach to an existing tmux session instead of spawning a new one.
    session: Option<String>,
}

async fn ws_term(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TermQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let cols = query.cols.unwrap_or(terminal::DEFAULT_COLS);
    let rows = query.rows.unwrap_or(terminal::DEFAULT_ROWS);
    let project = match query.project.as_deref() {
        Some(id) => state.projects.get(id),
        None => state.projects.default_project(),
    };
    let Some(project) = project else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    if let Some(key) = state.auth.client_key_from_headers(&headers) {
        state.pairing.touch(&key);
    } else {
        state.pairing.note_session();
    }
    let attach = query
        .session
        .filter(|name| terminal::session_exists(name));
    let target = terminal::Target {
        session: project.session,
        cwd: PathBuf::from(project.path),
        omp: state.cfg.omp_bin.clone(),
        resume: if attach.is_some() { None } else { query.resume },
        attach,
    };
    ws.on_upgrade(move |socket| terminal::run(socket, target, cols, rows))
}

/// Session list for the current project, used by the terminal empty state.
/// Terminals already running on this Mac, so the phone can join one instead of
/// starting a second agent with a separate context.
async fn api_tmux(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "sessions": terminal::list_tmux_sessions() })).into_response()
}

async fn api_project_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FsQuery>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((project, _root)) = project_root(&state, query.path.as_deref()) else {
        return (StatusCode::NOT_FOUND, "unknown project").into_response();
    };
    Json(json!({
        "project": project,
        "sessions": rpc::list_sessions(&PathBuf::from(&project.path)),
    }))
    .into_response()
}

/// Rewrites the session's own `title` records so omp shows the new name too.
/// Only the two header lines are touched; the transcript is copied verbatim.
async fn session_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let raw = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    let label = body.get("label").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return (StatusCode::BAD_REQUEST, "path is required").into_response();
    }
    if label.is_empty() {
        return (StatusCode::BAD_REQUEST, "label is required").into_response();
    }
    let Some(target) = session_file_path(raw) else {
        return (StatusCode::FORBIDDEN, "not a session file").into_response();
    };
    let Ok(content) = std::fs::read_to_string(&target) else {
        return (StatusCode::NOT_FOUND, "cannot read session").into_response();
    };

    let mut patched_title = false;
    let mut patched_session = false;
    let mut out = String::with_capacity(content.len() + label.len());
    for line in content.lines() {
        let mut replaced = None;
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) {
            match value.get("type").and_then(|v| v.as_str()) {
                Some("title") if !patched_title => {
                    value["title"] = serde_json::Value::String(label.to_string());
                    value["source"] = serde_json::Value::String("user".into());
                    patched_title = true;
                    replaced = Some(value);
                }
                Some("session") if !patched_session => {
                    value["title"] = serde_json::Value::String(label.to_string());
                    value["titleSource"] = serde_json::Value::String("user".into());
                    patched_session = true;
                    replaced = Some(value);
                }
                _ => {}
            }
        }
        match replaced {
            Some(value) => {
                out.push_str(&value.to_string());
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    if !patched_title && !patched_session {
        return (StatusCode::BAD_REQUEST, "session has no title record").into_response();
    }

    let tmp = target.with_extension("jsonl.tmp");
    if let Err(err) = std::fs::write(&tmp, out) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    if let Err(err) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    Json(json!({ "ok": true })).into_response()
}

/// Resolves a session path and refuses anything outside omp's session store.
fn session_file_path(raw: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let store = std::fs::canonicalize(PathBuf::from(&home).join(".omp/agent/sessions")).ok()?;
    let target = std::fs::canonicalize(raw).ok()?;
    if !target.starts_with(&store) || target.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return None;
    }
    Some(target)
}

/// Removes one session file so duplicated histories can be cleaned up. The
/// path must resolve inside omp's session store; anything else is refused.
async fn session_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !allow(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let raw = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return (StatusCode::BAD_REQUEST, "path is required").into_response();
    }
    let Some(target) = session_file_path(raw) else {
        return (StatusCode::FORBIDDEN, "not a session file").into_response();
    };
    if let Err(err) = std::fs::remove_file(&target) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    Json(json!({ "ok": true })).into_response()
}
