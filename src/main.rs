// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * main.rs
 *
 * Version: @(#)main.rs 0.2.0 2026
 *
 * Description: The Cave Entrance.
 *              Five threads and a tray. Hotkey hear finger. Audio hear mouth.
 *              Parakeet hear audio. Inject put words in cursor. Supervisor watch
 *              them all: thread die, thread come back. Window close, app no die.
 *              App live in tray, like bat in cave.
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

//  Five threads and a tray, wired with channels. Each stage owns exactly one
//  job and one mode transition; nobody shares mutable anything except the
//  config (RwLock) and the mode byte (CAS — see state.rs).
//
//      hotkey ──Cmd──► audio ──Clip──► asr ──Transcript──► inject ──► perf.json
//      (LL hook)       (cpal)          (parakeet)          (paste/unicode/HID)
//
//      Every Clip and Transcript carries a Stopwatch started at hotkey release.
//
//      ui        main thread: tao loop, tray, on-demand WebView2 settings
//      bus       optional fbdirector handshake
//      watchdog  un-sticks the mode byte if a stage dies mid-flight
//
//  asr and inject run under `supervise()`: if they panic or error, they get
//  respawned after 2 s instead of leaving you with a tray icon that records
//  and never types. The ancestor of this program learned that the hard way.

// No console window in release. `bench` re-attaches to its parent terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod asr;
mod audio;
mod bench;
mod binding;
mod bus;
mod config;
mod dsp;
mod history;
mod hotkey;
mod inject;
mod interception;
mod ipc;
mod keymap;
mod models;
mod perf;
mod state;
mod tray;
mod ui;

use anyhow::{bail, Result};
use crossbeam_channel::unbounded;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::config::{Config, HotkeyMode, Loaded, APP};
use crate::state::{Mode, State};

const BANNER: &str = concat!("DivineVoice v", env!("CARGO_PKG_VERSION"));

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => daemon(false),
        Some("--settings") => daemon(true),
        Some("models") => {
            win::attach_parent_console();
            models::CATALOGUE.iter().for_each(|s| println!("{:24} {:>8.2} GB  {:9} {:13} {}",
                s.id, s.size() as f64 / 1e9, format!("{:?}", s.engine), s.langs,
                if s.installed() { "installed" } else { "-" }));
            Ok(())
        }
        Some("fetch") => {
            win::attach_parent_console();
            fetch(args.get(1).map(String::as_str))
        }
        Some("bench") => {
            win::attach_parent_console();
            let (cfg, _) = Config::load();
            bench::run(&args[1..], &cfg)
        }
        Some("--version" | "-V") => {
            win::attach_parent_console();
            println!("{BANNER}");
            Ok(())
        }
        Some(other) => {
            win::attach_parent_console();
            bail!("unknown argument {other:?}\n\
                   usage: divinevoice [--settings | --version]\n\
                   \x20      divinevoice models                      list the catalogue\n\
                   \x20      divinevoice fetch <id>                  download a model, headless\n\
                   \x20      divinevoice bench <audio> [--model <dir|id>] [--iters N] [--thread-scan 1,2,4,8] [--text]")
        }
    }
}

fn daemon(show_settings: bool) -> Result<()> {
    let (cfg, loaded) = Config::load();
    let _log = init_logging(&cfg.log_level);
    info!("{BANNER} — config {}", Config::path().display());
    match loaded {
        Loaded::Found      => {}
        Loaded::Created    => info!("first run: wrote default config"),
        Loaded::Broken(e)  => error!("config.toml doesn't parse, running on defaults (file left alone): {e}"),
    }

    let Some(_mutex) = win::single_instance() else {
        warn!("another {APP} is already running — two of us would type everything twice. Bye.");
        return Ok(());
    };

    // No model = first run (or a nuked one). Open Settings so the Models page
    // can fix it, instead of sitting silently in the tray being useless.
    let first_run = !config::looks_like_model(&cfg.model_dir);
    if first_run {
        warn!("no model in {} — opening Settings ▸ Models", cfg.model_dir.display());
    }

    hotkey::set_binding(cfg.hotkey, cfg.hotkey_mode == HotkeyMode::Toggle);
    hotkey::on_learned(ipc::on_learned);
    let state = Arc::new(State::new(cfg));

    let (cmd_tx,  cmd_rx)  = unbounded();
    let (clip_tx, clip_rx) = unbounded::<perf::Clip>();
    let (text_tx, text_rx) = unbounded::<perf::Transcript>();

    spawn("hotkey", &state, move |s| hotkey::run(s, cmd_tx));
    spawn("audio",  &state, move |s| audio::run(s, cmd_rx, clip_tx));
    supervise("asr",    &state, move |s| asr::run(s, &clip_rx, &text_tx));
    supervise("inject", &state, move |s| inject::run(s, &text_rx));
    spawn("bus",      &state, bus::run);
    spawn("watchdog", &state, watchdog);

    ui::run(state, show_settings || first_run)
}

