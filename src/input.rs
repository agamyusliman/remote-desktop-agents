use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use tracing::{error, info, warn};

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

/// Full display size in points, independent of any capture. Used to map the
/// global pointer position into captured frame coordinates.
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGMainDisplayID() -> u32;
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPointNative {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSizeNative {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPointNative,
    size: CGSizeNative,
}

pub fn main_display_size() -> (i32, i32) {
    #[cfg(target_os = "macos")]
    unsafe {
        let rect = CGDisplayBounds(CGMainDisplayID());
        return (rect.size.width as i32, rect.size.height as i32);
    }
    #[cfg(not(target_os = "macos"))]
    (0, 0)
}

pub struct Input {
    tx: mpsc::Sender<Value>,
    screen: Arc<Mutex<(i32, i32)>>,
    trusted: Arc<AtomicBool>,
}

impl Input {
    pub fn new() -> Self {
        let trusted = Arc::new(AtomicBool::new(post_event_access()));
        // Ask Apple to surface the prompt at most once per process. Re-requesting
        // on every launch makes an already-granted permission look missing.
        if !trusted.load(Ordering::Relaxed) && !already_requested() {
            warn!("Accessibility permission missing; requesting once");
            #[cfg(target_os = "macos")]
            unsafe {
                let _ = CGRequestPostEventAccess();
            }
            mark_requested();
        }

        let (tx, rx) = mpsc::channel::<Value>();
        let screen = Arc::new(Mutex::new((1512, 982)));
        let screen_thread = screen.clone();
        let trusted_thread = trusted.clone();

        std::thread::spawn(move || {
            let mut enigo = open_enigo();
            if enigo.is_some() {
                trusted_thread.store(true, Ordering::Relaxed);
            }
            if let Some(e) = enigo.as_ref() {
                if let Ok(size) = e.main_display() {
                    *screen_thread.lock().unwrap() = size;
                }
            }
            while let Ok(msg) = rx.recv() {
                if enigo.is_none() {
                    enigo = open_enigo();
                    if enigo.is_some() {
                        trusted_thread.store(true, Ordering::Relaxed);
                    }
                    if let Some(e) = enigo.as_ref() {
                        if let Ok(size) = e.main_display() {
                            *screen_thread.lock().unwrap() = size;
                        }
                    }
                }
                let Some(e) = enigo.as_mut() else {
                    continue;
                };
                handle_one(e, &msg, &screen_thread);
            }
        });

        Self { tx, screen, trusted }
    }

    pub fn trusted(&self) -> bool {
        // Permissions can be granted while the app is running. Re-check the
        // system state instead of trusting the value cached at startup.
        if !self.trusted.load(Ordering::Relaxed) && post_event_access() {
            self.trusted.store(true, Ordering::Relaxed);
        }
        self.trusted.load(Ordering::Relaxed)
    }

    pub fn screen_size(&self) -> (i32, i32) {
        *self.screen.lock().unwrap()
    }

    pub fn handle(&self, msg: &Value) {
        let _ = self.tx.send(msg.clone());
    }

    /// System check without side effects, for the setup UI.
    pub fn permission_state(&self) -> bool {
        post_event_access()
    }
}

fn open_enigo() -> Option<Enigo> {
    match Enigo::new(&Settings::default()) {
        Ok(e) => {
            info!("input backend ready");
            Some(e)
        }
        Err(err) => {
            warn!("input backend unavailable: {err}");
            None
        }
    }
}

fn handle_one(enigo: &mut Enigo, msg: &Value, screen: &Mutex<(i32, i32)>) {
    let op = msg.get("t").and_then(Value::as_str).unwrap_or_default();
    let (w, h) = *screen.lock().unwrap();

    let point = || -> Option<(i32, i32)> {
        let nx = msg.get("x")?.as_f64()?;
        let ny = msg.get("y")?.as_f64()?;
        Some((
            ((nx.clamp(0.0, 1.0)) * w as f64).round() as i32,
            ((ny.clamp(0.0, 1.0)) * h as f64).round() as i32,
        ))
    };

    let result: enigo::InputResult<()> = match op {
        "move" => match point() {
            Some((x, y)) => enigo.move_mouse(x, y, Coordinate::Abs),
            None => return,
        },
        "click" | "down" | "up" => {
            if let Some((x, y)) = point() {
                if let Err(err) = enigo.move_mouse(x, y, Coordinate::Abs) {
                    error!("move: {err}");
                    return;
                }
            }
            let button = match msg.get("button").and_then(Value::as_str) {
                Some("right") => Button::Right,
                Some("middle") => Button::Middle,
                _ => Button::Left,
            };
            let direction = match op {
                "down" => Direction::Press,
                "up" => Direction::Release,
                _ => Direction::Click,
            };
            if op == "click" {
                let count = msg
                    .get("count")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 3);
                let mut out = Ok(());
                for _ in 0..count {
                    out = enigo.button(button, Direction::Click);
                    if out.is_err() {
                        break;
                    }
                }
                out
            } else {
                enigo.button(button, direction)
            }
        }
        "scroll" => {
            let dx = msg.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
            let dy = msg.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
            let lines = |v: f64| -> i32 { ((v / 40.0).round() as i32).clamp(-12, 12) };
            let mut out = Ok(());
            let v = lines(dy);
            let hz = lines(dx);
            if v != 0 {
                out = enigo.scroll(v, Axis::Vertical);
            }
            if out.is_ok() && hz != 0 {
                out = enigo.scroll(hz, Axis::Horizontal);
            }
            out
        }
        "key" => key(enigo, msg),
        "text" => {
            let text = msg.get("text").and_then(Value::as_str).unwrap_or_default();
            if text.is_empty() {
                return;
            }
            enigo.text(text)
        }
        _ => return,
    };

    if let Err(err) = result {
        error!("input {op}: {err}");
    }
}

