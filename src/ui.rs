// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * ui.rs
 *
 * Version: @(#)ui.rs 0.3.0 2026
 *
 * Description: The Main Thread.
 *              tao event loop, tray, frameless WebView2 Settings that exist only
 *              while open. Assets baked into exe, served from dv://. Worker
 *              threads never touch the window: they post event, loop decide.
 *              Ownership: one weird trick C programmers hate.
 *
 * Author: Magnus Leatti (layer07)
 * URL: https://kernelriot.com
 * Github: /layer07
 *
 *        ██▓    ▄▄▄     ▓██   ██▓▓█████  ██▀███
 *       ▓██▒   ▒████▄    ▒██  ██▒▓█   ▀ ▓██ ▒ ██▒
 *       ▒██░   ▒██  ▀█▄   ▒██ ██░▒███   ▓██ ░▄█ ▒
 *       ▒██░   ░██▄▄▄▄██  ░ ▐██▓░▒▓█ ▄ ▒██▀▀█▄
 *       ░██████▒▓█   ▓██▒ ░ ██▒▓░░▒████▒░██▓ ▒██▒
 *       ░ ▒░▓  ░▒▒   ▓▒█░  ██▒▒▒ ░░ ▒░ ░░ ▒▓ ░▒▓░
 *       ░ ░ ▒  ░ ▒   ▒▒ ░▓██ ░▒░  ░ ░  ░  ░▒ ░ ▒░
 *         ░ ░    ░   ▒   ▒ ▒ ░░     ░     ░░   ░
 *           ░  ░     ░  ░░ ░        ░  ░   ░
 */

// The Settings window is a WebView2 that EXISTS ONLY WHILE OPEN: close it and
// the browser process goes with it, so the dictation pipeline never shares a
// box with Chromium it didn't ask for. Assets are baked into the exe and
// served from a custom `dv://` scheme (which WebView2 spells
// http://dv.localhost/). No files on disk, no localhost server, no CORS
// séance.
//
// Data flow:
//   JS → Rust   window.ipc.postMessage(json)  → UserEvent::Ipc → ipc::handle
//   Rust → JS   ui::push(ev, data) from any thread → UserEvent::Push → dv.recv(...)
//
// Worker threads never touch the window. They post events; the loop, which
// owns the window, decides what to do with them. Ownership: the one weird
// trick C programmers hate.

use anyhow::Result;
use parking_lot::Mutex;
use serde::Serialize;
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Icon, Theme, Window, WindowBuilder};
use tracing::{debug, info, warn};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use wry::http::{header::CONTENT_TYPE, Request, Response};
use wry::{WebContext, WebView, WebViewBuilder};

use crate::config::{self, APP};
use crate::ipc;
use crate::state::{Mode, State};
use crate::tray::{self, Tray};

pub enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    Ipc(String),
    /// Pre-serialised `{"ev":..,"data":..}` for the page.
    Push(String),
    /// Something changed; resend the state snapshot.
    Refresh,
}

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

fn post(e: UserEvent) {
    if let Some(p) = PROXY.get() { let _ = p.lock().send_event(e); }
}

/// Send an event to the page, from any thread. No window open = no-op.
pub fn push<T: Serialize + ?Sized>(ev: &str, data: &T) {
    let msg = serde_json::json!({ "ev": ev, "data": data });
    if ECHO.load(std::sync::atomic::Ordering::Relaxed) { println!("{msg}"); }
    post(UserEvent::Push(msg.to_string()));
}

/// Headless mode (`divinevoice race`): page events go to stdout as JSON lines
/// instead of into a window. Same events, so the CLI is the UI's twin.
static ECHO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn echo_to_stdout() { ECHO.store(true, std::sync::atomic::Ordering::Relaxed) }

/// "The world changed, re-render." Cheap; the loop coalesces nothing and
/// doesn't need to — these are rare.
pub fn push_state() { post(UserEvent::Refresh) }

struct Settings { window: Window, view: WebView }

impl Settings {
    fn send(&self, js_obj: &str) {
        if let Err(e) = self.view.evaluate_script(&format!("window.dv && dv.recv({js_obj})")) {
            warn!("ui: evaluate_script: {e}");
        }
    }
}

