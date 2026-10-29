// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * binding.rs
 *
 * Version: @(#)binding.rs 0.3.1 2026
 *
 * Description: The Hotkey, As Data.
 *              "Ctrl+Shift+F13" in, (mods, VK) out, and back again. Pure. No
 *              Win32. Refuse binding that eat a key forever: plain A as push-to-
 *              talk mean you never type A again. Cave man learn this the hard
 *              way.
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

// Modifiers are side-agnostic (Ctrl = either Ctrl). The TRIGGER key is exact,
// and may itself be a side-specific modifier: "RCtrl" alone is a perfectly
// good push-to-talk key, and a popular one.

use std::fmt;
use std::str::FromStr;

pub const CTRL:  u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT:   u8 = 4;
pub const WIN:   u8 = 8;

const MOD_NAMES: [(u8, &str); 4] = [(CTRL, "Ctrl"), (SHIFT, "Shift"), (ALT, "Alt"), (WIN, "Win")];

pub const VK_ESCAPE: u16 = 0x1B;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub mods: u8,
    pub vk:   u16,
}

impl Default for Binding {
    fn default() -> Self { Self { mods: CTRL, vk: 0x20 } } // Ctrl+Space, the classic
}

/// Which modifier group a VK belongs to (L/R/generic all count), or 0.
pub const fn mod_of(vk: u16) -> u8 {
    match vk {
        0x11 | 0xA2 | 0xA3 => CTRL,
        0x10 | 0xA0 | 0xA1 => SHIFT,
        0x12 | 0xA4 | 0xA5 => ALT,
        0x5B | 0x5C        => WIN,
        _ => 0,
    }
}

// ── names ────────────────────────────────────────────────────────────────────

const NAMED: &[(u16, &str)] = &[
    (0x08, "Backspace"), (0x09, "Tab"), (0x0D, "Enter"), (0x13, "Pause"), (0x14, "CapsLock"),
    (0x1B, "Esc"), (0x20, "Space"), (0x21, "PageUp"), (0x22, "PageDown"), (0x23, "End"),
    (0x24, "Home"), (0x25, "Left"), (0x26, "Up"), (0x27, "Right"), (0x28, "Down"),
    (0x2C, "PrintScreen"), (0x2D, "Insert"), (0x2E, "Delete"),
    (0x5B, "LWin"), (0x5C, "RWin"), (0x5D, "Menu"),
    (0x6A, "Num*"), (0x6B, "Num+"), (0x6D, "Num-"), (0x6E, "Num."), (0x6F, "Num/"),
    (0x90, "NumLock"), (0x91, "ScrollLock"),
    (0xA0, "LShift"), (0xA1, "RShift"), (0xA2, "LCtrl"), (0xA3, "RCtrl"), (0xA4, "LAlt"), (0xA5, "RAlt"),
    (0xAD, "Mute"), (0xAE, "VolumeDown"), (0xAF, "VolumeUp"),
    (0xB0, "NextTrack"), (0xB1, "PrevTrack"), (0xB2, "Stop"), (0xB3, "PlayPause"),
    (0xBA, ";"), (0xBB, "="), (0xBC, ","), (0xBD, "-"), (0xBE, "."), (0xBF, "/"), (0xC0, "`"),
    (0xDB, "["), (0xDC, "\\"), (0xDD, "]"), (0xDE, "'"),
];

/// Every key we can name: the table above, plus the boring regular ranges.
fn names() -> impl Iterator<Item = (u16, String)> {
    let ascii = |r: std::ops::RangeInclusive<u8>| r.map(|c| (c as u16, (c as char).to_string()));
    NAMED.iter().map(|&(v, n)| (v, n.to_owned()))
        .chain(ascii(b'0'..=b'9'))
        .chain(ascii(b'A'..=b'Z'))
        .chain((0..=9u16).map(|i| (0x60 + i, format!("Num{i}"))))
        .chain((1..=24u16).map(|i| (0x6F + i, format!("F{i}"))))
}

pub fn key_name(vk: u16) -> String {
    names().find(|(v, _)| *v == vk).map(|(_, n)| n).unwrap_or_else(|| format!("VK{vk:02X}"))
}

fn key_from_name(s: &str) -> Option<u16> {
    names().find(|(_, n)| n.eq_ignore_ascii_case(s)).map(|(v, _)| v)
        .or_else(|| s.strip_prefix("VK").or(s.strip_prefix("vk"))
            .and_then(|h| u16::from_str_radix(h, 16).ok()))
}

