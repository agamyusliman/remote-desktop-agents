use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{broadcast, mpsc, oneshot, RwLock},
};
use tracing::{error, info, warn};

#[derive(Clone, Debug, Default, Serialize)]
pub struct RpcState {
    pub model: Option<Value>,
    pub session_id: Option<String>,
    pub session_file: Option<String>,
    pub session_name: Option<String>,
    pub thinking: Option<Value>,
    pub context: Option<Value>,
    pub todos: Option<Value>,
    pub queued: Option<Value>,
    pub busy: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub path: String,
    pub timestamp: String,
    pub mtime: u64,
}

struct Chunk {
    id: String,
    count: usize,
    parts: Vec<Option<Vec<u8>>>,
}

pub struct RpcProcess {
    pub project_id: String,
    pub events: broadcast::Sender<Value>,
    pub state: Arc<RwLock<RpcState>>,
    stdin: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    _child: Arc<Mutex<Option<Child>>>,
}

impl RpcProcess {
    pub async fn start(
        omp_bin: &PathBuf,
        project_id: &str,
        project_path: &PathBuf,
        resume: Option<&str>,
    ) -> Result<Arc<Self>> {
        let mut command = Command::new(omp_bin);
        if let Some(path) = crate::config::shell_path() {
            command.env("PATH", path);
        }
        command
            .arg("--mode")
            .arg("rpc-ui")
            .arg("--cwd")
            .arg(project_path);
        // Resuming must happen at spawn time. Sending `switch_session` to a
        // freshly spawned process creates a new session file first, which is
        // what produced a dozen duplicate entries in the picker.
        if let Some(path) = resume.filter(|p| !p.trim().is_empty()) {
            command.arg("--resume").arg(path);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("PI_SESSION_ID")
            .env_remove("PI_SESSION_FILE")
            .env_remove("PI_TERMINAL_OWNER_PID")
            .env_remove("PI_TOOL_CALL_ID")
            .env_remove("OMPCODE")
            .env("TERM", "xterm-256color");

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn {}", omp_bin.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

        let (events, _) = broadcast::channel(512);
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(RwLock::new(RpcState::default()));
        let alive = Arc::new(AtomicBool::new(true));

        let process = Arc::new(Self {
            project_id: project_id.to_string(),
            events: events.clone(),
            state: state.clone(),
            stdin: in_tx,
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
            alive: alive.clone(),
            _child: Arc::new(Mutex::new(Some(child))),
        });

        // stdin writer
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = in_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        // reader / frame reassembly
        let reader_events = events.clone();
        let reader_pending = pending.clone();
        let reader_state = state.clone();
        let reader_alive = alive.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut chunk: Option<Chunk> = None;
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let value = if value.get("type").and_then(Value::as_str) == Some("rpc_chunk") {
                    match handle_chunk(&mut chunk, &value) {
                        Ok(Some(full)) => full,
                        Ok(None) => continue,
                        Err(err) => {
                            warn!("rpc chunk error: {err}");
                            chunk = None;
                            continue;
                        }
                    }
                } else {
                    value
                };
                dispatch(&value, &reader_pending, &reader_state).await;
                let _ = reader_events.send(value);
            }
            reader_alive.store(false, Ordering::Relaxed);
            let _ = reader_events.send(json!({"type":"rpc_exit"}));
        });

        let stderr_alive = alive.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("error") || line.contains("ERROR") {
                    warn!("omp rpc: {line}");
                }
            }
            stderr_alive.store(false, Ordering::Relaxed);
        });

        // Initial handshake + state.
        let init = process.clone();
        tokio::spawn(async move {
            let _ = init
                .request(json!({"type":"negotiate_protocol","protocolVersion":2}), Duration::from_secs(10))
                .await;
            let _ = init
                .request(json!({"type":"set_ask_dialog","enabled":true}), Duration::from_secs(10))
                .await;
            let _ = init
                .request(
                    json!({"type":"set_event_filter","events":null,"messageUpdates":"full"}),
                    Duration::from_secs(10),
                )
                .await;
            let _ = init
                .request(json!({"type":"get_state"}), Duration::from_secs(20))
                .await;
            info!("rpc ready for project {}", init.project_id);
        });

        Ok(process)
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn send_command(&self, mut command: Value) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Some(obj) = command.as_object_mut() {
            obj.insert("id".into(), json!(format!("rpc-{id}")));
        }
        let _ = self.stdin.send(command.to_string());
    }

    pub async fn request(&self, mut command: Value, timeout: Duration) -> Result<Value> {
        let id = format!("rpc-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        if let Some(obj) = command.as_object_mut() {
            obj.insert("id".into(), json!(id));
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let _ = self.stdin.send(command.to_string());
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(anyhow!("rpc response dropped")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(anyhow!("rpc request timeout"))
            }
        }
    }

    pub fn snapshot(&self) -> RpcState {
        self.state.try_read().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for RpcProcess {
    fn drop(&mut self) {
        if let Ok(mut child) = self._child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

async fn dispatch(
    value: &Value,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    state: &Arc<RwLock<RpcState>>,
) {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or_default();
    if kind == "response" {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            if let Some(tx) = pending.lock().unwrap().remove(id) {
                let _ = tx.send(value.clone());
            }
        }
        let command = value.get("command").and_then(Value::as_str).unwrap_or_default();
        let success = value.get("success").and_then(Value::as_bool).unwrap_or(false);
        if success {
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            let mut state = state.write().await;
            match command {
                "get_state" => {
                    if let Some(model) = data.get("model") {
                        state.model = Some(model.clone());
                    }
                    state.session_id = data.get("sessionId").and_then(Value::as_str).map(str::to_string);
                    state.session_file = data.get("sessionFile").and_then(Value::as_str).map(str::to_string);
                    state.session_name = data.get("sessionName").and_then(Value::as_str).map(str::to_string);
                    state.thinking = data.get("thinkingLevel").cloned();
                    state.context = data.get("contextUsage").cloned();
                    state.todos = data.get("todoPhases").cloned();
                    state.queued = data.get("queuedMessages").cloned();
                    state.busy = data.get("isStreaming").and_then(Value::as_bool).unwrap_or(false);
                }
                "set_model" => {
                    state.model = Some(data);
                }
                "new_session" | "switch_session" => {
                    if let Some(session) = data.get("sessionId").and_then(Value::as_str) {
                        state.session_id = Some(session.to_string());
                    }
                }
                _ => {}
            }
        }
        return;
    }

    let mut state = state.write().await;
    match kind {
        "agent_start" => state.busy = true,
        "agent_end" | "prompt_result" | "session_settled" => state.busy = false,
        "session_info_update" => {
            if let Some(name) = value.get("sessionName").and_then(Value::as_str) {
                state.session_name = Some(name.to_string());
            }
        }
        _ => {}
    }
}

fn handle_chunk(current: &mut Option<Chunk>, value: &Value) -> Result<Option<Value>> {
    let id = value
        .get("chunkId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing chunkId"))?
        .to_string();
    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let data = value
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing chunk data"))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(data))?;

    if current.as_ref().map(|c| c.id != id || c.count != count).unwrap_or(true) {
        *current = Some(Chunk {
            id: id.clone(),
            count,
            parts: (0..count).map(|_| None).collect(),
        });
    }
    let chunk = current.as_mut().unwrap();
    if index >= chunk.parts.len() {
        return Err(anyhow!("chunk index out of range"));
    }
    chunk.parts[index] = Some(decoded);
    if chunk.parts.iter().all(|p| p.is_some()) {
        let mut bytes = Vec::new();
        for part in chunk.parts.iter_mut() {
            bytes.extend_from_slice(part.take().unwrap().as_slice());
        }
        let value = serde_json::from_slice(&bytes).context("decode reassembled rpc frame")?;
        *current = None;
        return Ok(Some(value));
    }
    Ok(None)
}

pub struct RpcHub {
    omp_bin: PathBuf,
    map: Mutex<HashMap<String, Arc<RpcProcess>>>,
}

impl RpcHub {
    pub fn new(omp_bin: PathBuf) -> Self {
        Self {
            omp_bin,
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, project_id: &str, path: &PathBuf) -> Result<Arc<RpcProcess>> {
        self.get_with_resume(project_id, path, None).await
    }

    /// Reuse a live process for this project; otherwise spawn one, optionally
    /// resuming an existing session so no duplicate is created.
    pub async fn get_with_resume(
        &self,
        project_id: &str,
        path: &PathBuf,
        resume: Option<&str>,
    ) -> Result<Arc<RpcProcess>> {
        if let Some(process) = self.map.lock().unwrap().get(project_id) {
            if process.alive() {
                return Ok(process.clone());
            }
        }
        let process = RpcProcess::start(&self.omp_bin, project_id, path, resume).await?;
        self.map
            .lock()
            .unwrap()
            .insert(project_id.to_string(), process.clone());
        Ok(process)
    }
}

pub fn session_dir(project_path: &PathBuf) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = PathBuf::from(&home);
    let rel = project_path
        .strip_prefix(&home_path)
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_path.clone());
    let rel_str = rel.to_string_lossy();
    let encoded = if rel_str.starts_with('/') {
        rel_str.replace('/', "-")
    } else {
        format!("-{}", rel_str.replace('/', "-"))
    };
    home_path.join(".omp/agent/sessions").join(encoded)
}