/// `divinevoice fetch <id>`: the Models page download, minus the page. For
/// boxes you only reach over SSH, and for people who like progress bars in a
/// terminal. Resumes like the UI does, same files, same folder.
fn fetch(id: Option<&str>) -> Result<()> {
    use std::io::Write;
    let Some(spec) = id.and_then(models::find) else {
        bail!("usage: divinevoice fetch <id>   (see `divinevoice models` for ids)");
    };
    if spec.installed() {
        println!("{} already installed in {}", spec.id, spec.dir().display());
        return Ok(());
    }
    println!("fetching {} ({:.2} GB) from {} → {}", spec.id, spec.size() as f64 / 1e9, spec.source, spec.dir().display());
    let t0 = Instant::now();
    let never = std::sync::atomic::AtomicBool::new(false);
    models::download(spec, &never, |done, total| {
        let mb_s = done as f64 / 1e6 / t0.elapsed().as_secs_f64().max(0.001);
        print!("\r  {:5.1}%  {:>7.0} / {:.0} MB  {:5.1} MB/s ",
            done as f64 * 100.0 / total.max(1) as f64, done as f64 / 1e6, total as f64 / 1e6, mb_s);
        let _ = std::io::stdout().flush();
    })?;
    println!("\ndone in {:.0?}", t0.elapsed());
    Ok(())
}

/// Fire-and-forget thread. If it returns Err, we log it; nobody restarts it.
fn spawn<F>(name: &'static str, state: &Arc<State>, f: F)
where
    F: FnOnce(&State) -> Result<()> + Send + 'static,
{
    let s = Arc::clone(state);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || if let Err(e) = f(&s) { error!("{name}: {e:#}") })
        .expect("spawning a thread failed; the OS is having a worse day than you");
}

/// Like `spawn`, but a crash (Err or panic) gets a 2 s timeout and another go.
/// A clean Ok(()) means "channels closed, we're shutting down" and ends it.
fn supervise<F>(name: &'static str, state: &Arc<State>, f: F)
where
    F: Fn(&State) -> Result<()> + Send + 'static,
{
    spawn(name, state, move |s| {
        while s.alive() {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(s))) {
                Ok(Ok(()))  => break,
                Ok(Err(e))  => error!("{name} died: {e:#} — respawn in 2 s"),
                Err(_)      => error!("{name} panicked — respawn in 2 s"),
            }
            s.force(Mode::Idle);
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok(())
    });
}

/// If the mode byte sits on a non-Idle value way past any sane duration,
/// something died holding it. Reset it so the tray stops lying.
fn watchdog(s: &State) -> Result<()> {
    let max_rec = Duration::from_millis(s.cfg.read().max_ms as u64 + 5_000);
    let mut last = (s.mode(), Instant::now());
    while s.alive() {
        std::thread::sleep(Duration::from_millis(500));
        let m = s.mode();
        if m != last.0 { last = (m, Instant::now()); continue; }
        let limit = match m {
            Mode::Idle => continue,
            Mode::Recording => max_rec,
            Mode::Transcribing | Mode::Injecting => Duration::from_secs(60),
        };
        if last.1.elapsed() > limit && s.advance(m, Mode::Idle) {
            warn!("watchdog: stuck in {} for {:.0?} — forced idle", m.label(), last.1.elapsed());
            last = (Mode::Idle, Instant::now());
        }
    }
    Ok(())
}

/// stderr (only visible in debug builds) + a daily-rotated file in
/// %APPDATA%\DivineVoice, 7 days kept. ORT's graph optimiser is muted to warn
/// because it has opinions and shares all of them.
fn init_logging(level: &str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_appender::rolling::{RollingFileAppender, Rotation};
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

    let filter = || EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},ort=warn")));

    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("divinevoice")
        .filename_suffix("log")
        .max_log_files(7)
        .build(config::home())
        .ok();
    let (writer, guard) = file.map(tracing_appender::non_blocking).unzip();

    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(filter()))
        .with(writer.map(|w| fmt::layer().with_ansi(false).with_writer(w).with_filter(filter())))
        .init();
    guard
}

mod win {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    use windows::Win32::System::Threading::CreateMutexW;

    /// Release builds have no console; borrow the terminal that launched us.
    pub fn attach_parent_console() {
        unsafe { let _ = AttachConsole(ATTACH_PARENT_PROCESS); }
    }

    /// Named mutex. Some(handle) = we're the one; keep it alive for the
    /// process lifetime. None = someone beat us to it.
    pub fn single_instance() -> Option<HANDLE> {
        unsafe {
            let h = CreateMutexW(None, false, w!("Local\\DivineVoice.single-instance")).ok()?;
            (GetLastError() != ERROR_ALREADY_EXISTS).then_some(h)
        }
    }
}
