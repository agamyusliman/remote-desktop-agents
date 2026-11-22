use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

const COOKIE_PREFIX: &str = "ra_session_";
const WINDOW: Duration = Duration::from_secs(5 * 60);
const MAX_ATTEMPTS: u32 = 10;

type HmacSha256 = Hmac<Sha256>;

pub struct Auth {
    public_id: String,
    token: String,
    secret: [u8; 32],
    ttl_secs: u64,
    /// When false, the public 9-digit ID is a display label only and cannot be
    /// used to log in. Pairing or the long token is then the only way in.
    allow_id_login: bool,
    attempts: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

/// Legacy cookie name for a pairing credential. New cookies are namespaced per
/// device (see `client_cookie_name`) because one relay hostname fronts several
/// Macs, and each Mac must only accept its own keys.
pub const CLIENT_COOKIE: &str = "ra_client";

impl Auth {
    pub fn new(
        public_id: String,
        token: String,
        secret: [u8; 32],
        ttl_days: u64,
        allow_id_login: bool,
    ) -> Self {
        Self {
            public_id,
            token,
            secret,
            allow_id_login,
            ttl_secs: ttl_days.clamp(1, 3650) * 24 * 3600,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl_days(&self) -> u64 {
        self.ttl_secs / (24 * 3600)
    }

    pub fn public_id(&self) -> &str {
        &self.public_id
    }

    /// The 9-digit device ID or the long secret both grant access.
    pub fn check(&self, candidate: &str) -> bool {
        self.token_matches(candidate)
    }

    pub fn token_matches(&self, candidate: &str) -> bool {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            return false;
        }
        if self.allow_id_login && candidate == self.public_id {
            return true;
        }
        let a = sha2::Sha256::digest(self.token.as_bytes());
        let b = sha2::Sha256::digest(candidate.as_bytes());
        a.as_slice().ct_eq(b.as_slice()).into()
    }

    pub fn cookie(&self) -> (String, u64) {
        let exp = now() + self.ttl_secs;
        let sig = self.sign(exp);
        (format!("{exp}.{sig}"), self.ttl_secs)
    }

    /// One cookie per device, so several devices behind one relay hostname can
    /// keep independent sessions in the same browser.
    pub fn cookie_name(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.public_id)
    }

    /// Relay mode: one hostname fronts several devices, but each device keeps
    /// its own session cookie so switching devices does not force a re-login.
    /// The bare `ra_session` name is still accepted for older sessions.
    pub fn relay_cookie_name(&self) -> String {
        self.cookie_name()
    }

    /// Cookie name for a paired client on this device: `ra_c_<device>_<prefix>`.
    /// Namespacing by device keeps a browser's keys for two Macs apart when both
    /// are served from the same relay hostname.
    pub fn client_cookie_name(device_id: &str, key: &str) -> String {
        format!(
            "ra_c_{}_{}",
            device_id,
            &key.chars().take(10).collect::<String>()
        )
    }

    pub fn check_headers(&self, headers: &http::HeaderMap) -> bool {
        let Some(raw) = headers.get(http::header::COOKIE).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let device_name = self.cookie_name();
        for part in raw.split(";") {
            let mut kv = part.trim().splitn(2, '=');
            match kv.next() {
                Some(name) if name == device_name || name == "ra_session" => {
                    if let Some(value) = kv.next() {
                        return self.verify(value);
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// A paired device carries the client key in a cookie. The key is a bearer
    /// credential stored only on that device and in this Mac's data dir.
    /// Every client key this browser sent for this device. A browser keeps one
    /// cookie per pairing, so a stale cookie can shadow a live one: callers
    /// must check them all instead of trusting the first.
    pub fn client_keys_from_headers(&self, headers: &http::HeaderMap) -> Vec<String> {
        let prefix = format!("ra_c_{}_", self.public_id);
        let Some(raw) = headers.get(http::header::COOKIE).and_then(|v| v.to_str().ok()) else {
            return Vec::new();
        };
        let mut keys = Vec::new();
        for part in raw.split(';') {
            let mut kv = part.trim().splitn(2, '=');
            let Some(name) = kv.next() else { continue };
            let Some(value) = kv.next() else { continue };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if name == CLIENT_COOKIE || name.starts_with(&prefix) {
                keys.push(value.to_string());
            }
        }
        keys
    }

    pub fn client_key_from_headers<'a>(&self, headers: &'a http::HeaderMap) -> Option<String> {
        let prefix = format!("ra_c_{}_", self.public_id);
        let raw = headers.get(http::header::COOKIE)?.to_str().ok()?;
        for part in raw.split(';') {
            let mut kv = part.trim().splitn(2, '=');
            let Some(name) = kv.next() else { continue };
            let Some(value) = kv.next() else { continue };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if name == CLIENT_COOKIE || name.starts_with(&prefix) {
                return Some(value.to_string());
            }
        }
        None
    }

    pub fn allow_attempt(&self, ip: IpAddr) -> bool {
        let mut map = self.attempts.lock().unwrap();
        let entry = map.entry(ip).or_insert((0, Instant::now()));
        if entry.1.elapsed() > WINDOW {
            *entry = (1, Instant::now());
            return true;
        }
        if entry.0 >= MAX_ATTEMPTS {
            return false;
        }
        entry.0 += 1;
        true
    }

    pub fn reset_attempts(&self, ip: IpAddr) {
        self.attempts.lock().unwrap().remove(&ip);
    }

    fn verify(&self, value: &str) -> bool {
        let mut parts = value.splitn(2, '.');
        let (Some(exp_raw), Some(sig_hex)) = (parts.next(), parts.next()) else {
            return false;
        };
        let Ok(exp) = exp_raw.parse::<u64>() else {
            return false;
        };
        if exp < now() || exp > now() + self.ttl_secs + 60 {
            return false;
        }
        let Ok(sig) = hex::decode(sig_hex) else {
            return false;
        };
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("hmac key");
        mac.update(exp_raw.as_bytes());
        mac.verify_slice(&sig).is_ok()
    }

    fn sign(&self, exp: u64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("hmac key");
        mac.update(exp.to_string().as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}



pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn request_is_https(headers: &http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim() == "https")
        .unwrap_or(false)
}


/// Replace data/token with a fresh random token and delete the HMAC secret so
/// every existing cookie is rejected on the next restart or request.
pub fn rotate(data_dir: &std::path::Path) -> std::io::Result<String> {
    use base64::Engine;
    use rand::RngCore;
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join("token"), &token)?;
    let _ = std::fs::remove_file(data_dir.join("secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(data_dir.join("token"), std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}
