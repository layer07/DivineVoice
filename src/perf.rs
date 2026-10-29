// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * perf.rs
 *
 * Version: @(#)perf.rs 0.3.2 2026
 *
 * Description: The Stopwatch.
 *              "Feels fast" is not a number. Release, resample, queue, asr,
 *              delivered: every clip timed, last 64 kept. Numbers only, never
 *              your words. Stats pure, tested, computed once, in Rust.
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

// Every clip gets a stopwatch from the moment you let go of the hotkey to the
// moment the text is delivered, split into stages:
//
//   release ─resample─► queued ─wait─► asr ─infer─► inject ─deliver─► done
//
//   resample_ms  48 kHz → 16 kHz, once, after release
//   queue_ms     sitting in the channel while the previous clip transcribed
//   asr_ms       Parakeet, encoder + greedy TDT decode
//   inject_ms    until the text is in the target (paste: Ctrl+V sent;
//                type/unicode: last key sent). Clipboard-restore naps don't count:
//                you already have your text by then.
//   total_ms     release → delivered. The number your brain feels.
//
// The last `perf_max` (default 64) runs live in perf.json. Numbers only — no
// transcript text — so unlike history this is always on.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Timing that rides along with a clip through the pipeline.
#[derive(Debug, Clone)]
pub struct Stopwatch {
    pub released:    Instant,
    pub audio_ms:    f64,
    pub resample_ms: f64,
    pub enqueued:    Instant,
}

pub struct Clip {
    pub pcm: Vec<f32>,
    pub sw:  Stopwatch,
}

/// Text plus everything we know about how it got made.
pub struct Transcript {
    pub text:     String,
    pub sw:       Stopwatch,
    pub queue_ms: f64,
    pub asr_ms:   f64,
    pub cold:     bool,
    pub model:    String,
    pub threads:  usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unix ms at release. Doubles as the id.
    pub id:          u64,
    pub audio_ms:    f64,
    pub words:       u32,
    pub chars:       u32,
    pub resample_ms: f64,
    pub queue_ms:    f64,
    pub asr_ms:      f64,
    pub inject_ms:   f64,
    pub total_ms:    f64,
    /// First inference after a model (re)load. The tax you came here to see.
    pub cold:        bool,
    pub model:       String,
    pub threads:     usize,
    pub mode:        String,
}

pub fn ms(d: Duration) -> f64 { (d.as_secs_f64() * 10_000.0).round() / 10.0 } // 0.1 ms grain

impl Transcript {
    /// Close the books: `delivered` is when the text landed.
    pub fn finish(&self, delivered: Instant, inject_started: Instant, mode: &str) -> Entry {
        let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let since_release = self.sw.released.elapsed().as_millis() as u64;
        Entry {
            id:          unix_ms.saturating_sub(since_release),
            audio_ms:    self.sw.audio_ms,
            words:       self.text.split_whitespace().count() as u32,
            chars:       self.text.chars().count() as u32,
            resample_ms: self.sw.resample_ms,
            queue_ms:    self.queue_ms,
            asr_ms:      self.asr_ms,
            inject_ms:   ms(delivered.saturating_duration_since(inject_started)),
            total_ms:    ms(delivered.saturating_duration_since(self.sw.released)),
            cold:        self.cold,
            model:       self.model.clone(),
            threads:     self.threads,
            mode:        mode.to_owned(),
        }
    }
}

// ── stats (pure) ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub n:             usize,
    pub p50_total_ms:  f64,
    pub p95_total_ms:  f64,
    pub p50_asr_ms:    f64,
    /// audio seconds per compute second. 30× = a 3 s clip in 100 ms.
    pub p50_xrt:       f64,
    /// output words per second of ASR compute.
    pub p50_wps:       f64,
    /// how fast YOU talk, words per minute of audio.
    pub p50_wpm:       f64,
    /// mean total of cold runs minus mean total of warm runs. None if either side is empty.
    pub cold_tax_ms:   Option<f64>,
}

