// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * hotkey_linux.rs
 *
 * Version: @(#)hotkey_linux.rs 0.3.2 2026
 *
 * Description: The Ear On The Penguin.
 *              No hook on Linux. X11 have one, Wayland say no, TTY say what. So go
 *              under all of them: read keyboard from kernel, /dev/input. Need
 *              `input` group, nothing more. Can hear, can't swallow: swallow mean
 *              grab whole keyboard, and that the old hostage filter again. No.
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

// Linux has no WH_KEYBOARD_LL. X11 has XRecord, Wayland gives apps nothing on
// purpose, and the GlobalShortcuts portal isn't everywhere yet. What works
// under all of them, and on a bare TTY: read the keyboards straight from the
// kernel, /dev/input/event*. Needs the `input` group and nothing else.
//
// What it can't do is swallow. Taking a key away from everyone else means
// EVIOCGRAB on the whole keyboard and re-injecting every other key ourselves,
// which is the Interception capture filter all over again (hotkey.rs has the
// scars). So on Linux the trigger still reaches the focused app: bind a key
// that does nothing on its own. F13–F24, Pause, a lone RCtrl.
//
// Shape:
//   keyboards()   pure: /proc/bus/input/devices → event nodes that are keyboards
//   vk_of()       pure: evdev key code → Windows VK (what bindings speak)
//   run()         poll them, decode input_event, chord::feed()
//
// Our own uinput keyboard (inject_linux.rs) is a keyboard too. It's skipped by
// name: the Linux twin of OUR_INJECTION.

use crossbeam_channel::Sender;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::chord;
use crate::keymap;
use crate::state::State;

pub use crate::chord::{cancel_learn, learn, on_learned, set_binding, Cmd, Learned};

/// What inject_linux.rs names its uinput keyboard. Never listened to.
pub const OUR_DEVICE: &str = "DivineVoice virtual keyboard";

const EV_KEY: u16 = 1;
/// Keyboards come and go (KVM, replug, an ESP32 rebooting): look again this often.
const RESCAN: Duration = Duration::from_secs(2);

/// struct input_event: a timeval, then type, code, value. 24 bytes on 64-bit.
#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent { time: libc::timeval, kind: u16, code: u16, value: i32 }

// ── the pure part: which devices are keyboards ───────────────────────────────

/// Event nodes ("event13") of every keyboard in /proc/bus/input/devices, minus
/// the one named `skip`. "Keyboard" = the kbd handler AND the letter keys:
/// power buttons, lid switches and webcams carry `kbd` too, with three keys. Pure.
pub fn keyboards(proc: &str, skip: &str) -> Vec<String> {
    proc.split("\n\n").filter_map(|block| {
        let field = |p: &str| block.lines().find_map(|l| l.strip_prefix(p));
        let name = field("N: Name=")?.trim().trim_matches('"');
        let handlers = field("H: Handlers=")?;
        let keys = field("B: KEY=")?;
        let has_letters = [30u16, 44, 57].iter().all(|&k| key_bit(keys, k)); // A, Z, Space
        let event = handlers.split_whitespace().find(|h| h.starts_with("event"))?;
        (name != skip && handlers.split_whitespace().any(|h| h == "kbd") && has_letters).then(|| event.to_owned())
    }).collect()
}

/// Bit `code` of a /proc KEY= bitmap: hex longs, most significant first. Pure.
fn key_bit(bitmap: &str, code: u16) -> bool {
    let bits = usize::BITS as usize;
    let words: Vec<&str> = bitmap.split_whitespace().collect();
    let (word, bit) = (code as usize / bits, code as usize % bits);
    words.len().checked_sub(word + 1)
        .and_then(|i| usize::from_str_radix(words[i], 16).ok())
        .is_some_and(|w| w >> bit & 1 == 1)
}

// ── the pure part: evdev code → Windows VK ───────────────────────────────────

/// Keys that aren't on keymap.rs's typing rows. Linux key codes are the PC/AT
/// set-1 make codes for the classic keys, so the rows give us the rest free.
const NAMED: &[(u16, u16)] = &[
    (1, 0x1B), (14, 0x08), (15, 0x09), (28, 0x0D), (57, 0x20), (58, 0x14),        // Esc BkSp Tab Enter Space Caps
    (29, 0xA2), (97, 0xA3), (42, 0xA0), (54, 0xA1), (56, 0xA4), (100, 0xA5),      // L/R Ctrl, Shift, Alt
    (125, 0x5B), (126, 0x5C), (127, 0x5D),                                        // LWin RWin Menu
    (87, 0x7A), (88, 0x7B),                                                       // F11 F12
    (119, 0x13), (70, 0x91), (69, 0x90), (99, 0x2C),                              // Pause ScrLk NumLk PrtSc
    (110, 0x2D), (111, 0x2E), (102, 0x24), (107, 0x23), (104, 0x21), (109, 0x22), // Ins Del Home End PgUp PgDn
    (103, 0x26), (105, 0x25), (106, 0x27), (108, 0x28),                           // arrows
    (82, 0x60), (79, 0x61), (80, 0x62), (81, 0x63), (75, 0x64),                   // Num0–Num4
    (76, 0x65), (77, 0x66), (71, 0x67), (72, 0x68), (73, 0x69),                   // Num5–Num9
    (55, 0x6A), (78, 0x6B), (74, 0x6D), (83, 0x6E), (98, 0x6F), (96, 0x0D),       // Num* + - . / Enter
    (113, 0xAD), (114, 0xAE), (115, 0xAF),                                        // Mute VolDown VolUp
    (163, 0xB0), (165, 0xB1), (166, 0xB2), (164, 0xB3),                           // Next Prev Stop PlayPause
];

/// US-layout punctuation: the character on the key → its VK_OEM_*.
const OEM: &[(char, u16)] = &[
    (';', 0xBA), ('=', 0xBB), (',', 0xBC), ('-', 0xBD), ('.', 0xBE), ('/', 0xBF),
    ('`', 0xC0), ('[', 0xDB), ('\\', 0xDC), (']', 0xDD), ('\'', 0xDE),
];

/// The Windows VK for a Linux key code, or None for keys bindings can't name. Pure.
pub fn vk_of(code: u16) -> Option<u16> {
    let function = match code {
        59..=68 => Some(0x70 + code - 59),   // F1–F10
        183..=194 => Some(0x7C + code - 183), // F13–F24
        _ => None,
    };
    let typed = || {
        let on_key = |c: char| keymap::scancode(c).is_some_and(|(sc, _)| sc == code);
        ('a'..='z').find(|&c| on_key(c)).map(|c| c.to_ascii_uppercase() as u16)
            .or_else(|| ('0'..='9').find(|&c| on_key(c)).map(|c| c as u16))
            .or_else(|| OEM.iter().find(|&&(c, _)| on_key(c)).map(|&(_, vk)| vk))
    };
    function
        .or_else(|| NAMED.iter().find(|&&(c, _)| c == code).map(|&(_, vk)| vk))
        .or_else(typed)
}

// ── the impure glue ──────────────────────────────────────────────────────────

pub fn run(state: &State, tx: Sender<Cmd>) -> anyhow::Result<()> {
    chord::connect(tx)?;
    let (b, toggle) = chord::current();
    info!("hotkey armed: {b} ({}), evdev, keys pass through", if toggle { "toggle" } else { "hold" });

    let mut open: BTreeMap<String, File> = BTreeMap::new();
    let mut scanned = Instant::now() - RESCAN;
    let mut warned = false;

    while state.alive() {
        if scanned.elapsed() >= RESCAN {
            scanned = Instant::now();
            rescan(&mut open);
            if open.is_empty() && !warned {
                warn!("hotkey: no readable keyboard in /dev/input. Is this user in the `input` group?");
            }
            warned = open.is_empty();
        }

        // poll(2) on every keyboard: no busy loop, and a press wakes us at once.
        let mut fds: Vec<libc::pollfd> = open.values()
            .map(|f| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 })
            .collect();
        if fds.is_empty() { std::thread::sleep(Duration::from_millis(250)); continue; }
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 250) };
        if ready <= 0 { continue; }

        let mut gone = Vec::new();
        for ((node, file), fd) in open.iter_mut().zip(&fds) {
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 { gone.push(node.clone()); continue; }
            if fd.revents & libc::POLLIN != 0 && drain(file).is_err() { gone.push(node.clone()); }
        }
        if !gone.is_empty() {
            gone.iter().for_each(|n| { open.remove(n); info!("hotkey: {n} unplugged"); });
            release_modifiers(); // a Ctrl held on a keyboard that vanished can't stay held forever
        }
    }
    info!("hotkey disarmed");
    Ok(())
}

