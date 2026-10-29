// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * interception.rs
 *
 * Version: @(#)interception.rs 0.3.0 2026
 *
 * Description: The Hand-Rolled Driver Bridge.
 *              LoadLibrary, four GetProcAddress, done. No import lib: crate
 *              version kill the process before main() if the DLL missing. Here,
 *              DLL missing = Type mode fall back, app keep living. Keystroke go
 *              below user mode, look like finger on plastic.
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

// Interception (oblitum/Interception) is a kernel keyboard/mouse filter driver.
// Strokes we `send` through it enter the input stack BELOW user mode, so to
// everything above — Windows, Parsec, a KVM capture loop — they are
// indistinguishable from a finger hitting plastic. That's the whole reason Type
// mode exists.
//
// Why not the `interception` crate: it import-links the DLL, so the PE loader
// kills the process before main() if interception.dll isn't on the search
// path. For a tool where Interception is an *optional* extra, that's backwards.
// So: LoadLibraryW + GetProcAddress, four function pointers, done. If any step
// fails, `Driver::open()` returns Err and the caller falls back to SendInput.
//
// ABI notes, straight from interception.h:
//   - InterceptionContext is an opaque void*.
//   - Devices 1..=10 are keyboards, 11..=20 are mice.
//   - interception_send() takes `const InterceptionStroke*` but, for keyboard
//     devices, walks the buffer as InterceptionKeyStroke[n] — 8-byte stride.
//     So we hand it an array of KeyStroke. Yes, really. C was a mistake, and
//     we all love it anyway.

use anyhow::{anyhow, Result};
use std::ffi::c_void;
use windows::core::{s, w, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

pub type Device = i32;
pub const KEYBOARDS: std::ops::RangeInclusive<Device> = 1..=10;

// InterceptionKeyState bits.
pub const KEY_DOWN: u16 = 0x00;
pub const KEY_UP:   u16 = 0x01;
pub const KEY_E0:   u16 = 0x02;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    pub code:        u16,
    pub state:       u16,
    pub information: u32,
}
const _: () = assert!(std::mem::size_of::<KeyStroke>() == 8);

impl KeyStroke {
    pub const fn new(code: u16, state: u16) -> Self { Self { code, state, information: 0 } }
    pub const fn down(code: u16) -> Self { Self::new(code, KEY_DOWN) }
    pub const fn up(code: u16)   -> Self { Self::new(code, KEY_UP) }
}

type Ctx = *mut c_void;
type FnCreate  = unsafe extern "C" fn() -> Ctx;
type FnDestroy = unsafe extern "C" fn(Ctx);
type FnSend    = unsafe extern "C" fn(Ctx, Device, *const KeyStroke, u32) -> i32;
type FnHwid    = unsafe extern "C" fn(Ctx, Device, *mut c_void, u32) -> u32;

pub struct Driver {
    ctx:     Ctx,
    destroy: FnDestroy,
    send:    FnSend,
    hwid:    FnHwid,
}

// The context is a bag of device HANDLEs; DeviceIoControl on them is
// thread-safe. We still only ever use a Driver from the inject thread.
unsafe impl Send for Driver {}

impl Driver {
    /// Load interception.dll (exe dir, then the usual DLL search path) and open
    /// a context. Err = DLL missing, or DLL present but driver not installed.
    pub fn open() -> Result<Self> {
        unsafe {
            let dll = LoadLibraryW(w!("interception.dll"))
                .map_err(|_| anyhow!("interception.dll not found (put it next to the exe)"))?;
            let create: FnCreate = sym(dll, s!("interception_create_context"))?;
            let destroy: FnDestroy = sym(dll, s!("interception_destroy_context"))?;
            let send: FnSend = sym(dll, s!("interception_send"))?;
            let hwid: FnHwid = sym(dll, s!("interception_get_hardware_id"))?;

            // The DLL is never FreeLibrary'd: it lives as long as the process,
            // which is exactly as long as anyone could want it.
            let ctx = create();
            if ctx.is_null() {
                return Err(anyhow!("interception driver not installed (DLL loaded, no context)"));
            }
            Ok(Self { ctx, destroy, send, hwid })
        }
    }

    /// Fire strokes at `dev`, in order, one ioctl. Returns how many landed.
    pub fn send(&self, dev: Device, strokes: &[KeyStroke]) -> usize {
        if strokes.is_empty() { return 0; }
        let n = unsafe { (self.send)(self.ctx, dev, strokes.as_ptr(), strokes.len() as u32) };
        n.max(0) as usize
    }

    /// Upper-cased hardware id ("HID\VID_046D&PID_C33F&..."), "" if none.
    /// The driver hands back a REG_MULTI_SZ in UTF-16; first string wins.
    pub fn hardware_id(&self, dev: Device) -> String {
        let mut buf = [0u16; 256];
        let bytes = unsafe {
            (self.hwid)(self.ctx, dev, buf.as_mut_ptr().cast(), (buf.len() * 2) as u32)
        } as usize;
        let units = &buf[..(bytes / 2).min(buf.len())];
        let first = units.split(|&u| u == 0).next().unwrap_or(&[]);
        String::from_utf16_lossy(first).to_uppercase()
    }

    /// The keyboard a human is actually typing on: first keyboard slot with a
    /// hardware id that isn't on the skip list. Re-resolved per injection so a
    /// USB replug reshuffling the slots can't send your text into the void.
    pub fn find_keyboard(&self, skip: &[String]) -> Option<Device> {
        KEYBOARDS
            .map(|d| (d, self.hardware_id(d)))
            .find(|(_, id)| !id.is_empty() && !skip.iter().any(|s| id.contains(s.as_str())))
            .map(|(d, _)| d)
    }
}

impl Drop for Driver {
    fn drop(&mut self) { unsafe { (self.destroy)(self.ctx) } }
}

/// GetProcAddress, but typed. `F` must be the exact fn-pointer type of the export.
unsafe fn sym<F: Copy>(dll: HMODULE, name: PCSTR) -> Result<F> {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<usize>()) };
    GetProcAddress(dll, name)
        .map(|f| std::mem::transmute_copy::<_, F>(&f))
        .ok_or_else(|| anyhow!("interception.dll is missing export {}", name.display()))
}
