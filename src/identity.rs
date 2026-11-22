//! Short, human-friendly device ID (like AnyDesk/TeamViewer) plus a long secret.
//! Both are accepted for login; the ID is what the QR encodes.

use anyhow::Result;
use base64::Engine;
use rand::RngCore;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Identity {
    /// 9-digit public ID, stable across restarts.
    pub id: String,
    /// Long secret, accepted as an alternative to the ID.
    pub token: String,
}

impl Identity {
    pub fn load(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let id = load_or_create_id(&data_dir.join("id"))?;
        let token = match std::env::var("REMOTE_TOKEN") {
            Ok(t) if t.len() >= 16 => t,
            _ => load_or_create_token(&data_dir.join("token"))?,
        };
        Ok(Self { id, token })
    }
}

fn load_or_create_id(path: &Path) -> Result<String> {
    if let Ok(raw) = std::fs::read_to_string(path) {
        let id = raw.trim();
        if id.len() == 9 && id.chars().all(|c| c.is_ascii_digit()) {
            return Ok(id.to_string());
        }
    }
    let mut buf = [0u8; 8];
    rand::rng().fill_bytes(&mut buf);
    let mut n = u64::from_le_bytes(buf) % 1_000_000_000;
    if n < 100_000_000 {
        n += 100_000_000;
    }
    let id = format!("{n:09}");
    std::fs::write(path, &id)?;
    restrict(path);
    Ok(id)
}

fn load_or_create_token(path: &Path) -> Result<String> {
    if let Ok(raw) = std::fs::read_to_string(path) {
        let token = raw.trim();
        if token.len() >= 16 {
            return Ok(token.to_string());
        }
    }
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    std::fs::write(path, &token)?;
    restrict(path);
    Ok(token)
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}