fn mod_from_name(s: &str) -> Option<u8> {
    match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control"         => Some(CTRL),
        "shift"                    => Some(SHIFT),
        "alt"                      => Some(ALT),
        "win" | "super" | "meta"   => Some(WIN),
        _ => None,
    }
}

// ── string form ──────────────────────────────────────────────────────────────

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        MOD_NAMES.iter()
            .filter(|(bit, _)| self.mods & bit != 0)
            .try_for_each(|(_, n)| write!(f, "{n}+"))?;
        f.write_str(&key_name(self.vk))
    }
}

/// Peel "Mod+" tokens off the front until what's left is the key. Can't just
/// split on '+': the key itself may contain one ("Ctrl+Num+").
fn peel(s: &str, acc: u8) -> Result<(u8, &str), String> {
    match s.split_once('+') {
        Some((head, tail)) if !tail.is_empty() => match mod_from_name(head.trim()) {
            Some(m) => peel(tail, acc | m),
            None if key_from_name(s.trim()).is_some() => Ok((acc, s)),
            None => Err(format!("{:?} isn't a modifier (Ctrl, Shift, Alt, Win)", head.trim())),
        },
        _ => Ok((acc, s)),
    }
}

impl FromStr for Binding {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (mods, key) = peel(s.trim(), 0)?;
        let vk = key_from_name(key.trim()).ok_or_else(|| format!("don't know a key called {:?}", key.trim()))?;
        let b = Binding { mods, vk };
        b.sane().map(|_| b)
    }
}

impl serde::Serialize for Binding {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.collect_str(self) }
}

impl<'de> serde::Deserialize<'de> for Binding {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

impl Binding {
    /// Reject bindings that would brick a key. A hold-to-talk on plain "A"
    /// means you can never type the letter A again while DivineVoice runs.
    /// We've all been there. Well, one of us has.
    pub fn sane(&self) -> Result<(), String> {
        let typing_key = matches!(self.vk,
            0x08 | 0x09 | 0x0D | 0x1B | 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDE);
        match self.vk {
            0x01..=0x06       => Err("mouse buttons aren't keys".into()),
            0x10..=0x12       => Err("pick a side: LCtrl/RCtrl, LShift/RShift, LAlt/RAlt".into()),
            _ if self.mods == 0 && typing_key =>
                Err(format!("{} alone would eat that key forever — add a modifier", key_name(self.vk))),
            _ => Ok(()),
        }
    }

    pub const fn pack(self) -> u32 { self.vk as u32 | (self.mods as u32) << 16 }
    pub const fn unpack(x: u32) -> Self { Self { vk: x as u16, mods: (x >> 16) as u8 & 0x0F } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Binding { s.parse().unwrap() }

    #[test]
    fn roundtrips() {
        for s in ["Ctrl+Space", "Ctrl+Shift+F13", "RAlt", "Pause", "Alt+Win+`", "Ctrl+Num+", "Ctrl+=", "F24"] {
            assert_eq!(p(s).to_string(), s, "{s}");
        }
    }

    #[test]
    fn parsing_is_forgiving() {
        assert_eq!(p(" control + space "), p("Ctrl+Space"));
        assert_eq!(p("ctrl+shift+a"), Binding { mods: CTRL | SHIFT, vk: 0x41 });
        assert_eq!(p("Ctrl+Num+"), Binding { mods: CTRL, vk: 0x6B });
        assert_eq!(p("vkE8"), Binding { mods: 0, vk: 0xE8 });
    }

    #[test]
    fn insane_bindings_rejected() {
        assert!("A".parse::<Binding>().is_err());
        assert!("Space".parse::<Binding>().is_err());
        assert!("Ctrl".parse::<Binding>().is_err());
        assert!("Hyper+X".parse::<Binding>().is_err());
        assert!("Ctrl+Banana".parse::<Binding>().is_err());
        assert!("RCtrl".parse::<Binding>().is_ok());
        assert!("F13".parse::<Binding>().is_ok());
    }

    #[test]
    fn pack_roundtrip() {
        let b = p("Ctrl+Alt+Win+F7");
        assert_eq!(Binding::unpack(b.pack()), b);
    }

    #[test]
    fn mods_are_side_agnostic() {
        assert_eq!(mod_of(0xA2), CTRL);
        assert_eq!(mod_of(0xA3), CTRL);
        assert_eq!(mod_of(0x5C), WIN);
        assert_eq!(mod_of(0x41), 0);
    }
}
