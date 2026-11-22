use crate::config::Config;

/// macOS ScreenCaptureKit (via scap) ignores `show_cursor`, so the pointer is
/// composited onto the frame here. Without this the phone shows a screen with
/// no visible cursor, which makes remote control feel broken.
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGEventGetLocation(event: *mut std::ffi::c_void) -> CGPoint;
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
fn pointer_location() -> Option<(f64, f64)> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

#[cfg(not(target_os = "macos"))]
fn pointer_location() -> Option<(f64, f64)> {
    None
}

/// Draw a small arrow at the pointer position, scaled into the capture frame.
fn draw_cursor(rgb: &mut [u8], width: u32, height: u32, scale_x: f64, scale_y: f64) {
    let Some((px, py)) = pointer_location() else { return };
    let cx = (px * scale_x) as i32;
    let cy = (py * scale_y) as i32;
    let w = width as i32;
    let h = height as i32;
    if w <= 0 || h <= 0 {
        return;
    }

    // 10x18 arrow, in frame pixels. Bright core + dark edge so it reads on any
    // background, like a real OS pointer.
    const SHAPE: &[(i32, i32)] = &[
        (0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (0, 8), (0, 9), (0, 10),
        (1, 2), (2, 4), (3, 6), (4, 8), (5, 10), (6, 12), (7, 14), (8, 16),
        (1, 8), (2, 9), (3, 10), (4, 11),
        (2, 12), (3, 13), (4, 14), (5, 15),
    ];
    let edge = 1i32;
    for (dx, dy) in SHAPE {
        for ox in -edge..=edge {
            for oy in -edge..=edge {
                let x = cx + dx + ox;
                let y = cy + dy + oy;
                if x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                let idx = ((y * w + x) * 3) as usize;
                if idx + 2 < rgb.len() {
                    rgb[idx] = 20;
                    rgb[idx + 1] = 24;
                    rgb[idx + 2] = 32;
                }
            }
        }
    }
    for (dx, dy) in SHAPE {
        let x = cx + dx;
        let y = cy + dy;
        if x < 0 || y < 0 || x >= w || y >= h {
            continue;
        }
        let idx = ((y * w + x) * 3) as usize;
        if idx + 2 < rgb.len() {
            rgb[idx] = 255;
            rgb[idx + 1] = 255;
            rgb[idx + 2] = 255;
        }
    }
}
use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};
use scap::{
    capturer::{Capturer, Options},
    frame::{Frame, FrameType},
};
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Guards the one-and-only permission prompt for this process.
static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Debug, Serialize)]
pub struct CaptureStatus {
    pub ok: bool,
    pub error: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: u8,
    pub resolution: &'static str,
    pub frames: u64,
}

impl CaptureStatus {
    fn starting(cfg: &Config) -> Self {
        Self {
            ok: false,
            error: None,
            width: 0,
            height: 0,
            fps: cfg.fps,
            quality: cfg.quality,
            resolution: cfg.resolution.label(),
            frames: 0,
        }
    }
}

pub struct Capture {
    pub tx: broadcast::Sender<Arc<Vec<u8>>>,
    pub latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    pub status: Arc<Mutex<CaptureStatus>>,
    clients: Arc<AtomicUsize>,
}

impl Capture {
    pub fn start(cfg: Config) -> Arc<Self> {
        let (tx, _) = broadcast::channel(4);
        let capture = Arc::new(Self {
            tx: tx.clone(),
            latest: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(CaptureStatus::starting(&cfg))),
            clients: Arc::new(AtomicUsize::new(0)),
        });

        let status = capture.status.clone();
        let latest = capture.latest.clone();
        let tx_thread = tx.clone();
        let clients = capture.clients.clone();

