// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * inject.rs
 *
 * Version: @(#)inject.rs 0.3.1 2026
 *
 * Description: The Last Mile.
 *              Words go to cursor four ways: Paste, Paste plain, Unicode, real
 *              HID scancode through Interception. Only scancode cross a KVM;
 *              paste on KVM paste yesterday's clipboard. Something fail? Fall to
 *              Unicode. Transcript never dropped.
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

//   mode         mechanism                       reaches                 charset
//   ───────────  ──────────────────────────────  ──────────────────────  ───────
//   Paste        clipboard + SendInput Ctrl+V     local                   all
//   PastePlain   clipboard + Ctrl+Shift+V         local terminals         all
//   Unicode      SendInput KEYEVENTF_UNICODE      local                   all
//   Type         Interception set-1 scancodes     local · Parsec · KVM    US ASCII
//
// Why Type is the only one that crosses a KVM: Paste sets THIS machine's
// clipboard, then presses Ctrl+V on the TARGET — which pastes the target's
// clipboard. Congrats, you just pasted whatever you copied yesterday. Only real
// keystrokes carry the actual text across a HID boundary.
//
// Degradation ladder (a transcript is never silently dropped):
//   Paste/PastePlain ── clipboard wedged ──► Unicode
//   Type             ── no Interception ───► Unicode

use anyhow::{anyhow, Result};
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_SHIFT, VK_V,
};

use crate::config::InjectMode;
use crate::hotkey::OUR_INJECTION;
use crate::interception::{self as ic, Driver, KeyStroke};
use crate::keymap::{self, sc};
use crate::perf::{Entry, Transcript};
use crate::state::{Mode, State};
use crate::ui;

fn nap(ms: u64) { std::thread::sleep(Duration::from_millis(ms)) }

/// File a finished benchmark and send the Perf page the fresh view. Stats are
/// computed once, in Rust (perf::summarize, unit-tested), never re-derived in JS.
pub fn record(state: &State, e: Entry) {
    info!("perf: total {} ms (asr {} · deliver {}){}", e.total_ms, e.asr_ms, e.inject_ms,
        if e.cold { " cold" } else { "" });
    let max = state.cfg.read().perf_max;
    state.perf.push(e, max);
    ui::push("perf", &crate::ipc::perf_view(state));
}

// Every injector returns the Instant the text was DELIVERED — Ctrl+V sent, or
// the last key sent. Anything after that (clipboard restore, settle naps) is
// housekeeping you don't wait on, so it stays off the stopwatch.

pub fn run(state: &State, texts: &Receiver<Transcript>) -> Result<()> {
    // Opened once per (re)spawn. Missing driver is a fact of life, not an error.
    let driver = Driver::open()
        .inspect(|_| info!("interception: online — Type mode available"))
        .inspect_err(|e| info!("interception: {e} — Type mode will fall back to Unicode"))
        .ok();
    state.interception.store(driver.is_some(), std::sync::atomic::Ordering::Release);

    for t in texts.iter().take_while(|_| state.alive()) {
        let (mode, settle, delay, skip) = {
            let c = state.cfg.read();
            (c.inject_mode, c.paste_settle_ms as u64, c.type_delay_ms as u64, c.skip_hwids.clone())
        };
        let text = &t.text;
        let started = Instant::now();

        let result = match mode {
            InjectMode::Paste      => paste(text, false, settle),
            InjectMode::PastePlain => paste(text, true,  settle),
            InjectMode::Unicode    => unicode(text, delay),
            InjectMode::Type       => match driver.as_ref().map(|d| (d, d.find_keyboard(&skip))) {
                Some((d, Some(dev))) => hid_type(d, dev, text, delay),
                Some((_, None)) => Err(anyhow!("no usable keyboard in Interception slots 1..=10")),
                None => Err(anyhow!("Interception unavailable")),
            },
        };

        let delivered = result.map(|at| (at, mode.short().to_owned())).or_else(|e| {
            warn!("{}: {e:#} — falling back to unicode", mode.short());
            unicode(text, delay).map(|at| (at, format!("{}→unicode", mode.short())))
        });
        match delivered {
            Ok((at, how)) => record(state, t.finish(at, started, &how)),
            Err(e) => warn!("unicode fallback died too: {e:#}"),
        }
        state.advance(Mode::Injecting, Mode::Idle);
    }
    Ok(())
}