/// Nearest-rank percentile on an already-sorted slice.
fn pct(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn sorted(it: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut v: Vec<f64> = it.filter(|x| x.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    v
}

fn mean(it: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = it.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| s / n as f64)
}

pub fn summarize(es: &[Entry]) -> Summary {
    let rate = |num: f64, den_ms: f64| if den_ms > 0.0 { num / (den_ms / 1000.0) } else { f64::NAN };
    let total = sorted(es.iter().map(|e| e.total_ms));
    let spoken: Vec<&Entry> = es.iter().filter(|e| e.words > 0).collect();
    let warm = mean(es.iter().filter(|e| !e.cold).map(|e| e.total_ms));
    let cold = mean(es.iter().filter(|e| e.cold).map(|e| e.total_ms));
    Summary {
        n:            es.len(),
        p50_total_ms: pct(&total, 0.50),
        p95_total_ms: pct(&total, 0.95),
        p50_asr_ms:   pct(&sorted(es.iter().map(|e| e.asr_ms)), 0.50),
        p50_xrt:      pct(&sorted(es.iter().map(|e| e.audio_ms / e.asr_ms.max(0.1))), 0.50),
        p50_wps:      pct(&sorted(spoken.iter().map(|e| rate(e.words as f64, e.asr_ms))), 0.50),
        p50_wpm:      pct(&sorted(spoken.iter().map(|e| rate(e.words as f64, e.audio_ms) * 60.0)), 0.50),
        cold_tax_ms:  cold.zip(warm).map(|(c, w)| c - w),
    }
}

// ── store ────────────────────────────────────────────────────────────────────

pub struct Perf {
    path:  PathBuf,
    items: Mutex<VecDeque<Entry>>, // newest first
}

impl Perf {
    pub fn load() -> Self {
        let path = crate::config::home().join("perf.json");
        let items = std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, items: Mutex::new(items) }
    }

    pub fn list(&self) -> Vec<Entry> { self.items.lock().iter().cloned().collect() }

    pub fn push(&self, e: Entry, max: usize) { self.mutate(|v| { v.push_front(e); v.truncate(max.max(1)); }) }
    pub fn clear(&self)                      { self.mutate(VecDeque::clear) }
    pub fn trim(&self, max: usize)           { self.mutate(|v| v.truncate(max.max(1))) }

    fn mutate(&self, f: impl FnOnce(&mut VecDeque<Entry>)) {
        let mut v = self.items.lock();
        f(&mut v);
        let json = serde_json::to_string(&*v).unwrap_or_else(|_| "[]".into());
        if let Err(e) = std::fs::write(&self.path, json) { warn!("perf: write {}: {e}", self.path.display()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(total: f64, asr: f64, audio: f64, words: u32, cold: bool) -> Entry {
        Entry { id: 0, audio_ms: audio, words, chars: words * 5, resample_ms: 1.0, queue_ms: 0.0,
                asr_ms: asr, inject_ms: total - asr - 1.0, total_ms: total, cold,
                model: "m".into(), threads: 8, mode: "paste".into() }
    }

    #[test]
    fn empty_is_zeroes_not_nan_soup() {
        let s = summarize(&[]);
        assert_eq!(s.n, 0);
        assert_eq!(s.p50_total_ms, 0.0);
        assert!(s.cold_tax_ms.is_none());
    }

    #[test]
    fn medians_and_rates() {
        let es = [e(100.0, 80.0, 3000.0, 10, false), e(120.0, 100.0, 4000.0, 12, false), e(400.0, 380.0, 3000.0, 9, true)];
        let s = summarize(&es);
        assert_eq!(s.p50_total_ms, 120.0);
        assert_eq!(s.p95_total_ms, 400.0);
        assert_eq!(s.p50_asr_ms, 100.0);
        // xRT: 3000/80=37.5, 4000/100=40, 3000/380≈7.9 → median 37.5
        assert!((s.p50_xrt - 37.5).abs() < 1e-9);
        // wps: 10/0.08=125, 12/0.1=120, 9/0.38≈23.7 → median 120
        assert!((s.p50_wps - 120.0).abs() < 1e-9);
        // wpm: 10/3*60=200, 12/4*60=180, 9/3*60=180 → median 180
        assert!((s.p50_wpm - 180.0).abs() < 1e-9);
        // cold tax: 400 - mean(100,120)=110 → 290
        assert_eq!(s.cold_tax_ms, Some(290.0));
    }

    #[test]
    fn silent_clips_dont_poison_word_rates() {
        let es = [e(50.0, 40.0, 1000.0, 0, false), e(100.0, 80.0, 3000.0, 10, false)];
        let s = summarize(&es);
        assert!((s.p50_wps - 125.0).abs() < 1e-9);
    }

    #[test]
    fn ms_has_tenth_grain() {
        assert_eq!(ms(Duration::from_micros(12_345)), 12.3);
    }
}
