// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * hotkey.rs
 *
 * Version: @(#)hotkey.rs 0.3.2 2026
 *
 * Description: The Ear On The Keyboard.
 *              WH_KEYBOARD_LL, not a filter driver. Old capture filter hold every
 *              key hostage: one stall, keyboard dead in Windows, alive in GRUB.
 *              Hook can't do that. Stall, Windows skip us. Decision is pure
 *              step(): state in, verdict out. Three atomics. Hook proc too dumb
 *              to have bugs.
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

// Hard-won lesson, written in blood: the ancestor of this program did hotkey
// detection with an Interception *capture* filter. Every keystroke on the box
// took a detour through our userland loop and had to be re-injected by hand.
// One stall and the keyboard is dead — dead in Windows, alive in GRUB, user
// questioning their life choices.
//
// A low-level hook cannot do that to you. If our callback dawdles, Windows
// shrugs after LowLevelHooksTimeout (~300 ms), skips us, and delivers the key
// anyway. Worst case: one hiccup. Never a hostage situation.
//
// Shape of this file:
//   step()        pure: (state, binding, key event) → (state', pass/swallow, cmd?)
//   learn_step()  pure: the "press your new hotkey" capture, same idea
//   ll_proc()     the only impure bit: decode → step → store → act
//
// All mutable state is three atomics. The hook proc is a bare extern fn with
// no user pointer, so statics it is. One hook, one thread, no regrets.

use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{info, warn};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, PM_REMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::binding::{mod_of, Binding, VK_ESCAPE};
use crate::state::State;

/// Stamped into dwExtraInfo of every SendInput event WE generate, so the hook
/// can tell our own paste chord from your fingers. ASCII "DVOX".
pub const OUR_INJECTION: usize = 0x44_56_4F_58;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd { Start, Stop }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict { Pass, Swallow }

// ── the pure part: hotkey ────────────────────────────────────────────────────

/// held: modifier groups down · armed: recording · ate: we swallowed a trigger
/// key-down, so we owe the system a swallowed key-up too (and autorepeats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hk { pub held: u8, pub armed: bool, pub ate: bool }

impl Hk {
    const fn pack(self) -> u8 { self.held & 0x0F | (self.armed as u8) << 4 | (self.ate as u8) << 5 }
    const fn unpack(b: u8) -> Self { Self { held: b & 0x0F, armed: b & 0x10 != 0, ate: b & 0x20 != 0 } }
}

const fn chord(held: u8, want: u8) -> bool { held & want == want }

/// The entire hotkey, as a transition function.
///
///   HOLD:   trigger down with mods held → Start (swallow) ·· trigger up → Stop
///           releasing a required modifier mid-recording → Stop
///   TOGGLE: trigger down with mods held → Start if idle, Stop if recording
///
/// The trigger key is swallowed (down, autorepeats, up). Modifiers NEVER are:
/// eating Ctrl is how you get "my Ctrl key is stuck" bug reports from people
/// who will never forgive you.
pub fn step(s: Hk, b: Binding, toggle: bool, vk: u16, down: bool) -> (Hk, Verdict, Option<Cmd>) {
    use {Cmd::*, Verdict::*};

    if vk == b.vk {
        return match (down, toggle) {
            (true, _) if s.ate => (s, Swallow, None), // OS autorepeat. Shh.
            (true, false) if chord(s.held, b.mods) =>
                (Hk { armed: true, ate: true, ..s }, Swallow, Some(Start)),
            (true, true) if chord(s.held, b.mods) =>
                (Hk { armed: !s.armed, ate: true, ..s }, Swallow, Some(if s.armed { Stop } else { Start })),
            (true, _) => (s, Pass, None),

            (false, false) if s.ate => (Hk { armed: false, ate: false, ..s }, Swallow, s.armed.then_some(Stop)),
            (false, true)  if s.ate => (Hk { ate: false, ..s }, Swallow, None),
            (false, _) => (s, Pass, None),
        };
    }

    match mod_of(vk) {
        0 => (s, Pass, None),
        m => {
            let held = if down { s.held | m } else { s.held & !m };
            let broke = !toggle && s.armed && !chord(held, b.mods);
            (Hk { held, armed: s.armed && !broke, ..s }, Pass, broke.then_some(Stop))
        }
    }
}

// ── the pure part: learning a new binding ────────────────────────────────────

/// held: modifier groups down · lone: the last modifier pressed with nothing
/// else since (so tapping RAlt by itself binds "RAlt"), 0 = none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Learn { pub held: u8, pub lone: u16 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Learned { Got(Binding), Cancelled }

impl Learn {
    const fn pack(self) -> u32 { 1 << 31 | (self.held as u32) << 16 | self.lone as u32 }
    const fn unpack(x: u32) -> Self { Self { held: (x >> 16) as u8 & 0x0F, lone: x as u16 } }
}

pub fn learn_step(l: Learn, vk: u16, down: bool) -> (Learn, Option<Learned>) {
    match (mod_of(vk), down) {
        (0, true) if vk == VK_ESCAPE && l.held == 0 => (l, Some(Learned::Cancelled)),
        (0, true)  => (l, Some(Learned::Got(Binding { mods: l.held, vk }))),
        (0, false) => (l, None),
        (m, true)  => (Learn { held: l.held | m, lone: vk }, None),
        (m, false) if l.lone == vk => (l, Some(Learned::Got(Binding { mods: l.held & !m, vk }))),
        (m, false) => (Learn { held: l.held & !m, lone: 0 }, None),
    }
}

// ── the impure glue ──────────────────────────────────────────────────────────

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
static HK: AtomicU8 = AtomicU8::new(0);
static BIND: AtomicU32 = AtomicU32::new(Binding { mods: crate::binding::CTRL, vk: 0x20 }.pack());
static TOGGLE: AtomicBool = AtomicBool::new(false);
/// Bit 31 set = learning. Low bits = packed `Learn`.
static LEARN: AtomicU32 = AtomicU32::new(0);
static LEARN_CB: OnceLock<Box<dyn Fn(Learned) + Send + Sync>> = OnceLock::new();

/// Swap the live binding. Takes effect on the very next keystroke; resets the
/// machine so a half-pressed old chord can't leave us armed forever.
pub fn set_binding(b: Binding, toggle: bool) {
    BIND.store(b.pack(), Relaxed);
    TOGGLE.store(toggle, Relaxed);
    HK.store(Hk { held: Hk::unpack(HK.load(Relaxed)).held, ..Hk::default() }.pack(), Relaxed);
}

/// Where learned bindings go (the UI). Set once at startup.
pub fn on_learned(f: impl Fn(Learned) + Send + Sync + 'static) {
    let _ = LEARN_CB.set(Box::new(f));
}

/// Start capturing the next chord. While learning, the hotkey is inert and
/// nothing is swallowed except the capturing key-down itself.
pub fn learn() { LEARN.store(Learn::default().pack(), Relaxed) }
pub fn cancel_learn() { LEARN.store(0, Relaxed) }

pub fn run(state: &State, tx: Sender<Cmd>) -> anyhow::Result<()> {
    TX.set(tx).map_err(|_| anyhow::anyhow!("hotkey::run called twice"))?;

    let hook = unsafe {
        let hmod = GetModuleHandleW(None)?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), HINSTANCE(hmod.0), 0)?
    };
    info!("hotkey armed: {} ({})", Binding::unpack(BIND.load(Relaxed)),
        if TOGGLE.load(Relaxed) { "toggle" } else { "hold" });

    // The hook only fires while this thread pumps messages. 5 ms naps keep CPU
    // at ~0 and stay two orders of magnitude under the hook timeout.
    while state.alive() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    unsafe { let _ = UnhookWindowsHookEx(hook); }
    info!("hotkey disarmed");
    Ok(())
}

