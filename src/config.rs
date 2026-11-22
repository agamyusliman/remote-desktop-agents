use anyhow::{Context, Result};
use base64::Engine;
use rand::RngCore;
use std::{
    fs,
    net::{IpAddr, UdpSocket},
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub session: String,
    pub cwd: PathBuf,
    pub fps: u32,
    pub quality: u8,
    pub resolution: CaptureResolution,
    pub data_dir: PathBuf,
    pub projects_file: PathBuf,
    pub allowed_roots: Vec<PathBuf>,
    pub home: PathBuf,
    pub omp_bin: PathBuf,
    pub public_url: Option<String>,
    pub session_ttl_days: u64,
    /// When true the public ID cannot be used to log in; only a paired client
    /// key or the long device token works.
    pub require_pairing: bool,
    /// Relay routing key for this device, if it sits behind the shared relay.
    pub relay_key: Option<String>,
    pub device_id: String,
    pub device_name: String,
}

/// User-editable settings persisted in `data/settings.json`. Every field is
/// optional: missing keys fall back to the environment/default.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub device_name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub public_url: Option<String>,
    pub capture_fps: Option<u32>,
    pub jpeg_quality: Option<u8>,
    pub resolution: Option<String>,
    pub omp_cwd: Option<String>,
    pub omp_bin: Option<String>,
    pub allowed_roots: Option<Vec<String>>,
    pub session_ttl_days: Option<u64>,
    /// Public device ID alone is not enough to log in once pairing is enabled.
    pub require_pairing: Option<bool>,
    /// Relay hostname used for pairing/QR. Purely a convenience default.
    pub relay_url: Option<String>,
    /// This device's key on the relay (e.g. "a"). Lets the phone reach
    /// `/pair` on the shared hostname before any device cookie exists.
    pub relay_key: Option<String>,
}