pub fn list_sessions(project_path: &PathBuf) -> Vec<SessionInfo> {
    let dir = session_dir(project_path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut title = String::new();
        let mut id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let mut timestamp = String::new();
        if let Ok(file) = std::fs::File::open(&path) {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(file).lines().take(6) {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                match value.get("type").and_then(Value::as_str) {
                    Some("title") => {
                        if let Some(t) = value.get("title").and_then(Value::as_str) {
                            title = t.to_string();
                        }
                    }
                    Some("session") => {
                        if let Some(i) = value.get("id").and_then(Value::as_str) {
                            id = i.to_string();
                        }
                        if let Some(t) = value.get("timestamp").and_then(Value::as_str) {
                            timestamp = t.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        if title.is_empty() {
            title = timestamp.clone();
        }
        if title.is_empty() {
            title = id.clone();
        }
        sessions.push(SessionInfo {
            id,
            title,
            path: path.display().to_string(),
            timestamp,
            mtime,
        });
    }
    sessions.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    sessions
}

pub async fn available_models(process: &RpcProcess) -> Result<Vec<Value>> {
    let response = process
        .request(json!({"type":"get_available_models"}), Duration::from_secs(45))
        .await?;
    if !response.get("success").and_then(Value::as_bool).unwrap_or(false) {
        error!("get_available_models failed: {response}");
        return Ok(Vec::new());
    }
    let models = response
        .pointer("/data/models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Value> = models
        .into_iter()
        .map(|m| {
            json!({
                "provider": m.get("provider").cloned().unwrap_or(Value::Null),
                "id": m.get("id").cloned().unwrap_or(Value::Null),
                "name": m.get("name").cloned().unwrap_or(Value::Null),
                "contextWindow": m.get("contextWindow").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        let pa = a.get("provider").and_then(Value::as_str).unwrap_or("");
        let pb = b.get("provider").and_then(Value::as_str).unwrap_or("");
        let na = a.get("name").and_then(Value::as_str).unwrap_or("");
        let nb = b.get("name").and_then(Value::as_str).unwrap_or("");
        pa.cmp(pb).then(na.cmp(nb))
    });
    Ok(out)
}