/// Open keyboards we haven't got yet; forget nodes that stopped being keyboards.
fn rescan(open: &mut BTreeMap<String, File>) {
    let proc = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    let now = keyboards(&proc, OUR_DEVICE);
    open.retain(|node, _| now.contains(node));
    let new: Vec<String> = now.into_iter().filter(|n| !open.contains_key(n)).collect();
    for node in new {
        let path = format!("/dev/input/{node}");
        match OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path) {
            Ok(f) => { info!("hotkey: listening on {path}"); open.insert(node, f); }
            Err(e) => debug!("hotkey: {path}: {e}"),
        }
    }
}

/// Read everything a device has queued and feed the key events. Err = the
/// device is gone. Autorepeat (value 2) is a key-down, as on Windows.
fn drain(file: &mut File) -> std::io::Result<()> {
    const N: usize = std::mem::size_of::<InputEvent>();
    let mut buf = [0u8; N * 64];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };
        for chunk in buf[..n].chunks_exact(N) {
            let ev: InputEvent = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
            if ev.kind != EV_KEY { continue; }
            if let Some(vk) = vk_of(ev.code) { let _ = chord::feed(vk, ev.value != 0); }
        }
    }
}

fn release_modifiers() {
    [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C].into_iter().for_each(|vk| { let _ = chord::feed(vk, false); });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lifted from the NUC: a power button, a USB audio device's volume keys,
    // the ESP32 posing as a Dell KB216, and our own uinput keyboard.
    const PROC: &str = "\
I: Bus=0019 Vendor=0000 Product=0001 Version=0000
N: Name=\"Power Button\"
H: Handlers=kbd event0
B: EV=3
B: KEY=10000000000000 0

I: Bus=0003 Vendor=0d8c Product=0014 Version=0100
N: Name=\"C-Media Electronics Inc. USB Audio Device\"
H: Handlers=kbd event5
B: EV=13
B: KEY=e000000000000 0

I: Bus=0003 Vendor=413c Product=2113 Version=0110
N: Name=\"Dell Dell KB216 Wired Keyboard\"
H: Handlers=sysrq kbd leds event13
B: EV=120013
B: KEY=1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe

I: Bus=0006 Vendor=4456 Product=4f58 Version=0001
N: Name=\"DivineVoice virtual keyboard\"
H: Handlers=sysrq kbd event20
B: EV=3
B: KEY=ffffffffffffffff ffffffffffffffff ffffffffffffffff fffffffffffffffe
";

    #[test]
    fn only_real_keyboards_and_never_ourselves() {
        assert_eq!(keyboards(PROC, OUR_DEVICE), vec!["event13"]);
        assert_eq!(keyboards(PROC, "nobody"), vec!["event13", "event20"]);
    }

    #[test]
    fn bitmap_reads_from_the_right_end() {
        assert!(key_bit("fffffffffffffffe", 30));
        assert!(!key_bit("fffffffffffffffe", 0));
        assert!(key_bit("10000000000000 0", 116));  // power button: KEY_POWER
        assert!(!key_bit("10000000000000 0", 30));
        assert!(!key_bit("", 30));
    }

    #[test]
    fn evdev_codes_become_the_vks_bindings_speak() {
        assert_eq!(vk_of(57), Some(0x20));          // Space
        assert_eq!(vk_of(29), Some(0xA2));          // LCtrl
        assert_eq!(vk_of(97), Some(0xA3));          // RCtrl
        assert_eq!(vk_of(30), Some(u16::from(b'A')));
        assert_eq!(vk_of(11), Some(u16::from(b'0')));
        assert_eq!(vk_of(41), Some(0xC0));          // `
        assert_eq!(vk_of(59), Some(0x70));          // F1
        assert_eq!(vk_of(183), Some(0x7C));         // F13
        assert_eq!(vk_of(194), Some(0x87));         // F24
        assert_eq!(vk_of(119), Some(0x13));         // Pause
        assert_eq!(vk_of(0x2FF), None);
    }

    #[test]
    fn the_default_and_documented_bindings_are_reachable() {
        let reachable = |s: &str| {
            let b: crate::binding::Binding = s.parse().unwrap();
            (1..=255u16).any(|c| vk_of(c) == Some(b.vk))
        };
        for s in ["Ctrl+Space", "Ctrl+Shift+F13", "RAlt", "RCtrl", "Pause", "F24", "Alt+Win+`"] {
            assert!(reachable(s), "{s} can't be pressed on Linux");
        }
    }
}