unsafe extern "system" fn ll_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pass = || CallNextHookEx(None, code, wparam, lparam);
    if code < 0 { return pass(); } // per MSDN: not ours to look at

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if kb.dwExtraInfo == OUR_INJECTION { return pass(); }
    let vk = kb.vkCode as u16;
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);

    // Learning a new binding? Then the hotkey is off duty.
    let learning = LEARN.load(Relaxed);
    if learning >> 31 == 1 {
        let (next, out) = learn_step(Learn::unpack(learning), vk, down);
        return match out {
            None => { LEARN.store(next.pack(), Relaxed); pass() }
            Some(l) => {
                LEARN.store(0, Relaxed);
                let swallow = matches!(l, Learned::Got(_)) && down;
                if let Some(cb) = LEARN_CB.get() { cb(l); }
                if swallow { LRESULT(1) } else { pass() }
            }
        };
    }

    let b = Binding::unpack(BIND.load(Relaxed));
    let (next, verdict, cmd) = step(Hk::unpack(HK.load(Relaxed)), b, TOGGLE.load(Relaxed), vk, down);
    HK.store(next.pack(), Relaxed);

    // No logging on the happy path: the hook has a deadline, and a log write
    // is I/O. The audio thread logs Start/Stop when it gets them.
    if let (Some(c), Some(tx)) = (cmd, TX.get()) {
        if tx.try_send(c).is_err() { warn!("hotkey: audio thread gone"); }
    }

    match verdict {
        Verdict::Pass    => pass(),
        Verdict::Swallow => LRESULT(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCTRL: u16 = 0xA2;
    const RCTRL: u16 = 0xA3;
    const SHIFT: u16 = 0xA0;
    const SPACE: u16 = 0x20;
    const RALT: u16 = 0xA5;

    fn ctrl_space() -> Binding { "Ctrl+Space".parse().unwrap() }

    /// Feed (vk, down) pairs, collect verdicts and commands.
    fn run(b: Binding, toggle: bool, keys: &[(u16, bool)]) -> (Hk, Vec<Verdict>, Vec<Cmd>) {
        keys.iter().fold((Hk::default(), vec![], vec![]), |(s, mut v, mut c), &(vk, d)| {
            let (n, verdict, cmd) = step(s, b, toggle, vk, d);
            v.push(verdict);
            c.extend(cmd);
            (n, v, c)
        })
    }

    use Verdict::{Pass as P, Swallow as S};

    #[test]
    fn hold_classic() {
        let (s, v, c) = run(ctrl_space(), false,
            &[(LCTRL, true), (SPACE, true), (SPACE, true), (SPACE, false), (LCTRL, false)]);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]);
        assert_eq!(v, vec![P, S, S, S, P]);
        assert_eq!(s, Hk::default());
    }

    #[test]
    fn hold_release_ctrl_first() {
        let (s, v, c) = run(ctrl_space(), false,
            &[(LCTRL, true), (SPACE, true), (LCTRL, false), (SPACE, false)]);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]); // exactly one Stop
        assert_eq!(v, vec![P, S, P, S]);              // trailing Space-up still eaten
        assert_eq!(s, Hk::default());
    }

    #[test]
    fn right_ctrl_counts_as_ctrl() {
        let (_, _, c) = run(ctrl_space(), false, &[(RCTRL, true), (SPACE, true), (SPACE, false)]);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]);
    }

    #[test]
    fn plain_space_untouched() {
        let (_, v, c) = run(ctrl_space(), false, &[(SPACE, true), (SPACE, false)]);
        assert!(c.is_empty());
        assert_eq!(v, vec![P, P]);
    }

    #[test]
    fn extra_modifiers_still_fire() {
        let (_, _, c) = run(ctrl_space(), false, &[(LCTRL, true), (SHIFT, true), (SPACE, true), (SPACE, false)]);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]);
    }

    #[test]
    fn toggle_mode() {
        let tap = [(LCTRL, true), (SPACE, true), (SPACE, false), (LCTRL, false)];
        let keys: Vec<_> = tap.iter().chain(&tap).copied().collect();
        let (s, v, c) = run(ctrl_space(), true, &keys);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]);
        assert_eq!(v, vec![P, S, S, P, P, S, S, P]);
        assert_eq!(s, Hk::default());
    }

    #[test]
    fn toggle_ignores_modifier_release() {
        let (s, _, c) = run(ctrl_space(), true, &[(LCTRL, true), (SPACE, true), (SPACE, false), (LCTRL, false)]);
        assert_eq!(c, vec![Cmd::Start]);
        assert!(s.armed); // still recording until the next tap
    }

    #[test]
    fn lone_modifier_trigger() {
        let b: Binding = "RAlt".parse().unwrap();
        let (_, v, c) = run(b, false, &[(RALT, true), (RALT, true), (RALT, false)]);
        assert_eq!(c, vec![Cmd::Start, Cmd::Stop]);
        assert_eq!(v, vec![S, S, S]);
    }

    #[test]
    fn modifiers_never_swallowed() {
        let (_, v, _) = run(ctrl_space(), false,
            &[(LCTRL, true), (SPACE, true), (LCTRL, false), (LCTRL, true), (SPACE, false), (LCTRL, false)]);
        assert_eq!([v[0], v[2], v[3], v[5]], [P, P, P, P]);
    }

    #[test]
    fn packs_roundtrip() {
        (0u8..64).for_each(|b| assert_eq!(Hk::unpack(b).pack(), b));
        let l = Learn { held: 0b1010, lone: 0xA5 };
        assert_eq!(Learn::unpack(l.pack()), l);
    }

    fn learn(keys: &[(u16, bool)]) -> Option<Learned> {
        keys.iter().try_fold(Learn::default(), |l, &(vk, d)| match learn_step(l, vk, d) {
            (n, None) => Ok(n),
            (_, Some(out)) => Err(out),
        }).err()
    }

    #[test]
    fn learn_chord() {
        assert_eq!(learn(&[(LCTRL, true), (SHIFT, true), (0x7C, true)]),
            Some(Learned::Got("Ctrl+Shift+F13".parse().unwrap())));
    }

    #[test]
    fn learn_lone_modifier() {
        assert_eq!(learn(&[(RALT, true), (RALT, false)]), Some(Learned::Got("RAlt".parse().unwrap())));
    }

    #[test]
    fn learn_escape_cancels() {
        assert_eq!(learn(&[(VK_ESCAPE, true)]), Some(Learned::Cancelled));
    }

    #[test]
    fn learn_modifier_on_modifier() {
        // Hold Ctrl, tap LShift → "Ctrl+LShift". Weird, but legal, and it's
        // exactly what the user's fingers said.
        assert_eq!(learn(&[(LCTRL, true), (SHIFT, true), (SHIFT, false)]),
            Some(Learned::Got(Binding { mods: crate::binding::CTRL, vk: SHIFT })));
    }
}
