// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * hotkey.rs
 *
 * Version: @(#)hotkey.rs 0.5.0 2026
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
// The decision itself lives in chord.rs, shared with Linux. This file is the
// Windows ear: install the hook, pump messages, decode, `chord::feed()`.

use crossbeam_channel::Sender;
use std::time::Duration;
use tracing::info;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, PM_REMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::chord::{self, Verdict};
use crate::state::State;

pub use crate::chord::{cancel_learn, learn, on_learned, set_binding, Cmd, Learned};

/// Stamped into dwExtraInfo of every SendInput event WE generate, so the hook
/// can tell our own paste chord from your fingers. ASCII "DVOX".
pub const OUR_INJECTION: usize = 0x44_56_4F_58;

pub fn run(state: &State, tx: Sender<Cmd>) -> anyhow::Result<()> {
    chord::connect(tx)?;

    let hook = unsafe {
        let hmod = GetModuleHandleW(None)?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), HINSTANCE(hmod.0), 0)?
    };
    let (b, toggle) = chord::current();
    info!("hotkey armed: {b} ({})", if toggle { "toggle" } else { "hold" });

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
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);

    match chord::feed(kb.vkCode as u16, down) {
        Verdict::Pass    => pass(),
        Verdict::Swallow => LRESULT(1),
    }
}
