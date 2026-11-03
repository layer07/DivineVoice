use anyhow::{bail, Result};
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CallbackResult, CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation,
    CGEventTapOptions, CGEventTapPlacement, CGEventType,
};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{info, warn};

use crate::binding::{Binding, ALT, CTRL, SHIFT, WIN};
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd { Start, Stop }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Learned { Got(Binding), Cancelled }

#[derive(Clone, Copy)]
struct LiveBinding { binding: Binding, toggle: bool, armed: bool, ate: bool }

static LIVE: Mutex<LiveBinding> = parking_lot::const_mutex(LiveBinding {
    binding: Binding { mods: CTRL, vk: 0x20 }, toggle: false, armed: false, ate: false,
});
static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
static LEARNING: AtomicBool = AtomicBool::new(false);
static LEARN_CB: OnceLock<Box<dyn Fn(Learned) + Send + Sync>> = OnceLock::new();

const KEYCODE_FIELD: u32 = 9; // kCGKeyboardEventKeycode

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

pub fn set_binding(binding: Binding, toggle: bool) {
    *LIVE.lock() = LiveBinding { binding, toggle, armed: false, ate: false };
}

pub fn on_learned(f: impl Fn(Learned) + Send + Sync + 'static) {
    let _ = LEARN_CB.set(Box::new(f));
}

pub fn learn() { LEARNING.store(true, Ordering::Release) }

pub fn cancel_learn() {
    if LEARNING.swap(false, Ordering::AcqRel) {
        if let Some(callback) = LEARN_CB.get() { callback(Learned::Cancelled); }
    }
}

fn request_listen_access() -> bool {
    unsafe { if CGPreflightListenEventAccess() { true } else { CGRequestListenEventAccess() } }
}

pub fn run(state: &State, tx: Sender<Cmd>) -> Result<()> {
    TX.set(tx).map_err(|_| anyhow::anyhow!("hotkey::run called twice"))?;
    if !request_listen_access() {
        bail!("Input Monitoring permission is required for the global hotkey")
    }

    info!("macOS hotkey armed");
    CGEventTap::with_enabled(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![CGEventType::KeyDown, CGEventType::KeyUp],
        |_proxy, event_type, event| handle_event(event_type, event),
        || while state.alive() {
            let mode = unsafe { kCFRunLoopDefaultMode };
            CFRunLoop::run_in_mode(mode, Duration::from_millis(100), true);
        },
    )
    .map_err(|_| anyhow::anyhow!("could not create macOS event tap; grant Input Monitoring permission"))?;
    Ok(())
}

fn handle_event(event_type: CGEventType, event: &CGEvent) -> CallbackResult {
    if LEARNING.load(Ordering::Acquire) { return CallbackResult::Keep; }
    let Some(vk) = mac_key_to_windows(event.get_integer_value_field(KEYCODE_FIELD) as u16) else {
        return CallbackResult::Keep;
    };
    let mut live = LIVE.lock();
    if vk != live.binding.vk { return CallbackResult::Keep; }
    let chord = modifiers_match(live.binding.mods, event.get_flags());
    let down = matches!(event_type, CGEventType::KeyDown);

    if down && chord && !live.ate {
        live.ate = true;
        if live.toggle { live.armed = !live.armed; } else { live.armed = true; }
        send(if live.armed { Cmd::Start } else { Cmd::Stop });
        return CallbackResult::Drop;
    }
    if !down && live.ate {
        live.ate = false;
        if !live.toggle && live.armed {
            live.armed = false;
            send(Cmd::Stop);
        }
        return CallbackResult::Drop;
    }
    CallbackResult::Keep
}

fn send(command: Cmd) {
    if let Some(tx) = TX.get() {
        if tx.try_send(command).is_err() { warn!("hotkey: audio worker is unavailable"); }
    }
}

fn modifiers_match(want: u8, flags: CGEventFlags) -> bool {
    let has = |bit, flag| want & bit == 0 || flags.contains(flag);
    has(CTRL, CGEventFlags::CGEventFlagControl)
        && has(SHIFT, CGEventFlags::CGEventFlagShift)
        && has(ALT, CGEventFlags::CGEventFlagAlternate)
        && has(WIN, CGEventFlags::CGEventFlagCommand)
}

/// Config bindings use Windows virtual-key values. This maps portable letter,
/// navigation, and function bindings. Unsupported macOS-only keys pass through.
fn mac_key_to_windows(key: u16) -> Option<u16> {
    let letter = [
        (0x00, b'A'), (0x0B, b'B'), (0x08, b'C'), (0x02, b'D'), (0x0E, b'E'),
        (0x03, b'F'), (0x05, b'G'), (0x04, b'H'), (0x22, b'I'), (0x26, b'J'),
        (0x28, b'K'), (0x25, b'L'), (0x2E, b'M'), (0x2D, b'N'), (0x1F, b'O'),
        (0x23, b'P'), (0x0C, b'Q'), (0x0F, b'R'), (0x01, b'S'), (0x11, b'T'),
        (0x20, b'U'), (0x09, b'V'), (0x0D, b'W'), (0x07, b'X'), (0x10, b'Y'),
        (0x06, b'Z'),
    ];
    letter.iter().find_map(|&(mac, ascii)| (key == mac).then_some(ascii as u16)).or_else(|| match key {
        0x31 => Some(0x20), 0x35 => Some(0x1B), 0x24 => Some(0x0D), 0x30 => Some(0x09),
        0x7B => Some(0x25), 0x7C => Some(0x27), 0x7D => Some(0x28), 0x7E => Some(0x26),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_default_hotkey() { assert_eq!(mac_key_to_windows(0x31), Some(0x20)); }

    #[test]
    fn maps_letters_and_navigation_keys() {
        assert_eq!(mac_key_to_windows(0x00), Some(u16::from(b'A')));
        assert_eq!(mac_key_to_windows(0x7B), Some(0x25));
        assert_eq!(mac_key_to_windows(0x55), None);
    }
}
