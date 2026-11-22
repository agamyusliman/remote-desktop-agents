//! Device pairing: a short, single-use code shown on the desktop. The phone
//! exchanges it for a random client key. The key lives in the phone's own
//! storage and in `data/clients.json` on this Mac — the relay never receives or
//! stores either the code's secret or the resulting credential.

use anyhow::Result;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::{Instant, SystemTime, UNIX_EPOCH}};

/// Unambiguous alphabet (no O/0, I/1/L).
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 6;
const CODE_TTL: u64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub name: String,
    #[serde(rename = "key")]
    pub key: String,
    pub created: u64,
    #[serde(default)]
    pub last_seen: u64,
    #[serde(default)]
    pub platform: String,
}

#[derive(Clone, Debug)]
struct PendingCode {
    code: String,
    expires: Instant,
}

pub struct Pairing {
    store: PathBuf,
    clients: Mutex<Vec<Client>>,
    pending: Mutex<Option<PendingCode>>,
    /// Keys handed out by the mobile app that this Mac has not approved yet.
    approvals: Mutex<Vec<PendingApproval>>,
    session_last_seen: Mutex<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingApproval {
    pub key: String,
    pub name: String,
    pub purpose: String,
    pub created: u64,
}

#[derive(Deserialize)]
pub struct ApproveRequest {
    pub key: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize)]
pub struct CodeResponse {
    pub code: String,
    pub expires_in: u64,
    pub id: String,
}

#[derive(Deserialize)]
pub struct ClaimRequest {
    pub code: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
}

#[derive(Serialize)]
pub struct ClaimResponse {
    pub ok: bool,
    pub client_key: String,
    pub client_id: String,
    pub device_id: String,
    pub device_name: String,
}