impl Settings {
    
pub fn path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    pub fn read(data_dir: &std::path::Path) -> Self {
        fs::read_to_string(Self::path(data_dir))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Validate and persist. Returns the list of fields that need a restart.
    pub fn save(data_dir: &std::path::Path, next: &Settings) -> Result<Vec<String>> {
        let mut errors = Vec::new();
        if let Some(name) = &next.device_name {
            if name.trim().is_empty() || name.chars().count() > 48 {
                errors.push("Device name must be 1–48 characters".into());
            }
        }
        if let Some(port) = next.port {
            if port < 1024 {
                errors.push("Port must be at least 1024".into());
            }
        }
        if let Some(url) = next.public_url.as_ref().filter(|u| !u.trim().is_empty()) {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                errors.push("Public URL must start with http:// or https://".into());
            }
        }
        if let Some(fps) = next.capture_fps {
            if !(1..=30).contains(&fps) {
                errors.push("FPS must be between 1 and 30".into());
            }
        }
        if let Some(q) = next.jpeg_quality {
            if !(20..=95).contains(&q) {
                errors.push("Quality must be between 20 and 95".into());
            }
        }
        if let Some(res) = &next.resolution {
            if res != "720p" && res != "1080p" {
                errors.push("Resolution must be 720p or 1080p".into());
            }
        }
        if let Some(dir) = next.omp_cwd.as_ref().filter(|d| !d.trim().is_empty()) {
            let path = PathBuf::from(dir.trim());
            if !path.is_dir() {
                errors.push(format!("Project folder not found: {}", path.display()));
            }
        }
        if let Some(roots) = &next.allowed_roots {
            for root in roots.iter().filter(|r| !r.trim().is_empty()) {
                if !PathBuf::from(root.trim()).is_dir() {
                    errors.push(format!("Access folder not found: {root}"));
                }
            }
        }
        if let Some(days) = next.session_ttl_days {
            if !(1..=3650).contains(&days) {
                errors.push("Login duration must be 1–3650 days".into());
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(errors.join(" · "));
        }

        // `None` means "leave as is", so only compare fields the UI actually sent.
        let current = Self::read(data_dir);
        let changed = |a: Option<&String>, b: Option<&String>| match (a, b) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        let mut restart = Vec::new();
        if next.host.is_some() && next.host != current.host || next.port.is_some() && next.port != current.port {
            restart.push("host".to_string());
        }
        if changed(next.public_url.as_ref(), current.public_url.as_ref()) {
            restart.push("publicUrl".to_string());
        }
        if next.omp_cwd != current.omp_cwd
            || next.omp_bin != current.omp_bin
            || (next.allowed_roots.is_some() && next.allowed_roots != current.allowed_roots)
        {
            restart.push("agent".to_string());
        }
        if next.session_ttl_days.is_some() && next.session_ttl_days != current.session_ttl_days {
            restart.push("sessionTtlDays".to_string());
        }
        // The relay key changes the QR payload and which backend answers.
        if next.relay_key.is_some() && next.relay_key != current.relay_key {
            restart.push("relayKey".to_string());
        }
        if next.relay_url.is_some() && next.relay_url != current.relay_url {
            restart.push("relayUrl".to_string());
        }
        if next.require_pairing.is_some() && next.require_pairing != current.require_pairing {
            restart.push("requirePairing".to_string());
        }
        if next.capture_fps.is_some() && next.capture_fps != current.capture_fps
            || next.jpeg_quality.is_some() && next.jpeg_quality != current.jpeg_quality
            || next.resolution.is_some() && next.resolution != current.resolution
        {
            restart.push("capture".to_string());
        }

        // Merge into what is already stored: a partial POST must not wipe the
        // other fields. `None` means "not provided" everywhere in this struct.
        let mut merged = current;
        macro_rules! keep {
            ($field:ident) => {
                if next.$field.is_some() {
                    merged.$field = next.$field.clone();
                }
            };
        }
        keep!(device_name);
        keep!(host);
        keep!(port);
        keep!(public_url);
        keep!(capture_fps);
        keep!(jpeg_quality);
        keep!(resolution);
        keep!(omp_cwd);
        keep!(omp_bin);
        keep!(allowed_roots);
        keep!(session_ttl_days);
        keep!(require_pairing);
        keep!(relay_url);
        keep!(relay_key);

        fs::create_dir_all(data_dir)?;
        let path = Self::path(data_dir);
        let _ = fs::copy(&path, data_dir.join("settings.json.bak"));
        let tmp = data_dir.join("settings.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&merged)?)?;
        fs::rename(&tmp, &path)?;
        restrict(&path);
        Ok(restart)
    }
}

impl Config {
    /// Effective settings, safe to show in the setup UI.
    pub fn settings(&self) -> Settings {
        Settings {
            device_name: Some(self.device_name.clone()),
            host: Some(self.host.clone()),
            port: Some(self.port),
            public_url: self.public_url.clone(),
            capture_fps: Some(self.fps),
            jpeg_quality: Some(self.quality),
            resolution: Some(self.resolution.label().to_string()),
            omp_cwd: Some(self.cwd.display().to_string()),
            omp_bin: Some(self.omp_bin.display().to_string()),
            allowed_roots: Some(
                self.allowed_roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
            ),
            session_ttl_days: Some(self.session_ttl_days),
            require_pairing: Some(self.require_pairing),
            relay_url: std::env::var("RELAY_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| self.public_url.clone()),
            relay_key: self.relay_key.clone(),
        }
    }

    /// Apply `data/settings.json` on top of the environment-derived values.
    fn apply_settings(&mut self, data_dir: &std::path::Path) {
        let s = Settings::read(data_dir);
        if let Some(name) = s.device_name.filter(|n| !n.trim().is_empty()) {
            self.device_name = name.trim().to_string();
        }
        if let Some(host) = s.host.filter(|h| !h.trim().is_empty()) {
            self.host = host.trim().to_string();
        }
        if let Some(port) = s.port.filter(|p| *p >= 1024) {
            self.port = port;
        }
        if let Some(url) = s.public_url {
            let url = url.trim().trim_end_matches('/').to_string();
            self.public_url = if url.is_empty() { None } else { Some(url) };
        }
        if let Some(fps) = s.capture_fps {
            self.fps = fps.clamp(1, 30);
        }
        if let Some(q) = s.jpeg_quality {
            self.quality = q.clamp(20, 95);
        }
        if let Some(res) = s.resolution.as_deref() {
            self.resolution = match res {
                "1080p" => CaptureResolution::P1080,
                _ => CaptureResolution::P720,
            };
        }
        if let Some(dir) = s.omp_cwd.filter(|d| !d.trim().is_empty()) {
            let path = PathBuf::from(dir.trim());
            if path.is_dir() {
                self.cwd = std::fs::canonicalize(&path).unwrap_or(path);
            }
        }
        if let Some(bin) = s.omp_bin.filter(|b| !b.trim().is_empty()) {
            self.omp_bin = PathBuf::from(bin.trim());
        }
        if let Some(roots) = s.allowed_roots {
            let mut resolved: Vec<PathBuf> = roots
                .iter()
                .filter(|r| !r.trim().is_empty())
                .filter_map(|r| std::fs::canonicalize(r.trim()).ok())
                .collect();
            if !resolved.is_empty() {
                if !resolved.iter().any(|r| r == &self.home) {
                    resolved.push(self.home.clone());
                }
                self.allowed_roots = resolved;
            }
        }
        if let Some(days) = s.session_ttl_days {
            self.session_ttl_days = days.clamp(1, 3650);
        }
        if let Some(required) = s.require_pairing {
            self.require_pairing = required;
        }
        if let Some(key) = s.relay_key.as_ref().map(|k| k.trim()).filter(|k| !k.is_empty()) {
            self.relay_key = Some(key.to_string());
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CaptureResolution {
    P720,
    P1080,
}

impl CaptureResolution {
    pub fn as_scap(self) -> scap::capturer::Resolution {
        match self {
            Self::P720 => scap::capturer::Resolution::_720p,
            Self::P1080 => scap::capturer::Resolution::_1080p,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::P720 => "720p",
            Self::P1080 => "1080p",
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        load_dotenv();
        let data_dir = std::env::var("REMOTE_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_dir());
        fs::create_dir_all(&data_dir)
            .with_context(|| format!("create data dir {}", data_dir.display()))?;
        migrate_legacy_data(&data_dir);

        let token = match std::env::var("REMOTE_TOKEN") {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => load_or_create(&data_dir.join("token"), 24)?,
        };

        let cwd = std::env::var("OMP_CWD")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_workspace());
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);

        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .ok()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .unwrap_or_else(|| PathBuf::from("/"));

        // The file explorer always includes the user's home, so the ⌂ button
        // means "home of this machine", not "project root".
        let mut allowed_roots = match std::env::var("ALLOWED_ROOTS") {
            Ok(value) if !value.trim().is_empty() => value
                .split(':')
                .filter(|s| !s.trim().is_empty())
                .map(|s| PathBuf::from(s.trim()))
                .filter_map(|p| std::fs::canonicalize(p).ok())
                .collect::<Vec<_>>(),
            _ => vec![cwd.clone(), home.clone()],
        };
        if !allowed_roots.iter().any(|r| r == &home) {
            allowed_roots.push(home.clone());
        }

        let mut cfg = Self {
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8788),
            token,
            session: std::env::var("OMP_SESSION").unwrap_or_else(|_| "omp".into()),
            cwd,
            fps: std::env::var("CAPTURE_FPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5)
                .clamp(1, 30),
            quality: std::env::var("JPEG_QUALITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(62)
                .clamp(20, 95),
            resolution: match std::env::var("CAPTURE_RESOLUTION").as_deref() {
                Ok("1080p") => CaptureResolution::P1080,
                _ => CaptureResolution::P720,
            },
            projects_file: data_dir.join("projects.json"),
            allowed_roots,
            home,
            omp_bin: Self::resolve_omp_bin(),
            public_url: std::env::var("PUBLIC_URL").ok().filter(|v| !v.trim().is_empty()),
            session_ttl_days: std::env::var("SESSION_TTL_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(365)
                .clamp(1, 3650),
            relay_key: std::env::var("RELAY_KEY")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            require_pairing: std::env::var("REQUIRE_PAIRING")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(false),
            data_dir,
            device_id: String::new(), // filled by main after Identity::load
            device_name: std::env::var("DEVICE_NAME")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| {
                    std::process::Command::new("scutil")
                        .args(["--get", "ComputerName"])
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Remote Agents".into())
                }),
        };
        let data_dir = cfg.data_dir.clone();
        cfg.apply_settings(&data_dir);
        Ok(cfg)
    }

    pub fn secret(&self) -> Result<[u8; 32]> {
        let path = self.data_dir.join("secret");
        if let Ok(raw) = fs::read(&path) {
            if raw.len() == 32 {
                let mut out = [0u8; 32];
                out.copy_from_slice(&raw);
                return Ok(out);
            }
        }
        let mut out = [0u8; 32];
        rand::rng().fill_bytes(&mut out);
        fs::write(&path, out)?;
        restrict(&path);
        Ok(out)
    }

    pub fn lan_ip(&self) -> Option<IpAddr> {
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("8.8.8.8:80").ok()?;
        Some(socket.local_addr().ok()?.ip())
    }

    /// Where the bundled web UI lives. In a `.app`/`.exe` bundle the assets sit
    /// next to the executable; in a dev checkout they are in `./public`.
    pub fn public_dir() -> PathBuf {
        if let Ok(explicit) = std::env::var("REMOTE_PUBLIC") {
            let path = PathBuf::from(explicit);
            if path.is_dir() {
                return path;
            }
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("public"));
                candidates.push(dir.join("../Resources/public"));
            }
        }
        candidates.push(PathBuf::from("public"));
        candidates
            .into_iter()
            .find(|p| p.join("index.html").is_file())
            .unwrap_or_else(|| PathBuf::from("public"))
    }

    /// Find the omp CLI without asking the user. `OMP_BIN` wins; otherwise the
    /// common install paths are probed so a fresh `.dmg` install just works.
    pub fn resolve_omp_bin() -> PathBuf {
        if let Ok(explicit) = std::env::var("OMP_BIN") {
            let path = PathBuf::from(&explicit);
            if path.is_file() {
                return path;
            }
        }
        let mut candidates = vec![
            PathBuf::from("/opt/homebrew/bin/omp"),
            PathBuf::from("/usr/local/bin/omp"),
        ];
        if let Ok(home) = std::env::var("HOME") {
            for rel in [".local/bin/omp", ".bun/bin/omp", ".npm-global/bin/omp", ".volta/bin/omp"] {
                candidates.push(PathBuf::from(&home).join(rel));
            }
        }
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if dir.trim().is_empty() {
                    continue;
                }
                candidates.push(PathBuf::from(dir).join("omp"));
            }
        }
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("omp"))
    }

    /// Address a phone should use. Prefers the relay hostname, because a LAN IP
    /// is useless to a device outside the network.
    pub fn external_base(&self) -> String {
        let from_env = std::env::var("RELAY_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        if let Some(url) = from_env.or_else(|| self.public_url.clone()) {
            return url.trim().trim_end_matches('/').to_string();
        }
        match self.lan_ip() {
            Some(ip) => format!("http://{ip}:{}", self.port),
            None => format!("http://127.0.0.1:{}", self.port),
        }
    }

    /// Pairing link for a one-time code. `?k=` must come before the fragment so
    /// nginx can pick the right backend before the phone has any cookie.
    pub fn pair_link(&self, code: &str) -> String {
        let base = self.external_base();
        let id = if self.device_id.is_empty() { "" } else { &self.device_id };
        match self.relay_key.as_deref() {
            Some(key) => format!("{base}/pair?k={key}#c={code}&d={id}"),
            None => format!("{base}/pair#c={code}&d={id}"),
        }
    }

    pub fn base_url(&self) -> String {
        match self.lan_ip() {
            Some(ip) if self.host != "127.0.0.1" => format!("http://{ip}:{}", self.port),
            _ => format!("http://127.0.0.1:{}", self.port),
        }
    }
}

/// Older checkouts kept identity in `./data`. On the first run of a bundled
/// build, carry that identity over so the device ID (and its pairings) survive
/// the switch to Application Support.
fn migrate_legacy_data(data_dir: &std::path::Path) {
    if data_dir.join("id").exists() {
        return;
    }
    let legacy = PathBuf::from(
        std::env::var("REMOTE_LEGACY_DATA").unwrap_or_else(|_| "data".into()),
    );
    if !legacy.join("id").exists() || legacy == data_dir {
        return;
    }
    for name in ["id", "token", "secret", "clients.json", "settings.json", "projects.json"] {
        let from = legacy.join(name);
        if from.is_file() {
            let _ = fs::copy(&from, data_dir.join(name));
            restrict(&data_dir.join(name));
        }
    }
}

/// First-run friendly default: keep identity/settings next to the user's other
/// app data, so a `.dmg` install works without choosing a folder.
/// Workspace shown first: an existing Ai/Documents folder, else the user's home.
/// Never the app bundle, which would be read-only.
/// When launched from a `.app`, point the data dir, assets and relay at the
/// bundle/user locations before `Config::load` reads any variable. Explicit
/// environment variables always win.
pub fn apply_bundle_defaults() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(macos_dir) = exe.parent() else { return };
    // .../Name.app/Contents/MacOS/remot-agent
    if macos_dir.file_name().and_then(|n| n.to_str()) != Some("MacOS") {
        return;
    }
    let Some(contents) = macos_dir.parent() else { return };
    if contents.file_name().and_then(|n| n.to_str()) != Some("Contents") {
        return;
    }
    let resources = contents.join("Resources");
    let mut set = |key: &str, value: String| {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    };
    let public = resources.join("public");
    if public.is_dir() {
        set("REMOTE_PUBLIC", public.display().to_string());
    }
    set("REMOT_AGENT_DESKTOP", "1".into());
    let home = std::env::var("HOME").unwrap_or_default();
    set(
        "REMOTE_DATA",
        PathBuf::from(&home)
            .join("Library/Application Support/Remote Agents")
            .display()
            .to_string(),
    );
    if let Ok(url) = std::fs::read_to_string(resources.join("relay-url")) {
        let url = url.trim().to_string();
        if !url.is_empty() {
            set("RELAY_URL", url.clone());
            set("PUBLIC_URL", url);
        }
    }
    // Bundled builds always want stable settings; the UI owns them from here.
    set("PATH", format!("/opt/homebrew/bin:/usr/local/bin:{}", std::env::var("PATH").unwrap_or_default()));
}

fn default_workspace() -> PathBuf {
    // No guessing between Documents/Ai or Documents. Until the user picks a
    // folder in the setup UI, the workspace is the account home
    // (e.g. /Users/you).
    let home = std::env::var("HOME").unwrap_or_default();
    let path = PathBuf::from(&home);
    if path.is_dir() {
        return path;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if cfg!(target_os = "macos") {
        return PathBuf::from(home).join("Library/Application Support/Remote Agents");
    }
    if cfg!(target_os = "windows") {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("Remote Agents");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("remote-agents");
    }
    PathBuf::from(home).join(".local/share/remote-agents")
}

fn load_or_create(path: &PathBuf, bytes: usize) -> Result<String> {
    if let Ok(existing) = fs::read_to_string(path) {
        let trimmed = existing.trim();
        if trimmed.len() >= 16 {
            return Ok(trimmed.to_string());
        }
    }
    let mut raw = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut raw);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    fs::write(path, &token)?;
    restrict(path);
    Ok(token)
}

/// Minimal .env loader: KEY=VALUE, # comments, optional quotes. Never overrides
/// variables already present in the real environment.
fn load_dotenv() {
    let path = std::env::var("REMOTE_ENV_FILE").unwrap_or_else(|_| ".env".into());
    let Ok(raw) = fs::read_to_string(&path) else { return };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() || std::env::var_os(key).is_some() {
            continue;
        }
        let mut value = value.trim().to_string();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = value[1..value.len() - 1].to_string();
        }
        let expanded = value.replace("$HOME", &std::env::var("HOME").unwrap_or_default());
        std::env::set_var(key, expanded);
    }
}

fn restrict(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

/// PATH as the user's login shell sees it. The app is started by launchd,
/// which gives it `/usr/bin:/bin:/usr/sbin:/sbin` — so anything installed with
/// Homebrew, nvm, bun, or a package manager (`npx`, `adb`, `emulator`) is
/// invisible to child processes unless we borrow it from the shell.
pub fn shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let out = std::process::Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() || !path.contains('/') {
        return None;
    }
    Some(path)
}
