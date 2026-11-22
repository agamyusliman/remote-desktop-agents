//! Native desktop shell: a small onboarding window (QR + status) and a macOS
//! status-bar item. The HTTP server runs on the same Tokio runtime.

use crate::config::Config;
use anyhow::Result;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::{ActivationPolicy, EventLoopExtMacOS},
    window::{Window, WindowBuilder},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};
use wry::WebViewBuilder;

#[derive(Debug, Clone)]
pub enum UserEvent {
    ToggleWindow,
    ShowWindow,
    Quit,
}

pub struct DesktopHandle {
    pub window_open: Arc<AtomicBool>,
    tx: tao::event_loop::EventLoopProxy<UserEvent>,
}

pub fn is_headless() -> bool {
    std::env::var("REMOT_AGENT_HEADLESS").ok().as_deref() == Some("1")
        || std::env::var("REMOT_AGENT_DESKTOP").ok().as_deref() == Some("0")
}

/// Starts the native shell. Must be called on the main thread before the Tokio
/// runtime blocks; the caller should run the server in a background runtime.
pub fn run(cfg: Config, ready_url: String, shutdown: Arc<AtomicBool>) -> Result<()> {
    eprintln!("desktop ready_url={ready_url:?}");
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Accessory app: no Dock icon; status-bar item only.
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let window_open = Arc::new(AtomicBool::new(false));
    let window_open_thread = window_open.clone();
    let mut window: Option<Window> = None;
    let mut webview: Option<wry::WebView> = None;
    let mut tray: Option<TrayIcon> = None;
    let mut last_title = String::new();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(800),
        );

        match event {
            Event::NewEvents(StartCause::Init) => {
                let menu = Menu::new();
                let open_item = MenuItem::new("Open Remote Agents", true, None);
                let quit_item = MenuItem::new("Quit", true, None);
                let _ = menu.append_items(&[
                    &open_item,
                    &PredefinedMenuItem::separator(),
                    &quit_item,
                ]);
                let open_id = open_item.id().clone();
                let quit_id = quit_item.id().clone();
                let proxy_menu = proxy.clone();

                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    if event.id == open_id {
                        let _ = proxy_menu.send_event(UserEvent::ShowWindow);
                    } else if event.id == quit_id {
                        let _ = proxy_menu.send_event(UserEvent::Quit);
                    }
                }));

                let icon = load_icon();
                let mut builder = TrayIconBuilder::new()
                    .with_tooltip("Remote Agents")
                    .with_menu(Box::new(menu));
                if let Some(icon) = icon {
                    builder = builder.with_icon(icon);
                }
                // macOS menu bar: the glyph must be a template image, otherwise
                // the system renders it as a solid block.
                #[cfg(target_os = "macos")]
                {
                    builder = builder.with_icon_as_template(true);
                }
                tray = builder.build().ok();

                // Show the onboarding window on first launch.
                let _ = proxy.send_event(UserEvent::ShowWindow);
            }

            Event::UserEvent(UserEvent::ShowWindow) => {
                if window.is_none() {
                    let win = WindowBuilder::new()
                        .with_title("Remote Agents")
                        // Landscape: setup is a side-by-side workspace, not a
                        // long scrolling page.
                        .with_inner_size(tao::dpi::LogicalSize::new(960.0, 620.0))
                        .with_min_inner_size(tao::dpi::LogicalSize::new(720.0, 480.0))
                        .with_resizable(true)
                        .build(target)
                        .expect("window");
                    // `ready_url` is like "http://127.0.0.1:8788/"; keep the
                    // separator or the webview gets a host of "8788desktop".
                    let desktop_url = format!("{}/desktop", ready_url.trim_end_matches('/'));
                    eprintln!("desktop loading={desktop_url:?}");
                    let view = WebViewBuilder::new()
                        .with_url(&desktop_url)
                        .with_initialization_script(
                            "window.__RA_DESKTOP__=true;",
                        )

                        .with_devtools(false)
                        .build(&win)
                        .expect("webview");
                    window = Some(win);
                    webview = Some(view);
                }
                if let Some(win) = window.as_ref() {
                    win.set_visible(true);
                    win.set_focus();
                    window_open_thread.store(true, Ordering::Relaxed);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(win) = window.as_ref() {
                    win.set_visible(false);
                }
                window_open_thread.store(false, Ordering::Relaxed);
            }

            Event::UserEvent(UserEvent::Quit) => {
                shutdown.store(true, Ordering::Relaxed);
                tray.take();
                webview.take();
                window.take();
                *control_flow = ControlFlow::Exit;
            }

            Event::LoopDestroyed => {
                shutdown.store(true, Ordering::Relaxed);
            }

            Event::MainEventsCleared => {
                // Keep the tray tooltip fresh without a timer thread.
                if let Some(tray) = tray.as_ref() {
                    let title = format!("Remote Agents · {}", ready_url);
                    if title != last_title {
                        let _ = tray.set_tooltip(Some(&title));
                        last_title = title;
                    }
                }
            }

            _ => {}
        }
    });
}

fn load_icon() -> Option<tray_icon::Icon> {
    // The logo ships with the app: a monochrome template for the macOS menu bar,
    // a full-colour icon for the Windows/Linux notification area.
    let exe = std::env::current_exe().ok()?;
    let base = exe.parent()?.join("../Resources");
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["tray.png", "tray-32.png"]
    } else {
        &["tray-color.png", "icon-96.png", "tray.png"]
    };
    let path = candidates
        .iter()
        .map(|name| base.join(name))
        .find(|p| p.is_file())?;
    if let Ok(img) = image::open(&path) {
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        let rgba = img.into_raw();
        return tray_icon::Icon::from_rgba(rgba, w, h).ok();
    }
    let size = 18u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for _ in 0..size * size {
        rgba.extend_from_slice(&[150, 196, 255, 255]);
    }
    tray_icon::Icon::from_rgba(rgba, size, size).ok()
}