// ── Paste ────────────────────────────────────────────────────────────────────

fn paste(text: &str, plain: bool, settle_ms: u64) -> Result<Instant> {
    let saved = clipboard_swap(text)?;
    nap(15);

    let chord = [VK_CONTROL].into_iter().chain(plain.then_some(VK_SHIFT));
    send_vk(&chord.clone().map(|k| (k, false))
        .chain([(VK_V, false), (VK_V, true)])
        .chain(chord.rev().map(|k| (k, true)))
        .collect::<Vec<_>>());
    let delivered = Instant::now();
    info!("{} {} chars", if plain { "paste-plain" } else { "paste" }, text.chars().count());

    // Give the target app time to actually READ the clipboard before we put
    // yours back. Too short and it pastes your old clipboard. Ask us how we know.
    nap(settle_ms);
    if let Some(old) = saved { clipboard_restore(&old); }
    Ok(delivered)
}

/// Set clipboard to `text`, retrying through lock contention (clipboard
/// managers, RDP, that one Electron app). Returns what was there before —
/// text only; if you had an image on the clipboard, it's gone. Sorry.
fn clipboard_swap(text: &str) -> Result<Option<String>> {
    (0..6u64)
        .find_map(|attempt| {
            let r = arboard::Clipboard::new().and_then(|mut cb| {
                let prev = cb.get_text().ok();
                cb.set_text(text).map(|_| prev)
            });
            r.inspect_err(|_| nap(15 * (attempt + 1))).ok()
        })
        .ok_or_else(|| anyhow!("clipboard still locked after 6 tries"))
}

fn clipboard_restore(text: &str) {
    let ok = (0..3).any(|_| {
        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)).is_ok() || { nap(15); false }
    });
    if !ok { warn!("couldn't restore your clipboard; it now holds the transcript"); }
}

// ── Unicode (SendInput) ──────────────────────────────────────────────────────

fn kbd(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: OUR_INJECTION },
        },
    }
}

fn send_vk(keys: &[(VIRTUAL_KEY, bool)]) {
    let evs: Vec<INPUT> = keys.iter()
        .map(|&(vk, up)| kbd(vk, 0, if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }))
        .collect();
    unsafe { SendInput(&evs, std::mem::size_of::<INPUT>() as i32); }
}

/// Every UTF-16 unit as a VK_PACKET down/up. Layout-independent, so "ação" and
/// "Größe" come out right on any keyboard. Doesn't cross Parsec/KVM: it's a
/// window message in a trench coat, not a keystroke.
fn unicode(text: &str, delay_ms: u64) -> Result<Instant> {
    // You might still be holding Ctrl from the hotkey. Ctrl+<packet> does
    // fun things in some apps. Let go of it for you.
    send_vk(&[(VK_LCONTROL, true), (VK_RCONTROL, true)]);

    let per_char = |u: u16| [kbd(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE),
                              kbd(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)];
    let send = |evs: &[INPUT]| unsafe { SendInput(evs, std::mem::size_of::<INPUT>() as i32) } as usize;

    let sent = if delay_ms == 0 {
        let evs: Vec<INPUT> = text.encode_utf16().flat_map(per_char).collect();
        send(&evs) / 2
    } else {
        text.encode_utf16()
            .map(|u| { let n = send(&per_char(u)); nap(delay_ms); n / 2 })
            .sum()
    };

    let delivered = Instant::now();
    let want = text.encode_utf16().count();
    info!("unicode {sent}/{want} units");
    (sent == want).then_some(delivered).ok_or_else(|| anyhow!("SendInput blocked after {sent}/{want} (UIPI? elevated target?)"))
}

// ── Type (Interception HID scancodes) ────────────────────────────────────────

