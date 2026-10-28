// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * state.rs
 *
 * Version: @(#)state.rs 0.2.0 2026
 *
 * Description: The Shared Truth.
 *              One byte say what the pipeline doing. Every hop is compare-and-
 *              swap: old clip finishing can't stomp a new recording, so tray
 *              never lie. Only watchdog and supervisor allowed the blind store.
 *              Big red button.
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

//   Idle ──hotkey──► Recording ──release──► Transcribing ──text──► Injecting ──► Idle
//                       │                        │
//                       └──too short─► Idle      └──silence/err──► Idle
//
// Every pipeline hop is a compare-and-swap (`advance(from, to)`), not a blind
// store. Why: you can start the NEXT recording while the last one is still
// being transcribed. With blind stores the inject thread finishing would slam
// the mode to Idle *while you're talking*, and the tray would lie to your
// face. With CAS, a stale stage's "I'm done" just loses the race.
//
// The only blind store is `force()` — recording start, the watchdog, and the
// supervisor's big red button.

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::config::Config;
use crate::history::History;
use crate::perf::Perf;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Idle         = 0,
    Recording    = 1,
    Transcribing = 2,
    Injecting    = 3,
}

impl Mode {
    const fn from_u8(b: u8) -> Self {
        match b {
            1 => Self::Recording,
            2 => Self::Transcribing,
            3 => Self::Injecting,
            _ => Self::Idle,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle         => "idle",
            Self::Recording    => "RECORDING",
            Self::Transcribing => "transcribing…",
            Self::Injecting    => "injecting…",
        }
    }
}

/// How the speech model is doing, as the UI sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ModelStatus {
    Loading { dir: String },
    Ready   { dir: String, load_ms: u64, warmup_ms: u64 },
    Missing { dir: String },
    Failed  { dir: String, error: String },
}

/// The Benchmark tab's half of the world. While `armed`, a finished clip goes
/// to the judge instead of your cursor, and its audio is kept for races.
#[derive(Default)]
pub struct Bench {
    pub armed:     bool,
    pub reference: String,
    pub take:      Option<std::sync::Arc<Vec<f32>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Download {
    pub id:    String,
    pub done:  u64,
    pub total: u64,
    pub error: Option<String>,
}

pub struct State {
    mode:              AtomicU8,
    pub running:       AtomicBool,
    pub bus_connected: AtomicBool,
    /// Set by the inject thread once it knows whether the driver is there.
    pub interception:  AtomicBool,
    /// Bumped whenever model_dir changes; the ASR thread reloads on mismatch.
    pub model_gen:     AtomicU64,
    pub model:         RwLock<ModelStatus>,
    pub download:      Mutex<Option<Download>>,
    pub dl_cancel:     AtomicBool,
    pub cfg:           RwLock<Config>,
    pub history:       History,
    pub perf:          Perf,
    pub bench:         Mutex<Bench>,
    pub racing:        AtomicBool,
    pub race_cancel:   AtomicBool,
}

impl State {
    pub fn new(cfg: Config) -> Self {
        let dir = cfg.model_dir.display().to_string();
        Self {
            mode:          AtomicU8::new(Mode::Idle as u8),
            running:       AtomicBool::new(true),
            bus_connected: AtomicBool::new(false),
            interception:  AtomicBool::new(false),
            model_gen:     AtomicU64::new(0),
            model:         RwLock::new(ModelStatus::Loading { dir }),
            download:      Mutex::new(None),
            dl_cancel:     AtomicBool::new(false),
            cfg:           RwLock::new(cfg),
            history:       History::load(),
            perf:          Perf::load(),
            bench:         Mutex::new(Bench::default()),
            racing:        AtomicBool::new(false),
            race_cancel:   AtomicBool::new(false),
        }
    }

    pub fn alive(&self) -> bool { self.running.load(Ordering::Acquire) }
    pub fn shutdown(&self)      { self.running.store(false, Ordering::Release) }

    pub fn mode(&self) -> Mode { Mode::from_u8(self.mode.load(Ordering::Acquire)) }

    /// `from → to` iff we're still in `from`. Returns whether we won.
    pub fn advance(&self, from: Mode, to: Mode) -> bool {
        self.mode
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Unconditional. Recording start, watchdog, supervisor. Nobody else.
    pub fn force(&self, m: Mode) { self.mode.store(m as u8, Ordering::Release) }

    pub fn input_device(&self) -> Option<String> { self.cfg.read().input_device.clone() }

    pub fn reload_model(&self) { self.model_gen.fetch_add(1, Ordering::AcqRel); }

    /// Window hidden/closed or page left: dictation goes back to your cursor.
    pub fn disarm_bench(&self) { self.bench.lock().armed = false; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_stage_cannot_clobber_a_new_recording() {
        let s = State::new(Config::default());
        s.force(Mode::Recording);
        assert!(s.advance(Mode::Recording, Mode::Transcribing));
        s.force(Mode::Recording);                                 // user hits the hotkey again
        assert!(!s.advance(Mode::Transcribing, Mode::Injecting)); // old clip finishes: loses
        assert!(!s.advance(Mode::Injecting, Mode::Idle));
        assert_eq!(s.mode(), Mode::Recording);                    // tray tells the truth
    }
}
