// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * inject_linux.rs
 *
 * Version: @(#)inject_linux.rs 0.3.2 2026
 *
 * Description: The Hand On The Penguin.
 *              Kernel make us a keyboard: uinput. Our keys come out of a real
 *              input device, so X11, Wayland, TTY all believe them. Paste = put
 *              words on clipboard, press Ctrl+V on our keyboard. Type = press
 *              every key. Nothing work? Words stay on clipboard. Never lost.
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

//   mode         mechanism                              charset
//   ───────────  ─────────────────────────────────────  ───────
//   Paste        clipboard + Ctrl+V on our uinput kbd   all
//   PastePlain   clipboard + Ctrl+Shift+V               all
//   Unicode      (none: see below) → Paste              all
//   Type         uinput key codes, one per character    US ASCII
//
// uinput is Linux's Interception: a keyboard the kernel creates for us, below
// X11 and Wayland, so every compositor believes it. Needs write access to
// /dev/uinput (the `input` group on most distros). It's created once, at
// thread start, and lives as long as we do: a fresh device takes a moment
// before the compositor listens to it.
//
// Unicode has no Linux equivalent. KEYEVENTF_UNICODE is a Windows message in
// a trench coat; uinput only speaks key codes, and the Wayland tools that fake
// it (wtype) only work on wlroots. Paste already covers every language.
//
// Degradation ladder (a transcript is never silently dropped):
//   Type / Unicode ── fails ──► Paste ── no uinput ──► left on the clipboard
//
// The clipboard object lives as long as this thread. On X11 and Wayland the
// app that copied is the one serving the paste: drop it and the text you
// "copied" goes with it.

use anyhow::{anyhow, bail, Result};
use crossbeam_channel::Receiver;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::config::InjectMode;
use crate::hotkey::OUR_DEVICE;
use crate::keymap::{self, sc};
use crate::perf::{Entry, Transcript};
use crate::state::{Mode, State};
use crate::ui;

fn nap(ms: u64) { std::thread::sleep(Duration::from_millis(ms)) }

/// File a finished benchmark and send the Perf page the fresh view.
pub fn record(state: &State, e: Entry) {
    info!("perf: total {} ms (asr {} · deliver {}){}", e.total_ms, e.asr_ms, e.inject_ms,
        if e.cold { " cold" } else { "" });
    let max = state.cfg.read().perf_max;
    state.perf.push(e, max);
    ui::push("perf", &crate::ipc::perf_view(state));
}

pub fn run(state: &State, texts: &Receiver<Transcript>) -> Result<()> {
    // Missing uinput is a fact of life, not an error: the clipboard still works.
    let kbd = Uinput::open()
        .inspect(|_| info!("uinput: online, Paste and Type available"))
        .inspect_err(|e| warn!("uinput: {e:#}. Transcripts will wait on the clipboard for you to paste"))
        .ok();
    state.interception.store(kbd.is_some(), std::sync::atomic::Ordering::Release);
    let mut clip = arboard::Clipboard::new().inspect_err(|e| warn!("clipboard: {e}")).ok();

    for t in texts.iter().take_while(|_| state.alive()) {
        let (mode, settle, delay) = {
            let c = state.cfg.read();
            (c.inject_mode, c.paste_settle_ms as u64, c.type_delay_ms as u64)
        };
        let text = &t.text;
        let started = Instant::now();

        let first = match (mode, kbd.as_ref()) {
            (_, None) => Err(anyhow!("no uinput keyboard")),
            (InjectMode::Paste, Some(k))      => paste(k, clip.as_mut(), text, false, settle),
            (InjectMode::PastePlain, Some(k)) => paste(k, clip.as_mut(), text, true, settle),
            (InjectMode::Unicode, Some(_))    => Err(anyhow!("Linux has no Unicode injection")),
            (InjectMode::Type, Some(k))       => type_ascii(k, text, delay),
        };
        let delivered = first.map(|at| (at, mode.short().to_owned())).or_else(|e| match (mode, kbd.as_ref()) {
            (InjectMode::Unicode | InjectMode::Type, Some(k)) => {
                warn!("{}: {e:#} — falling back to paste", mode.short());
                paste(k, clip.as_mut(), text, false, settle).map(|at| (at, format!("{}→paste", mode.short())))
            }
            _ => Err(e),
        });
        match delivered {
            Ok((at, how)) => record(state, t.finish(at, started, &how)),
            Err(e) => match clip.as_mut().map(|c| c.set_text(text.as_str())) {
                Some(Ok(())) => warn!("{}: {e:#} — the transcript is on your clipboard", mode.short()),
                _ => warn!("{}: {e:#} — and the clipboard is out too; the transcript is lost", mode.short()),
            },
        }
        state.advance(Mode::Injecting, Mode::Idle);
    }
    Ok(())
}

// ── Paste ────────────────────────────────────────────────────────────────────

fn paste(kbd: &Uinput, clip: Option<&mut arboard::Clipboard>, text: &str, plain: bool, settle_ms: u64) -> Result<Instant> {
    let clip = clip.ok_or_else(|| anyhow!("no clipboard (no X11 or Wayland display?)"))?;
    let saved = clip.get_text().ok();
    clip.set_text(text)?;
    nap(15);

    let chord: &[u16] = if plain { &[sc::CTRL, sc::LSHIFT] } else { &[sc::CTRL] };
    chord.iter().try_for_each(|&k| kbd.key(k, true))?;
    kbd.tap(KEY_V)?;
    chord.iter().rev().try_for_each(|&k| kbd.key(k, false))?;
    let delivered = Instant::now();
    info!("{} {} chars", if plain { "paste-plain" } else { "paste" }, text.chars().count());

    // Give the target app time to actually READ the clipboard before we put
    // yours back. Too short and it pastes your old clipboard.
    nap(settle_ms);
    if let Some(old) = saved {
        if clip.set_text(old).is_err() { warn!("couldn't restore your clipboard; it now holds the transcript"); }
    }
    Ok(delivered)
}

// ── Type ─────────────────────────────────────────────────────────────────────

const KEY_V: u16 = 47;

/// Pure: text → (key code, shift) per character, plus what a US keyboard
/// can't type. Linux key codes ARE the set-1 make codes keymap.rs knows.
fn plan(text: &str) -> (Vec<(u16, bool)>, Vec<char>) {
    keymap::asciify(text).chars().fold((Vec::new(), Vec::new()), |(mut keys, mut bad), c| {
        match keymap::scancode(c) { Some(k) => keys.push(k), None => bad.push(c) }
        (keys, bad)
    })
}

fn type_ascii(kbd: &Uinput, text: &str, delay_ms: u64) -> Result<Instant> {
    let (keys, dropped) = plan(text);
    if keys.is_empty() && !dropped.is_empty() { bail!("nothing a US keyboard can type"); }
    let mut shift = false;
    for &(code, want) in &keys {
        if want != shift { kbd.key(sc::LSHIFT, want)?; shift = want; nap(2); }
        kbd.tap(code)?;
        if delay_ms > 0 { nap(delay_ms) }
    }
    if shift { kbd.key(sc::LSHIFT, false)?; }
    let delivered = Instant::now();
    if !dropped.is_empty() {
        let peek: String = dropped.iter().take(12).collect();
        warn!("type: {} chars have no US-QWERTY key, dropped: {peek:?} (use Paste for these)", dropped.len());
    }
    info!("type {} chars via uinput", keys.len());
    Ok(delivered)
}

// ── uinput ───────────────────────────────────────────────────────────────────

/// Linux ioctl request numbers: dir << 30 | size << 16 | type << 8 | nr. Pure.
const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> u32 { dir << 30 | (size as u32) << 16 | (kind as u32) << 8 | nr as u32 }
const fn io(nr: u8) -> u32 { ioc(0, b'U', nr, 0) }
const fn iow(nr: u8, size: usize) -> u32 { ioc(1, b'U', nr, size) }

const UI_DEV_CREATE: u32 = io(1);
const UI_DEV_DESTROY: u32 = io(2);
const UI_DEV_SETUP: u32 = iow(3, std::mem::size_of::<UinputSetup>());
const UI_SET_EVBIT: u32 = iow(100, std::mem::size_of::<libc::c_int>());
const UI_SET_KEYBIT: u32 = iow(101, std::mem::size_of::<libc::c_int>());

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
const BUS_VIRTUAL: u16 = 0x06;

#[repr(C)]
struct InputId { bustype: u16, vendor: u16, product: u16, version: u16 }

#[repr(C)]
struct UinputSetup { id: InputId, name: [u8; 80], ff_effects_max: u32 }

#[repr(C)]
struct InputEvent { time: libc::timeval, kind: u16, code: u16, value: i32 }

struct Uinput { dev: File }

impl Uinput {
    /// Create the virtual keyboard: every key code 1..=255, then a pause so the
    /// compositor has picked it up before the first paste.
    fn open() -> Result<Self> {
        let dev = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/uinput")
            .map_err(|e| anyhow!("/dev/uinput: {e} (needs the `input` group, or a udev rule)"))?;
        let fd = dev.as_raw_fd();
        let ctl = |req: u32, arg: libc::c_ulong, what: &str| -> Result<()> {
            if unsafe { libc::ioctl(fd, req as _, arg) } < 0 { bail!("uinput {what}: {}", std::io::Error::last_os_error()); }
            Ok(())
        };
        ctl(UI_SET_EVBIT, EV_KEY as _, "EV_KEY")?;
        (1..=255).try_for_each(|k| ctl(UI_SET_KEYBIT, k, "keybit"))?;

        let mut name = [0u8; 80];
        name[..OUR_DEVICE.len()].copy_from_slice(OUR_DEVICE.as_bytes());
        let setup = UinputSetup {
            id: InputId { bustype: BUS_VIRTUAL, vendor: 0x4456, product: 0x4F58, version: 1 }, // "DVOX"
            name,
            ff_effects_max: 0,
        };
        ctl(UI_DEV_SETUP, &setup as *const UinputSetup as libc::c_ulong, "setup")?;
        ctl(UI_DEV_CREATE, 0, "create")?;
        nap(250);
        Ok(Self { dev })
    }

    fn emit(&self, kind: u16, code: u16, value: i32) -> Result<()> {
        let ev = InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind, code, value };
        let bytes = unsafe {
            std::slice::from_raw_parts((&ev as *const InputEvent).cast::<u8>(), std::mem::size_of::<InputEvent>())
        };
        (&self.dev).write_all(bytes).map_err(|e| anyhow!("uinput write: {e}"))
    }

    /// One key change, reported. A few ms after it so the compositor sees an
    /// order, not a smear.
    fn key(&self, code: u16, down: bool) -> Result<()> {
        self.emit(EV_KEY, code, down as i32)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        nap(2);
        Ok(())
    }

    fn tap(&self, code: u16) -> Result<()> { self.key(code, true)?; self.key(code, false) }
}

impl Drop for Uinput {
    fn drop(&mut self) { unsafe { libc::ioctl(self.dev.as_raw_fd(), UI_DEV_DESTROY as _); } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_linux_uinput_h() {
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_DESTROY, 0x5502);
        assert_eq!(UI_DEV_SETUP, 0x405C_5503);
        assert_eq!(UI_SET_EVBIT, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
        assert_eq!(std::mem::size_of::<UinputSetup>(), 92);
    }

    #[test]
    fn plan_types_ascii_and_reports_the_rest() {
        let (keys, bad) = plan("Hi, ação!");
        assert_eq!(keys[0], (0x23, true));  // H
        assert_eq!(keys[1], (0x17, false)); // i
        assert_eq!(bad, vec!['ç', 'ã']);
        assert_eq!(plan("it\u{2019}s").1, Vec::<char>::new()); // curly quote folded, not dropped
    }

    #[test]
    fn v_is_where_keymap_says() {
        assert_eq!(keymap::scancode('v'), Some((KEY_V, false)));
    }
}
