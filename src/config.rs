// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * config.rs
 *
 * Version: @(#)config.rs 0.2.0 2026
 *
 * Description: The One File Humans Touch.
 *              config.toml in %APPDATA%. Every key has a default, so half a file
 *              still load. Broken file get logged and ignored, never overwritten.
 *              Ancestor of this program stomp your config on a typo. We no do
 *              that.
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

// Lives at %APPDATA%\DivineVoice\config.toml. Every field has a default and the
// struct is #[serde(default)], so a config with half its keys missing still
// loads — missing keys just take defaults. A config that doesn't PARSE gets
// logged and ignored, never overwritten. (The ancestor of this program would
// "helpfully" stomp your file with defaults on a typo. We don't do that here.)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::binding::Binding;

pub const APP: &str = "DivineVoice";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    /// Hold to talk, release to send. Walkie-talkie rules.
    #[default]
    Hold,
    /// Tap to start, tap again to send. For monologues and tired fingers.
    Toggle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InjectMode {
    /// Clipboard + Ctrl+V. Instant, any language, local machine only.
    Paste,
    /// Clipboard + Ctrl+Shift+V. For terminals where Ctrl+V means something weird.
    PastePlain,
    /// SendInput KEYEVENTF_UNICODE, one char at a time. Any language, local only,
    /// leaves your clipboard alone.
    Unicode,
    /// Raw set-1 scancodes through the Interception driver. ASCII / US-QWERTY
    /// only, but they're *real* HID strokes: they survive Parsec and a hardware
    /// KVM. Falls back to Unicode if Interception isn't installed.
    Type,
}

impl InjectMode {
    pub fn short(self) -> &'static str {
        match self {
            Self::Paste      => "paste",
            Self::PastePlain => "paste-plain",
            Self::Unicode    => "unicode",
            Self::Type       => "type",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "Ctrl+Space", "Ctrl+Shift+F13", "RAlt", … — see binding.rs for names.
    pub hotkey: Binding,
    pub hotkey_mode: HotkeyMode,

    /// Dir with encoder-model[.int8].onnx (+.data), decoder_joint-model[.int8].onnx,
    /// vocab.txt. The Models page manages this; hand-edit for a custom folder.
    pub model_dir: PathBuf,

    pub inject_mode: InjectMode,

    /// Mic pin: case-insensitive substring of the device name ("QuadCast").
    /// Unset = Windows default input. Resolved on every press.
    pub input_device: Option<String>,

    /// Clips shorter than this are a fat-fingered hotkey, not speech.
    pub min_ms: u32,
    /// Hard cap. Nobody dictates a novel in one breath.
    pub max_ms: u32,

    /// How long Ctrl+V gets to finish before we put your clipboard back.
    pub paste_settle_ms: u32,

    /// Gap between chars in Type/Unicode mode. 0 = flat out (local only).
    /// 5 survives an i9 → serial → ESP32 → target HID KVM chain without
    /// dropping or reordering.
    pub type_delay_ms: u32,

    /// ONNX Runtime intra-op threads. Sweet spot = physical P-cores. Run
    /// `divinevoice bench <file> --thread-scan 1,2,4,8,16` and pick the knee.
    pub intra_threads: usize,

    /// Interception hardware-id substrings to NEVER inject through — e.g. the
    /// HID gadget of your own KVM, so Type doesn't feed its output back into
    /// its input. Upper-case, like "VID_1209&PID_4B4D".
    pub skip_hwids: Vec<String>,

    /// fbdirector bus (host:port). When set, we tell fbdirector we own
    /// Ctrl+Space so it doesn't forward the chord to the remote box.
    pub fbdirector: Option<String>,

    /// tracing filter. "debug" also logs transcripts — off by default because
    /// a plaintext log of everything you've ever said is a bad default.
    pub log_level: String,

    /// Keep the last `history_max` transcripts in history.json. Opt-in, same
    /// reason as above.
    pub history: bool,
    pub history_max: usize,

    /// How many runs the Perf page keeps. Timings only, no text — always on.
    pub perf_max: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey:          Binding::default(),
            hotkey_mode:     HotkeyMode::Hold,
            model_dir:       default_model_dir(),
            inject_mode:     InjectMode::Paste,
            input_device:    None,
            min_ms:          250,
            max_ms:          120_000,
            paste_settle_ms: 80,
            type_delay_ms:   5,
            intra_threads:   default_intra_threads(),
            skip_hwids:      Vec::new(),
            fbdirector:      None,
            log_level:       "info".into(),
            history:         false,
            history_max:     100,
            perf_max:        64,
        }
    }
}