pub fn run(state: Arc<State>, open_at_start: bool) -> Result<()> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let _ = PROXY.set(Mutex::new(proxy.clone()));

    // tray-icon/muda deliver on their own channels; forward them into the loop
    // so a click actually wakes us instead of waiting for the next tick.
    let p = Mutex::new(proxy.clone());
    TrayIconEvent::set_event_handler(Some(move |e| { let _ = p.lock().send_event(UserEvent::Tray(e)); }));
    let p = Mutex::new(proxy.clone());
    MenuEvent::set_event_handler(Some(move |e| { let _ = p.lock().send_event(UserEvent::Menu(e)); }));

    let tray = Tray::new(&state)?;
    let mut web_ctx = WebContext::new(Some(config::home().join("webview")));
    let mut settings: Option<Settings> = None;
    let mut shown = (Mode::Idle, false);
    info!("ui up — left-click the tray icon for Settings");

    event_loop.run_return(|event, target, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

        let mut open = false;
        match event {
            Event::NewEvents(StartCause::Init) => open = open_at_start,

            Event::UserEvent(UserEvent::Tray(
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. },
            )) => open = true,

            Event::UserEvent(UserEvent::Menu(m)) => match tray.action(&m.id) {
                Some(tray::Action::Settings) => open = true,
                Some(tray::Action::Logs)     => ipc::open_path(&config::home()),
                Some(tray::Action::Quit)     => { info!("quit from tray"); state.shutdown(); }
                None => {}
            },

            Event::UserEvent(UserEvent::Ipc(msg)) => match chrome_cmd(&msg) {
                // Our own titlebar. The page has no window handle; we do.
                Some(Chrome::Drag) => if let Some(s) = &settings { let _ = s.window.drag_window(); },
                // Minimise = hide to tray, WebView kept warm for an instant reopen.
                // (Both also disarm the Benchmark tab: a hidden window must
                // never swallow your dictation.)
                Some(Chrome::Hide) => {
                    state.disarm_bench();
                    if let Some(s) = &settings { s.window.set_visible(false); }
                }
                // Close = to tray too, but the WebView2 process goes with it.
                Some(Chrome::Close) => {
                    state.disarm_bench();
                    hotkey_learn_abort();
                    settings = None;
                    info!("settings closed (still in the tray)");
                }
                None => {
                    let out = ipc::handle(&state, &msg);
                    if let Some(s) = &settings { out.iter().for_each(|o| s.send(o)); }
                    tray.refresh(&state); // hotkey / mode may have changed
                }
            },

            Event::UserEvent(UserEvent::Push(js)) => {
                if let Some(s) = &settings { s.send(&js); }
            }

            Event::UserEvent(UserEvent::Refresh) => {
                if let Some(s) = &settings { s.send(&ipc::envelope("state", &ipc::snapshot(&state, false))); }
            }

            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                state.disarm_bench();
                hotkey_learn_abort();
                settings = None; // drops the WebView2 process with it
                info!("settings closed");
            }

            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let now = (state.mode(), state.bus_connected.load(std::sync::atomic::Ordering::Acquire));
                if now != shown {
                    shown = now;
                    tray.refresh(&state);
                    if let Some(s) = &settings {
                        s.send(&ipc::envelope("status", &serde_json::json!({ "mode": now.0, "bus": now.1 })));
                    }
                }
            }
            _ => {}
        }

        if open {
            match &settings {
                Some(s) => { s.window.set_visible(true); s.window.set_minimized(false); s.window.set_focus(); }
                None => match build_settings(target, &mut web_ctx, proxy.clone()) {
                    Ok(s) => { info!("settings opened"); settings = Some(s); }
                    Err(e) => warn!("couldn't open settings: {e:#} (is the WebView2 runtime installed?)"),
                },
            }
        }

        if !state.alive() { *flow = ControlFlow::Exit; }
    });
    Ok(())
}

fn hotkey_learn_abort() { crate::hotkey::cancel_learn() }

/// Fixed window size, in logical px. Every page is laid out to fit inside it;
/// if you add a page that needs more, the page is wrong, not the window.
pub const WIN_W: f64 = 1100.0;
pub const WIN_H: f64 = 720.0;

enum Chrome { Drag, Hide, Close }