fn key(enigo: &mut Enigo, msg: &Value) -> enigo::InputResult<()> {
    let name = msg.get("key").and_then(Value::as_str).unwrap_or_default();
    let Some(key) = key_from_name(name) else {
        return Ok(());
    };
    let mods = msg
        .get("mods")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(modifier)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let action = msg.get("action").and_then(Value::as_str).unwrap_or("click");

    for m in &mods {
        enigo.key(*m, Direction::Press)?;
    }
    let out = match action {
        "press" => enigo.key(key, Direction::Press),
        "release" => enigo.key(key, Direction::Release),
        _ => enigo.key(key, Direction::Click),
    };
    for m in mods.iter().rev() {
        let _ = enigo.key(*m, Direction::Release);
    }
    out
}

fn modifier(name: &str) -> Key {
    match name {
        "cmd" | "meta" | "command" => Key::Meta,
        "ctrl" | "control" => Key::Control,
        "alt" | "option" => Key::Alt,
        _ => Key::Shift,
    }
}

fn key_from_name(name: &str) -> Option<Key> {
    let key = match name {
        "Enter" | "Return" => Key::Return,
        "Tab" => Key::Tab,
        " " | "Spacebar" | "Space" => Key::Space,
        "Backspace" => Key::Backspace,
        "Escape" | "Esc" => Key::Escape,
        "Delete" => Key::Delete,
        "ArrowLeft" | "Left" => Key::LeftArrow,
        "ArrowRight" | "Right" => Key::RightArrow,
        "ArrowUp" | "Up" => Key::UpArrow,
        "ArrowDown" | "Down" => Key::DownArrow,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        _ => {
            let mut chars = name.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Unicode(first)
        }
    };
    Some(key)
}

#[cfg(target_os = "macos")]
fn post_event_access() -> bool {
    unsafe { CGPreflightPostEventAccess() }
}

/// One prompt per install, tracked on disk so relaunching the tray app does not
/// re-prompt. Only a missing entry triggers a request.
#[cfg(target_os = "macos")]
fn request_marker() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library/Application Support/Remote Agents")
            .join("accessibility-requested"),
    )
}

#[cfg(target_os = "macos")]
fn already_requested() -> bool {
    request_marker().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn mark_requested() {
    if let Some(path) = request_marker() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, b"1");
    }
}

#[cfg(not(target_os = "macos"))]
fn post_event_access() -> bool {
    true
}
