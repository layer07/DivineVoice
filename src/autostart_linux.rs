// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * autostart_linux.rs
 *
 * Version: @(#)autostart_linux.rs 0.4.0 2026
 *
 * Description: The Alarm Clock On The Penguin.
 *              One .desktop file in ~/.config/autostart. Desktop read it at
 *              login, start us. No root, no systemd unit, no cron. Delete file,
 *              done. Hyprland and friends don't read it: exec-once there.
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

// XDG autostart: GNOME, KDE, XFCE, Cinnamon and anything run through
// `dex` / `uwsm` read ~/.config/autostart/*.desktop at login. Bare tiling
// compositors (Hyprland, Sway) don't; there it's one line in their config,
// `exec-once = divinevoice`. This file can't fix that, so it doesn't pretend.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

const FILE: &str = "divinevoice.desktop";

/// ~/.config/autostart, honouring $XDG_CONFIG_HOME.
fn dir() -> Option<PathBuf> {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    env("XDG_CONFIG_HOME").filter(|p| p.is_absolute())
        .or_else(|| env("HOME").map(|h| h.join(".config")))
        .map(|c| c.join("autostart"))
}

/// The whole .desktop entry for `exe`. Pure.
pub fn entry(exe: &Path) -> String {
    format!("[Desktop Entry]\nType=Application\nName=DivineVoice\n\
             Comment=Hold the shortcut, talk, let go\nExec=\"{}\"\nTerminal=false\n\
             X-GNOME-Autostart-enabled=true\n", exe.display())
}

/// Does this .desktop text start THIS exe? Quotes don't count; a different
/// path (the exe moved) does. Pure.
pub fn points_here(desktop: &str, exe: &Path) -> bool {
    desktop.lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .is_some_and(|cmd| cmd.trim().trim_matches('"') == exe.display().to_string())
}

/// Will the desktop start this very exe at login?
pub fn enabled() -> bool {
    let (Some(d), Ok(exe)) = (dir(), std::env::current_exe()) else { return false };
    std::fs::read_to_string(d.join(FILE)).is_ok_and(|s| points_here(&s, &exe))
}

pub fn set(on: bool) -> Result<()> {
    let d = dir().ok_or_else(|| anyhow!("no $HOME to put an autostart entry in"))?;
    let path = d.join(FILE);
    if on {
        std::fs::create_dir_all(&d)?;
        std::fs::write(&path, entry(&std::env::current_exe()?)).with_context(|| format!("write {}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("rm {}", path.display())),
            _ => Ok(()), // already off is off
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_points_at_the_exe_and_round_trips() {
        let exe = Path::new("/home/nuc/DivineVoice/target/release/divinevoice");
        let e = entry(exe);
        assert!(e.starts_with("[Desktop Entry]\n"));
        assert!(e.contains("Exec=\"/home/nuc/DivineVoice/target/release/divinevoice\"\n"));
        assert!(points_here(&e, exe));
        assert!(!points_here(&e, Path::new("/opt/divinevoice/divinevoice"))); // moved: a ghost, so off
        assert!(!points_here("", exe));
    }
}