impl Pairing {
    pub fn load(data_dir: &PathBuf) -> Self {
        let store = data_dir.join("clients.json");
        let clients = std::fs::read_to_string(&store)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<Client>>(&raw).ok())
            .unwrap_or_default();
        Self {
            store,
            clients: Mutex::new(clients),
            pending: Mutex::new(None),
            approvals: Mutex::new(Vec::new()),
            session_last_seen: Mutex::new(0),
        }
    }

    /// Activity from a credential that is not a paired client (an older
    /// `ra_session` cookie). Recorded so the desktop can still show that a
    /// device is connected.
    pub fn note_session(&self) {
        *self.session_last_seen.lock().unwrap() = now();
    }

    pub fn session_presence_secs(&self) -> Option<u64> {
        let seen = *self.session_last_seen.lock().unwrap();
        if seen == 0 {
            None
        } else {
            Some(now().saturating_sub(seen))
        }
    }

    /// Forget presence so the desktop stops showing a live session.
    pub fn clear_presence(&self) {
        *self.session_last_seen.lock().unwrap() = 0;
    }

    pub fn list(&self) -> Vec<Client> {
        self.clients.lock().unwrap().clone()
    }

    /// Clients active within the last 45 seconds count as connected.
    pub fn list_with_presence(&self) -> Vec<serde_json::Value> {
        let now = now();
        self.clients
            .lock()
            .unwrap()
            .iter()
            .map(|c| {
                let online = now.saturating_sub(c.last_seen) < 45;
                serde_json::json!({
                    "id": c.id,
                    "name": c.name,
                    "platform": c.platform,
                    "created": c.created,
                    "last_seen": c.last_seen,
                    "online": online,
                })
            })
            .collect()
    }

    pub fn is_known(&self, key: &str) -> bool {
        if key.trim().is_empty() {
            return false;
        }
        self.clients.lock().unwrap().iter().any(|c| c.key == key)
    }

    pub fn revoke(&self, id: &str) -> Result<bool> {
        let mut clients = self.clients.lock().unwrap();
        let before = clients.len();
        clients.retain(|c| c.id != id);
        let removed = clients.len() != before;
        if removed {
            self.persist(&clients)?;
        }
        Ok(removed)
    }

    pub fn revoke_all(&self) -> Result<()> {
        let mut clients = self.clients.lock().unwrap();
        clients.clear();
        self.persist(&clients)
    }

    pub fn touch(&self, key: &str) {
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.iter_mut().find(|c| c.key == key) {
            client.last_seen = now();
            let _ = self.persist(&clients);
        }
    }

    /// Issue a fresh single-use code. Only one code is valid at a time.
    pub fn issue(&self) -> CodeResponse {
        let mut raw = [0u8; CODE_LEN];
        rand::rng().fill_bytes(&mut raw);
        let code: String = raw
            .iter()
            .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
            .collect();
        let display = format!("{}-{}", &code[..3], &code[3..]);
        *self.pending.lock().unwrap() = Some(PendingCode {
            code: code.clone(),
            expires: Instant::now() + std::time::Duration::from_secs(CODE_TTL),
        });
        CodeResponse {
            code: display,
            expires_in: CODE_TTL,
            id: self.pending_id(),
        }
    }

    fn pending_id(&self) -> String {
        // Non-secret fingerprint so the UI can poll its own code state.
        self.pending
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.code.chars().rev().collect::<String>())
            .unwrap_or_default()
    }

    pub fn pending_code(&self) -> Option<String> {
        let pending = self.pending.lock().unwrap();
        pending.as_ref().and_then(|p| {
            if p.expires > Instant::now() {
                Some(p.code.clone())
            } else {
                None
            }
        })
    }

    pub fn pending_expires_in(&self) -> u64 {
        let pending = self.pending.lock().unwrap();
        pending
            .as_ref()
            .map(|p| p.expires.saturating_duration_since(Instant::now()).as_secs())
            .unwrap_or(0)
    }

    /// Consume the pending code and mint a client key. Returns None for a bad or
    /// expired code so the caller can rate-limit without leaking which it was.
    pub fn claim(&self, req: &ClaimRequest, device_id: &str, device_name: &str) -> Option<ClaimResponse> {
        let submitted: String = req
            .code
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        {
            let mut pending = self.pending.lock().unwrap();
            let Some(active) = pending.as_ref() else { return None };
            if active.expires <= Instant::now() || active.code != submitted {
                return None;
            }
            *pending = None;
        }

        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let id = format!("c{}", &key[..8]);
        let name = if req.name.trim().is_empty() {
            "Device".to_string()
        } else {
            req.name.trim().chars().take(40).collect()
        };
        let client = Client {
            id: id.clone(),
            name,
            key: key.clone(),
            created: now(),
            last_seen: now(),
            platform: req.platform.trim().chars().take(20).collect(),
        };
        {
            let mut clients = self.clients.lock().unwrap();
            clients.push(client);
            let _ = self.persist(&clients);
        }
        Some(ClaimResponse {
            ok: true,
            client_key: key,
            client_id: id,
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
        })
    }

    /// Mobile-initiated pairing: the phone mints a key and asks the Mac to
    /// approve it. Nothing works until `approve()` is called here.
    pub fn request(&self, req: &ApproveRequest, device_id: &str, device_name: &str) -> ClaimResponse {
        let key = req.key.trim().to_string();
        {
            let mut approvals = self.approvals.lock().unwrap();
            approvals.retain(|a| a.key != key);
            approvals.push(PendingApproval {
                key: key.clone(),
                name: if req.name.trim().is_empty() {
                    "New device".into()
                } else {
                    req.name.trim().chars().take(40).collect()
                },
                purpose: if req.purpose.trim().is_empty() {
                    "login".into()
                } else {
                    req.purpose.trim().chars().take(20).collect()
                },
                created: now(),
            });
        }
        ClaimResponse {
            ok: true,
            client_key: key,
            client_id: String::new(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
        }
    }

    pub fn pending_approvals(&self) -> Vec<PendingApproval> {
        self.approvals.lock().unwrap().clone()
    }

    /// Approve a pending key. Returns true when it was pending (or already known).
    pub fn approve(&self, key: &str) -> bool {
        let key = key.trim().to_string();
        let pending = {
            let mut approvals = self.approvals.lock().unwrap();
            match approvals.iter().position(|a| a.key == key) {
                Some(idx) => Some(approvals.remove(idx)),
                None => None,
            }
        };
        let Some(entry) = pending else {
            return self.is_known(&key);
        };
        let id = format!("c{}", &key.chars().take(8).collect::<String>());
        let mut clients = self.clients.lock().unwrap();
        if !clients.iter().any(|c| c.key == key) {
            clients.push(Client {
                id,
                name: entry.name,
                key,
                created: now(),
                last_seen: now(),
                platform: entry.purpose,
            });
            let _ = self.persist(&clients);
        }
        true
    }

    fn persist(&self, clients: &[Client]) -> Result<()> {
        if let Some(parent) = self.store.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.store.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(clients)?)?;
        std::fs::rename(&tmp, &self.store)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.store, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Small helper for the setup UI: recent clients keyed by id.
pub fn summarize(clients: &[Client]) -> HashMap<String, String> {
    clients
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect()
}
