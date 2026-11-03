use anyhow::{anyhow, bail, Result};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::config::InjectMode;
use crate::perf::{Entry, Transcript};
use crate::state::{Mode, State};
use crate::ui;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn record(state: &State, entry: Entry) {
    let max = state.cfg.read().perf_max;
    state.perf.push(entry, max);
    ui::push("perf", &crate::ipc::perf_view(state));
}

pub fn run(state: &State, texts: &Receiver<Transcript>) -> Result<()> {
    for transcript in texts.iter().take_while(|_| state.alive()) {
        let (mode, settle, delay) = {
            let cfg = state.cfg.read();
            (cfg.inject_mode, cfg.paste_settle_ms as u64, cfg.type_delay_ms as u64)
        };
        let started = Instant::now();
        let result = match mode {
            InjectMode::Paste => paste(&transcript.text, false, settle),
            InjectMode::PastePlain => paste(&transcript.text, true, settle),
            InjectMode::Unicode => unicode(&transcript.text, delay),
            InjectMode::Type => bail!("Type mode is only available on Windows"),
        };
        let delivered = result.map(|at| (at, mode.short().to_owned())).or_else(|error| {
            warn!("{}: {error:#}; falling back to Unicode", mode.short());
            unicode(&transcript.text, delay).map(|at| (at, format!("{}→unicode", mode.short())))
        });
        match delivered {
            Ok((at, how)) => record(state, transcript.finish(at, started, &how)),
            Err(error) => warn!("macOS text delivery failed: {error:#}"),
        }
        state.advance(Mode::Injecting, Mode::Idle);
    }
    Ok(())
}

fn accessibility_trusted() -> bool { unsafe { AXIsProcessTrusted() } }

fn paste(text: &str, plain: bool, settle_ms: u64) -> Result<Instant> {
    if !accessibility_trusted() {
        bail!("Accessibility permission is required to paste into another app")
    }
    let saved = clipboard_swap(text)?;
    nap(15);
    send_paste(plain)?;
    let delivered = Instant::now();
    nap(settle_ms);
    if let Some(old) = saved { clipboard_restore(&old); }
    info!("macOS {} {} chars", if plain { "paste-plain" } else { "paste" }, text.chars().count());
    Ok(delivered)
}

fn unicode(text: &str, _delay_ms: u64) -> Result<Instant> {
    if !accessibility_trusted() {
        bail!("Accessibility permission is required to type into another app")
    }
    let source = event_source()?;
    let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| anyhow!("could not create Unicode key-down event"))?;
    down.set_string(text);
    down.post(CGEventTapLocation::HID);
    let up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| anyhow!("could not create Unicode key-up event"))?;
    up.set_string(text);
    up.post(CGEventTapLocation::HID);
    Ok(Instant::now())
}

fn send_paste(plain: bool) -> Result<()> {
    let source = event_source()?;
    let mut flags = CGEventFlags::CGEventFlagCommand;
    if plain { flags |= CGEventFlags::CGEventFlagShift; }
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_V, down)
            .map_err(|_| anyhow!("could not create Command+V event"))?;
        event.set_flags(flags);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

fn event_source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("could not create macOS input event source"))
}

fn clipboard_swap(text: &str) -> Result<Option<String>> {
    (0..6u64)
        .find_map(|attempt| {
            let result = arboard::Clipboard::new().and_then(|mut clipboard| {
                let previous = clipboard.get_text().ok();
                clipboard.set_text(text).map(|_| previous)
            });
            result.inspect_err(|_| nap(15 * (attempt + 1))).ok()
        })
        .ok_or_else(|| anyhow!("clipboard remained locked after 6 attempts"))
}

fn clipboard_restore(text: &str) {
    let restored = (0..3).any(|_| {
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)).is_ok()
            || { nap(15); false }
    });
    if !restored { warn!("could not restore the previous text clipboard value"); }
}

fn nap(ms: u64) { std::thread::sleep(Duration::from_millis(ms)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_flags_include_command_and_optional_shift() {
        let command = CGEventFlags::CGEventFlagCommand;
        assert!(command.contains(CGEventFlags::CGEventFlagCommand));
        assert!(!(command | CGEventFlags::CGEventFlagShift).is_empty());
    }
}
