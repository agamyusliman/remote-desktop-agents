
use axum::extract::ws::{Message, WebSocket};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use tracing::{error, info};


/// tmux session name for this target. A plain start reuses the project session
/// so reconnects reattach; a resume gets its own session, otherwise `-A` would
/// attach to the already-running omp and ignore `--resume`.
fn session_name(base: &str, resume: Option<&str>) -> String {
    use std::hash::{Hash, Hasher};
    let Some(path) = resume.filter(|p| !p.trim().is_empty()) else {
        return base.to_string();
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{base}-r{:08x}", (hasher.finish() & 0xffff_ffff) as u32)
}

pub const DEFAULT_COLS: u16 = 100;
pub const DEFAULT_ROWS: u16 = 28;

#[derive(Clone, Debug)]
pub struct Target {
    pub session: String,
    pub cwd: PathBuf,
    pub omp: PathBuf,
    /// When set, omp resumes this session file instead of starting a new one.
    pub resume: Option<String>,
    /// Attach to this tmux session instead of creating a project-scoped one.
    /// This is what lets the phone drive the CLI that is already running on
    /// the Mac, so the agent keeps one context and its tools stay visible.
    pub attach: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TmuxSession {
    pub name: String,
    pub cwd: String,
    pub command: String,
}

/// Sessions currently running on this Mac. The UI lists them so the phone can
/// join a terminal the user already has open instead of spawning a stranger.
pub fn list_tmux_sessions() -> Vec<TmuxSession> {
    let Ok(out) = std::process::Command::new(tmux_bin())
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{pane_current_path}\t#{pane_current_command}",
        ])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(TmuxSession {
                name: name.to_string(),
                cwd: parts.next().unwrap_or("").to_string(),
                command: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// True only for a session that currently exists, so a crafted `session` query
/// cannot point tmux at something else.
pub fn session_exists(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return false;
    }
    list_tmux_sessions().iter().any(|s| s.name == name)
}

pub async fn run(mut socket: WebSocket, target: Target, cols: u16, rows: u16) {
    let tmux = tmux_bin();
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: rows.max(2),
        cols: cols.max(2),
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = socket
                .send(Message::Text(format!("failed to open pty: {err}\r\n").into()))
                .await;
            return;
        }
    };

    let omp = if target.omp.exists() {
        target.omp.clone()
    } else {
        PathBuf::from("omp")
    };
    let session = match target.attach.as_deref() {
        Some(name) if session_exists(name) => name.to_string(),
        _ => session_name(&target.session, target.resume.as_deref()),
    };
    let mut cmd = CommandBuilder::new(tmux.as_os_str());
    cmd.arg("new-session");
    cmd.arg("-A");
    // A tmux server started by launchd inherits a bare PATH. Pass the login
    // shell's PATH into the session so tools the agent shells out to resolve.
    if let Some(path) = crate::config::shell_path() {
        cmd.arg("-e");
        cmd.arg(format!("PATH={path}"));
    }
    // Attach when this exact target already runs, otherwise create it.
    cmd.arg("-s");
    cmd.arg(session.as_str());
    cmd.arg("-c");
    cmd.arg(target.cwd.as_os_str());
    // Everything after the command belongs to omp. tmux takes flags of its
    // own before it, which is why `--resume` used to fail as an invalid flag.
    cmd.arg(omp.as_os_str());
    if let Some(path) = target.resume.as_deref().filter(|p| !p.trim().is_empty()) {
        cmd.arg("--resume");
        cmd.arg(path);
    }
    cmd.cwd(&target.cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env_remove("TMUX");
    cmd.env_remove("TMUX_PANE");

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(child) => child,
        Err(err) => {
            let _ = socket
                .send(Message::Text(format!("failed to start tmux: {err}\r\n").into()))
                .await;
            return;
        }
    };
    drop(pair.slave);

    let master = pair.master;
    let mut reader = match master.try_clone_reader() {
        Ok(reader) => reader,
        Err(err) => {
            let _ = child.kill();
            let _ = socket
                .send(Message::Text(format!("pty reader error: {err}\r\n").into()))
                .await;
            return;
        }
    };
    let writer = match master.take_writer() {
        Ok(writer) => writer,
        Err(err) => {
            let _ = child.kill();
            let _ = socket
                .send(Message::Text(format!("pty writer error: {err}\r\n").into()))
                .await;
            return;
        }
    };

    info!(
        "terminal attached to tmux session '{}' in {}",
        session,
        target.cwd.display()
    );
    let writer = Arc::new(Mutex::new(writer));
    let master = Arc::new(Mutex::new(master));
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(text.as_str()) else {
                            continue;
                        };
                        match value.get("type").and_then(|v| v.as_str()) {
                            Some("input") => {
                                if let Some(data) = value.get("data").and_then(|v| v.as_str()) {
                                    let mut guard = writer.lock().unwrap();
                                    if guard.write_all(data.as_bytes()).is_err() { break; }
                                    let _ = guard.flush();
                                }
                            }
                            Some("scroll") => {
                                // Scrollback belongs to tmux, not to omp: drive
                                // copy-mode directly instead of synthesising
                                // prefix keys through the pty (which tmux
                                // ignores when the client is already attached).
                                let dir = value.get("dir").and_then(|v| v.as_str()).unwrap_or("up");
                                let lines = value.get("lines").and_then(|v| v.as_i64()).unwrap_or(10);
                                scroll_session(&session, dir, lines);
                            }
                            Some("resize") => {
                                let cols = value.get("cols").and_then(|v| v.as_u64()).unwrap_or(cols as u64) as u16;
                                let rows = value.get("rows").and_then(|v| v.as_u64()).unwrap_or(rows as u64) as u16;
                                let _ = master.lock().unwrap().resize(PtySize {
                                    rows: rows.max(2),
                                    cols: cols.max(2),
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        let mut guard = writer.lock().unwrap();
                        if guard.write_all(&data).is_err() { break; }
                        let _ = guard.flush();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(err)) => {
                        error!("terminal socket: {err}");
                        break;
                    }
                    _ => {}
                }
            }
            Some(data) = rx.recv() => {
                if socket.send(Message::Binary(data.into())).await.is_err() {
                    break;
                }
            }
            else => break,
        }
    }

    // Kill only THIS client. `detach-client -s <session>` detaches every client
    // attached to the session, so one stale phone connection closing would rip
    // the live viewer off the same tmux session.
    let _ = child.kill();
    let _ = child.wait();
}