/// Titlebar commands are window business, so the loop handles them itself
/// instead of routing through ipc.rs (which never sees the window).
fn chrome_cmd(raw: &str) -> Option<Chrome> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    match v["cmd"].as_str()? {
        "win_drag"  => Some(Chrome::Drag),
        "win_min"   => Some(Chrome::Hide),
        "win_close" => Some(Chrome::Close),
        _ => None,
    }
}

fn build_settings(
    target: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
    ctx: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<Settings> {
    use tao::platform::windows::WindowBuilderExtWindows;
    // Frameless, fixed, no maximise. The titlebar is ours (ui/index.html);
    // the native shadow stays so it doesn't look like a sticker on the desktop.
    let window = WindowBuilder::new()
        .with_title(APP)
        .with_inner_size(tao::dpi::LogicalSize::new(WIN_W, WIN_H))
        .with_resizable(false)
        .with_maximizable(false)
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_theme(Some(Theme::Dark))
        .with_background_color((10, 10, 10, 255))
        .with_window_icon(Icon::from_rgba(tray::rgba(Mode::Idle), 32, 32).ok())
        .build(target)?;

    let ipc_proxy = Mutex::new(proxy);
    let view = WebViewBuilder::new_with_web_context(ctx)
        .with_custom_protocol("dv".into(), |_id, req| serve(&req))
        .with_ipc_handler(move |req: Request<String>| {
            let _ = ipc_proxy.lock().send_event(UserEvent::Ipc(req.into_body()));
        })
        // Runs before any page script: JS errors land in OUR log, not in a
        // devtools console nobody has open.
        .with_initialization_script(JS_ERROR_TRAP)
        .with_on_page_load_handler(|ev, url| debug!("page {} {url}",
            if matches!(ev, wry::PageLoadEvent::Started) { "loading" } else { "loaded" }))
        .with_background_color((10, 10, 10, 255))
        .with_devtools(cfg!(debug_assertions))
        .with_url("dv://localhost/")
        .build(&window)?;

    Ok(Settings { window, view })
}

const JS_ERROR_TRAP: &str = r#"
  const __dvReport = (msg) => { try { window.ipc.postMessage(JSON.stringify({ cmd: 'jserror', msg })) } catch (_) {} };
  window.addEventListener('error', (e) => __dvReport(`${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`));
  window.addEventListener('unhandledrejection', (e) => __dvReport(`unhandled rejection: ${e.reason}`));
"#;

// ── embedded assets ──────────────────────────────────────────────────────────

const ASSETS: &[(&str, &str, &[u8])] = &[
    ("/",            "text/html; charset=utf-8",       include_bytes!("../ui/index.html")),
    ("/theme.css",   "text/css; charset=utf-8",        include_bytes!("../ui/theme.css")),
    ("/app.css",     "text/css; charset=utf-8",        include_bytes!("../ui/app.css")),
    ("/app.js",      "text/javascript; charset=utf-8", include_bytes!("../ui/app.js")),
    ("/themes.js",   "text/javascript; charset=utf-8", include_bytes!("../ui/themes.js")),
    ("/speed.js",    "text/javascript; charset=utf-8", include_bytes!("../ui/speed.js")),
    // 634 KB, 121 frames of a pink pixel devil-cat jumping. Worth every byte.
    ("/mascot.webp", "image/webp",                     include_bytes!("../ui/mascot.webp")),
];

fn serve(req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = match req.uri().path() { "/index.html" => "/", p => p };
    // /clip/<id>: benchmark audio for play-along (baked or from %APPDATA%).
    let clip = path.strip_prefix("/clip/").and_then(crate::clips::audio);
    let (status, mime, body): (u16, &str, Cow<'static, [u8]>) = match clip {
        Some((bytes, mime)) => (200, mime, bytes),
        None => ASSETS.iter().find(|(p, _, _)| *p == path)
            .map(|&(_, m, b)| (200, m, Cow::Borrowed(b)))
            .unwrap_or((404, "text/plain", Cow::Borrowed(b"404: not in the binary".as_slice()))),
    };
    debug!("dv:// {} → {status}", req.uri());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, mime)
        .header("Content-Security-Policy",
            "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; img-src 'self' data:")
        .body(body)
        .expect("static response")
}