/// Key-ups for every modifier, both sides. UP for a key that isn't down is a
/// no-op, so this is safe to spray. Runs before AND after a type run: a torn
/// previous run (or you, still holding Ctrl) must not turn "hello" into
/// Ctrl+H Ctrl+E Ctrl+L Ctrl+L Ctrl+O on the target.
const ALL_MODS_UP: [KeyStroke; 6] = [
    KeyStroke::up(sc::LSHIFT),
    KeyStroke::up(sc::RSHIFT),
    KeyStroke::up(sc::CTRL),
    KeyStroke::new(sc::CTRL, ic::KEY_UP | ic::KEY_E0), // right Ctrl
    KeyStroke::up(sc::ALT),
    KeyStroke::new(sc::ALT, ic::KEY_UP | ic::KEY_E0),  // right Alt / AltGr
];

/// One "event" of a type run: strokes to send, then how long to wait.
type Burst = (Vec<KeyStroke>, u64);

/// Pure: text → schedule of stroke bursts. Shift is only toggled when the
/// next char's shift state differs, with a settle after each toggle — over the
/// KVM hop a shift-down and key-down sent back-to-back can coalesce and arrive
/// in the wrong order, and then "Hello" is "hello". 2–8 ms fixes it.
fn plan(text: &str, delay_ms: u64) -> (Vec<Burst>, Vec<char>) {
    let settle = delay_ms.clamp(2, 8);
    let (bursts, dropped, shift) = keymap::asciify(text).chars().fold(
        (Vec::<Burst>::new(), Vec::new(), false),
        |(mut out, mut bad, shift), c| match keymap::scancode(c) {
            None => { bad.push(c); (out, bad, shift) }
            Some((code, want)) => {
                if want != shift {
                    let s = if want { KeyStroke::down(sc::LSHIFT) } else { KeyStroke::up(sc::LSHIFT) };
                    out.push((vec![s], settle));
                }
                out.push((vec![KeyStroke::down(code), KeyStroke::up(code)], delay_ms));
                (out, bad, want)
            }
        },
    );
    let mut bursts = bursts;
    if shift { bursts.push((vec![KeyStroke::up(sc::LSHIFT)], settle)); }
    (bursts, dropped)
}

fn hid_type(drv: &Driver, dev: ic::Device, text: &str, delay_ms: u64) -> Result<Instant> {
    let (bursts, dropped) = plan(text, delay_ms);

    drv.send(dev, &ALL_MODS_UP);
    nap(30);
    bursts.iter().for_each(|(strokes, wait)| {
        drv.send(dev, strokes);
        if *wait > 0 { nap(*wait) }
    });
    drv.send(dev, &ALL_MODS_UP);
    let delivered = Instant::now();
    nap(30); // let the last release clear the KVM before the next clip barges in

    if !dropped.is_empty() {
        let peek: String = dropped.iter().take(12).collect();
        warn!("type: {} chars have no US-QWERTY key, dropped: {peek:?} (use Unicode mode for these)",
            dropped.len());
    }
    info!("type {} chars via interception dev {dev}", text.chars().count() - dropped.len());
    Ok(delivered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(text: &str) -> Vec<KeyStroke> {
        plan(text, 0).0.into_iter().flat_map(|(s, _)| s).collect()
    }

    #[test]
    fn shift_toggles_only_on_change() {
        let s = flat("HEy");
        let shifts = s.iter().filter(|k| k.code == sc::LSHIFT).count();
        assert_eq!(shifts, 2); // down before H, up before y
    }

    #[test]
    fn trailing_shift_is_released() {
        let s = flat("OK");
        assert_eq!(s.last(), Some(&KeyStroke::up(sc::LSHIFT)));
    }

    #[test]
    fn non_ascii_dropped_and_reported() {
        let (b, bad) = plan("aç!", 0);
        assert_eq!(bad, vec!['ç']);
        // a, shift↓, !, shift↑
        assert_eq!(b.len(), 4);
    }

    #[test]
    fn curly_apostrophe_survives() {
        let (_, bad) = plan("it\u{2019}s", 0);
        assert!(bad.is_empty());
    }
}