/// Move the tmux viewport. `dir` is "up", "down", "top" or "bottom".
fn scroll_session(session: &str, dir: &str, lines: i64) {
    let tmux = tmux_bin();
    match dir {
        "top" => {
            let _ = std::process::Command::new(&tmux)
                .args(["copy-mode", "-e", "-t", session])
                .status();
            let _ = std::process::Command::new(&tmux)
                .args(["send-keys", "-t", session, "-X", "history-top"])
                .status();
        }
        "bottom" => {
            let _ = std::process::Command::new(&tmux)
                .args(["send-keys", "-t", session, "q"])
                .status();
        }
        "down" => {
            let _ = std::process::Command::new(&tmux)
                .args(["send-keys", "-t", session, "-X", "scroll-down"])
                .status();
        }
        _ => {
            // `copy-mode -e` leaves copy-mode as soon as the bottom is reached.
            let _ = std::process::Command::new(&tmux)
                .args(["copy-mode", "-e", "-t", session])
                .status();
            for _ in 0..lines.max(1).min(50) {
                let _ = std::process::Command::new(&tmux)
                    .args(["send-keys", "-t", session, "-X", "scroll-up"])
                    .status();
            }
        }
    }
}

fn tmux_bin() -> PathBuf {
    if let Ok(path) = std::env::var("TMUX_BIN") {
        return PathBuf::from(path);
    }
    let homebrew = PathBuf::from("/opt/homebrew/bin/tmux");
    if homebrew.exists() {
        homebrew
    } else {
        PathBuf::from("tmux")
    }
}