        std::thread::spawn(move || {
          loop {
            // Outer cycle: create a capturer, feed it to the reader thread, and
            // restart if the stream dies.
            // No viewer: stop the capture engine entirely. This is the main
            // thermal/battery win when the phone is idle or on the terminal tab.
            if clients.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(Duration::from_millis(300));
                continue;
            }
            if !scap::has_permission() {
                {
                    let mut s = status.lock().unwrap();
                    s.ok = false;
                    s.error = Some("Screen Recording permission not granted".into());
                }
                // Ask macOS once per process. Re-requesting on every retry makes
                // a granted permission look broken and reopens the dialog.
                if !ASKED.swap(true, Ordering::Relaxed) {
                    warn!("screen recording permission missing; requesting once");
                    let _ = scap::request_permission();
                } else {
                    // The user may have granted it in System Settings while we
                    // kept polling; a fresh preflight notices without a restart.
                    if scap::has_permission() {
                        info!("screen recording permission granted");
                        continue;
                    }
                }
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }

            let options = Options {
                fps: 15,
                show_cursor: true,
                show_highlight: false,
                output_type: FrameType::BGRAFrame,
                output_resolution: cfg.resolution.as_scap(),
                ..Default::default()
            };

            let mut capturer = match Capturer::build(options) {
                Ok(c) => c,
                Err(err) => {
                    let mut s = status.lock().unwrap();
                    s.ok = false;
                    s.error = Some(err.to_string());
                    drop(s);
                    error!("capturer build failed: {err}");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };

            info!(
                "capture started: {} @ {} fps, jpeg q{}",
                cfg.resolution.label(),
                cfg.fps,
                cfg.quality
            );
            capturer.start_capture();

            // scap's get_next_frame() blocks on an internal mpsc channel that
            // only yields when the screen *changes*. Reading it directly means
            // an idle desktop starves the loop (no cursor updates, no
            // keep-alive). A dedicated reader thread decouples the two: it
            // always consumes frames, and the compositor here runs on a timer.
            let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<(Vec<u8>, u32, u32)>(3);
            let producer_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let producer_stop_thread = producer_stop.clone();

            let reader = std::thread::spawn(move || {
                while !producer_stop_thread.load(Ordering::Relaxed) {
                    match capturer.get_next_frame() {
                        Ok(Frame::BGRA(f)) => {
                            let _ = frame_tx.send((f.data, f.width as u32, f.height as u32));
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                capturer.stop_capture();
            });

            let min_interval = Duration::from_millis(1000 / cfg.fps as u64);
            let mut last_sent = Instant::now() - min_interval;
            let mut rgb = Vec::new();
            let mut last_hash = 0u64;
            let mut last_raw: Option<(Vec<u8>, u32, u32)> = None;
            let mut last_cursor = (f64::NAN, f64::NAN);

            loop {
                if clients.load(Ordering::Relaxed) == 0 {
                    break;
                }

                // Prefer a fresh frame; otherwise reuse the last one so the
                // compositor can still emit cursor movement and keep-alives.
                let fresh = frame_rx.recv_timeout(Duration::from_millis(40)).ok();
                let tick_only = fresh.is_none();
                let (data, width, height) = match fresh {
                    Some((d, w, h)) => {
                        last_raw = Some((d.clone(), w, h));
                        (d, w, h)
                    }
                    None => match last_raw.as_ref() {
                        Some((d, w, h)) => (d.clone(), *w, *h),
                        None => continue,
                    },
                };
                if !tick_only && last_sent.elapsed() < min_interval {
                    continue;
                }
                if tick_only {
                    // Static screen: refresh when the pointer moved, otherwise
                    // send a keep-alive frame about once a second so the client
                    // never sits on a stale image.
                    let now_cursor = pointer_location().unwrap_or((0.0, 0.0));
                    let moved = (now_cursor.0 - last_cursor.0).abs() > 0.4
                        || (now_cursor.1 - last_cursor.1).abs() > 0.4;
                    if !moved && last_sent.elapsed() < std::time::Duration::from_millis(1000) {
                        std::thread::sleep(std::time::Duration::from_millis(40));
                        continue;
                    }
                    last_cursor = now_cursor;
                }

                // Cheap change detection: skip JPEG encode/send for an idle screen.
                let mut hash = 1469598103934665603u64;
                for byte in data.iter().step_by(97) {
                    hash ^= *byte as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }

                // The pointer is composited by us, so it must also drive the
                // change test; otherwise moving the mouse over a static screen
                // produces no frames at all.
                let cursor = pointer_location();
                if let Some((cx, cy)) = cursor {
                    hash ^= (cx as i64 as u64).wrapping_mul(0x9E3779B97F4A7C15);
                    hash ^= (cy as i64 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F);
                }
                if hash == last_hash {
                    continue;
                }
                last_hash = hash;

                rgb.clear();
                rgb.reserve(data.len() / 4 * 3);
                for px in data.chunks_exact(4) {
                    rgb.extend_from_slice(&[px[2], px[1], px[0]]);
                }

                // Scale from the full screen to the captured frame.
                if let Some((_, _)) = cursor {
                    let (sw, sh) = crate::input::main_display_size();
                    if sw > 0 && sh > 0 {
                        draw_cursor(
                            &mut rgb,
                            width,
                            height,
                            width as f64 / sw as f64,
                            height as f64 / sh as f64,
                        );
                    }
                }

                let mut jpeg = Vec::with_capacity(160 * 1024);
                let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, cfg.quality);
                if encoder
                    .encode(&rgb, width, height, ExtendedColorType::Rgb8)
                    .is_err()
                {
                    continue;
                }

                last_sent = Instant::now();
                let jpeg = Arc::new(jpeg);
                *latest.lock().unwrap() = Some(jpeg.clone());
                let _ = tx_thread.send(jpeg);

                let mut s = status.lock().unwrap();
                s.ok = true;
                s.error = None;
                s.width = width;
                s.height = height;
                s.frames += 1;
            }

            producer_stop.store(true, Ordering::Relaxed);
            let _ = reader.join();
            std::thread::sleep(Duration::from_millis(300));
          }
        });

        capture
    }

    pub fn add_client(&self) -> usize {
        self.clients.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn remove_client(&self) -> usize {
        self.clients.fetch_sub(1, Ordering::Relaxed).saturating_sub(1)
    }

    pub fn client_count(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }
}