/// What happened when we went looking for the config. Returned instead of
/// logged, because logging isn't up yet when we read it (the log level is IN it).
pub enum Loaded {
    Found,
    Created,
    Broken(String),
}

impl Config {
    pub fn path() -> PathBuf { home().join("config.toml") }

    pub fn load() -> (Self, Loaded) {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s)
                .map(|c| (c, Loaded::Found))
                .unwrap_or_else(|e| (Self::default(), Loaded::Broken(e.to_string()))),
            Err(_) => {
                let c = Self::default();
                let _ = c.save();
                (c, Loaded::Created)
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        p.parent().map(std::fs::create_dir_all).transpose()?;
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(p, s)
    }

    /// `None` when intra_threads is 0 → let ORT guess.
    pub fn threads(&self) -> Option<usize> { Some(self.intra_threads).filter(|&n| n > 0) }
}

/// %APPDATA%\DivineVoice. Config, log, and (by default) the model live here.
pub fn home() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP)
}

/// First of: `<exe dir>\model`, then each Models-page download, that holds a
/// model. If none does, the default download's folder — the Models page fills
/// it on first run.
fn default_model_dir() -> PathBuf {
    let beside_exe = std::env::current_exe().ok()
        .and_then(|e| e.parent().map(|d| d.join("model")));
    let downloads = crate::models::CATALOGUE.iter().map(|s| s.dir());
    beside_exe.into_iter()
        .chain(downloads)
        .find(|d| looks_like_model(d))
        .unwrap_or_else(|| crate::models::CATALOGUE[0].dir())
}

/// A TDT model dir: vocab.txt + some encoder*.onnx + some decoder_joint*.onnx
/// (fp32 or int8 — parakeet-rs picks whichever is there).
pub fn looks_like_model(dir: &Path) -> bool {
    let has = |prefix: &str| std::fs::read_dir(dir).into_iter().flatten().flatten()
        .any(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(prefix) && n.ends_with(".onnx")));
    dir.join("vocab.txt").is_file() && has("encoder") && has("decoder_joint")
}

/// Half the logical CPUs (≈ physical cores with SMT), clamped to [1, 8].
/// Past ~8 the encoder stops scaling and E-cores start dragging the mean.
fn default_intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 8))
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_in_toml() {
        let c: Config = toml::from_str("hotkey = \"Ctrl+Shift+F13\"\nhotkey_mode = \"toggle\"").unwrap();
        assert_eq!(c.hotkey.to_string(), "Ctrl+Shift+F13");
        assert_eq!(c.hotkey_mode, HotkeyMode::Toggle);
        assert!(toml::from_str::<Config>("hotkey = \"A\"").is_err()); // bricks the A key
    }

    #[test]
    fn partial_config_takes_defaults() {
        let c: Config = toml::from_str("inject_mode = \"type\"\nmin_ms = 100").unwrap();
        assert_eq!(c.inject_mode, InjectMode::Type);
        assert_eq!(c.min_ms, 100);
        assert_eq!(c.max_ms, Config::default().max_ms);
        assert!(c.fbdirector.is_none());
    }

    #[test]
    fn legacy_keys_are_ignored() {
        // GPU-era configs carried `use_cuda` / `engine`. Don't choke on them.
        let c: Config = toml::from_str("use_cuda = true\nengine = \"cuda\"").unwrap();
        assert_eq!(c.inject_mode, InjectMode::Paste);
    }

    #[test]
    fn roundtrip() {
        let c = Config::default();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.inject_mode, c.inject_mode);
        assert_eq!(back.max_ms, c.max_ms);
    }
}
