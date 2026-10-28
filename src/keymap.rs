// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * keymap.rs
 *
 * Version: @(#)keymap.rs 0.2.0 2026
 *
 * Description: The Keyboard Map.
 *              char to set-1 scancode, US-QWERTY, table shaped like a real
 *              keyboard. IBM give consecutive codes per row in 1981; we use it.
 *              Fancy quote fold to ASCII so "what's" no become "whats".
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

// Each row: (scancode of the first key, unshifted legends, shifted legends).
// Keys in a row have consecutive scancodes — IBM got that much right in 1981.

/// Set-1 make codes we need by name. Break code = make | 0x80, but Interception
/// wants make code + KEY_UP flag, so we never touch the high bit.
pub mod sc {
    pub const ENTER:  u16 = 0x1C;
    pub const TAB:    u16 = 0x0F;
    pub const SPACE:  u16 = 0x39;
    pub const LSHIFT: u16 = 0x2A;
    pub const RSHIFT: u16 = 0x36;
    pub const CTRL:   u16 = 0x1D; // right Ctrl = same code + E0 prefix
    pub const ALT:    u16 = 0x38; // right Alt  = same code + E0 prefix
}

const ROWS: &[(u16, &str, &str)] = &[
    (0x02, "1234567890-=",  "!@#$%^&*()_+"),
    (0x10, "qwertyuiop[]",  "QWERTYUIOP{}"),
    (0x1E, "asdfghjkl;'`",  "ASDFGHJKL:\"~"),
    (0x2B, "\\zxcvbnm,./",  "|ZXCVBNM<>?"),
];

/// (scancode, needs_shift), or None if a US keyboard can't type it.
pub fn scancode(c: char) -> Option<(u16, bool)> {
    match c {
        ' '         => Some((sc::SPACE, false)),
        '\t'        => Some((sc::TAB,   false)),
        '\n' | '\r' => Some((sc::ENTER, false)),
        _ => ROWS.iter().find_map(|&(base, lo, hi)| {
            let at = |legends: &str| legends.chars().position(|k| k == c).map(|i| base + i as u16);
            at(lo).map(|s| (s, false)).or_else(|| at(hi).map(|s| (s, true)))
        }),
    }
}

/// Parakeet speaks fluent typography. US-QWERTY does not. Fold the fancy stuff
/// down to ASCII so "what’s up" doesn't come out as "whats up".
pub fn asciify(s: &str) -> String {
    s.chars()
        .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
            match c {
                '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => Box::new(std::iter::once('\'')),
                '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => Box::new(std::iter::once('"')),
                '\u{2010}'..='\u{2015}' | '\u{2212}'              => Box::new(std::iter::once('-')),
                '\u{2026}'                                        => Box::new("...".chars()),
                '\u{00A0}' | '\u{2009}' | '\u{202F}'              => Box::new(std::iter::once(' ')),
                _                                                 => Box::new(std::iter::once(c)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_scancodes() {
        // Spot-checked against the IBM PC/AT reference, not against this table.
        assert_eq!(scancode('1'),  Some((0x02, false)));
        assert_eq!(scancode('!'),  Some((0x02, true)));
        assert_eq!(scancode('='),  Some((0x0D, false)));
        assert_eq!(scancode('q'),  Some((0x10, false)));
        assert_eq!(scancode('P'),  Some((0x19, true)));
        assert_eq!(scancode('}'),  Some((0x1B, true)));
        assert_eq!(scancode('a'),  Some((0x1E, false)));
        assert_eq!(scancode('"'),  Some((0x28, true)));
        assert_eq!(scancode('~'),  Some((0x29, true)));
        assert_eq!(scancode('\\'), Some((0x2B, false)));
        assert_eq!(scancode('z'),  Some((0x2C, false)));
        assert_eq!(scancode('?'),  Some((0x35, true)));
        assert_eq!(scancode(' '),  Some((0x39, false)));
    }

    #[test]
    fn every_printable_ascii_is_typeable() {
        assert!((' '..='~').all(|c| scancode(c).is_some()));
    }

    #[test]
    fn no_two_chars_share_a_key_and_shift_state() {
        let mut seen: Vec<(u16, bool)> = (' '..='~').filter_map(scancode).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n);
    }

    #[test]
    fn non_ascii_is_refused() {
        assert_eq!(scancode('ç'), None);
        assert_eq!(scancode('ã'), None);
    }

    #[test]
    fn asciify_folds_typography() {
        assert_eq!(asciify("what\u{2019}s \u{201C}up\u{201D} \u{2014} ok\u{2026}"),
                   "what's \"up\" - ok...");
        assert_eq!(asciify("ação"), "ação"); // not our job to mangle Portuguese
    }
}
