// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * autostart.rs
 *
 * Version: @(#)autostart.rs 0.3.0 2026
 *
 * Description: The Alarm Clock.
 *              One string under HKCU\...\CurrentVersion\Run. The registry is
 *              the truth, not config.toml: the switch shows what Windows will
 *              ACTUALLY do at logon. Moved the exe to a new folder? Then the
 *              value points at a ghost, the switch says off, flip it again.
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

// Per-user, no admin, no scheduled task, no shortcut in a Startup folder that
// survives uninstalling by being forgotten. Started at logon it comes up the
// usual way: tray only, window stays shut unless there's no model yet.

use anyhow::{anyhow, Result};
use std::path::Path;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = crate::config::APP;

/// What goes in the Run value: the exe, quoted (Program Files has spaces). Pure.
pub fn run_line(exe: &Path) -> String { format!("\"{}\"", exe.display()) }

/// Does a stored Run value start THIS exe? Quotes, case and slash direction
/// don't count; a different path does. Pure.
pub fn points_here(stored: &str, exe: &Path) -> bool {
    let norm = |s: &str| s.trim().trim_matches('"').replace('/', "\\").to_lowercase();
    norm(stored) == norm(&exe.display().to_string())
}

/// Will Windows start this very exe at logon?
pub fn enabled() -> bool {
    std::env::current_exe().is_ok_and(|exe| reg::read().is_some_and(|v| points_here(&v, &exe)))
}

pub fn set(on: bool) -> Result<()> {
    if on { reg::write(&run_line(&std::env::current_exe()?)) } else { reg::delete() }
}

mod reg {
    use super::{anyhow, Result, NAME, RUN};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    };

    /// An open key that closes itself.
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) { unsafe { let _ = RegCloseKey(self.0); } }
    }

    fn open(access: REG_SAM_FLAGS) -> Result<Key> {
        let mut k = HKEY::default();
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN), 0, access, &mut k) };
        if r == ERROR_SUCCESS { Ok(Key(k)) } else { Err(anyhow!("open HKCU\\{RUN}: {r:?}")) }
    }

    pub fn read() -> Option<String> {
        let k = open(KEY_QUERY_VALUE).ok()?;
        let name = HSTRING::from(NAME);
        let (mut ty, mut len) = (REG_VALUE_TYPE::default(), 0u32);
        // Ask for the size, then the string. UTF-16, NUL-terminated (usually).
        unsafe { RegQueryValueExW(k.0, &name, None, Some(&mut ty), None, Some(&mut len)) }.ok().ok()?;
        let mut buf = vec![0u16; (len as usize).div_ceil(2)];
        unsafe { RegQueryValueExW(k.0, &name, None, Some(&mut ty), Some(buf.as_mut_ptr().cast()), Some(&mut len)) }.ok().ok()?;
        (ty == REG_SZ).then(|| String::from_utf16_lossy(&buf).trim_end_matches('\0').to_owned())
    }

    pub fn write(line: &str) -> Result<()> {
        let k = open(KEY_SET_VALUE)?;
        let wide: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
        unsafe { RegSetValueExW(k.0, &HSTRING::from(NAME), 0, REG_SZ, Some(bytes)) }
            .ok().map_err(|e| anyhow!("write Run\\{NAME}: {e}"))
    }

    pub fn delete() -> Result<()> {
        let k = open(KEY_SET_VALUE)?;
        match unsafe { RegDeleteValueW(k.0, &HSTRING::from(NAME)) } {
            r if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND => Ok(()), // already off is off
            r => Err(anyhow!("delete Run\\{NAME}: {r:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn run_line_quotes_paths_with_spaces() {
        assert_eq!(run_line(Path::new(r"C:\Program Files\DivineVoice\divinevoice.exe")),
                   r#""C:\Program Files\DivineVoice\divinevoice.exe""#);
    }

    #[test]
    fn points_here_ignores_quotes_case_and_slashes_not_paths() {
        let exe = PathBuf::from(r"C:\Tools\DivineVoice\divinevoice.exe");
        assert!(points_here(r#""C:\Tools\DivineVoice\divinevoice.exe""#, &exe));
        assert!(points_here("c:/tools/divinevoice/DIVINEVOICE.EXE", &exe));
        assert!(!points_here(r#""D:\old\divinevoice.exe""#, &exe)); // moved: a ghost, so off
        assert!(!points_here("", &exe));
    }
}
