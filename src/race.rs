// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * race.rs
 *
 * Version: @(#)race.rs 0.2.0 2026
 *
 * Description: The Race Track.
 *              You read once. Every model hears the same take: same breath,
 *              same "uh", same mic hiss. Load, warm up, run three times, keep
 *              the median, ask the judge. One model in RAM at a time, besides
 *              the one that types for you. Winner by accuracy, then by speed.
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

// Events to the page (ui::push):
//   race_start  { lineup: [{id, name, engine}] }
//   race_step   { id, phase: "load" | "warm" | "run", run, of }
//   race_result { id, name, engine, load_ms, ms, xrt, score, text } | { id, name, error }
//   race_end    { cancelled }

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

use crate::audio::SR;
use crate::models::{self, CATALOGUE};
use crate::state::State;
use crate::{asr, config, perf, ui, wer};

const RUNS: usize = 3;

struct Contender { id: String, name: String, engine: models::Engine, dir: PathBuf }

/// Every installed catalogue model, plus the active folder if it's a custom
/// one. The ACTIVE model runs first: its verdict is what the terminal reveals,
/// so it shouldn't wait behind three 2.5 GB loads.
fn lineup(state: &State) -> Vec<Contender> {
    let active = state.cfg.read().model_dir.clone();
    let is_active = |c: &Contender| models::which(&active).map_or(c.id == "custom", |s| s.id == c.id);
    let custom = (models::which(&active).is_none() && config::looks_like_model(&active)).then(|| Contender {
        id: "custom".into(), name: asr::model_name(&active), engine: models::engine_of(&active), dir: active.clone(),
    });
    let (mut first, rest): (Vec<_>, Vec<_>) = CATALOGUE.iter()
        .filter(|s| s.installed())
        .map(|s| Contender { id: s.id.into(), name: s.name.into(), engine: s.engine, dir: s.dir() })
        .chain(custom)
        .partition(is_active);
    first.extend(rest);
    first
}

/// The take you just read, if any.
pub fn last_take(state: &State) -> Result<(Arc<Vec<f32>>, String)> {
    let b = state.bench.lock();
    Ok((b.take.clone().ok_or_else(|| anyhow!("record a take first"))?, b.reference.clone()))
}

/// Race `take` (16 kHz mono) through every installed model, graded against `reference`.
pub fn start(state: Arc<State>, take: Arc<Vec<f32>>, reference: String) -> Result<()> {
    if state.racing.swap(true, Ordering::AcqRel) { bail!("already racing"); }
    state.race_cancel.store(false, Ordering::Release);

    let field = lineup(&state);
    if field.is_empty() {
        state.racing.store(false, Ordering::Release);
        bail!("no models installed — get one from the Models page");
    }
    ui::push("race_start", &json!({
        "audio_ms": take.len() as f64 * 1000.0 / SR as f64,
        "lineup": field.iter().map(|c| json!({ "id": c.id, "name": c.name, "engine": c.engine })).collect::<Vec<_>>()
    }));

    let st = Arc::clone(&state);
    let spawned = std::thread::Builder::new().name("race".into()).spawn(move || {
        let state = st;
        let threads = state.cfg.read().threads();
        let audio_ms = take.len() as f64 * 1000.0 / SR as f64;
        let cancelled = || state.race_cancel.load(Ordering::Acquire);

        for c in &field {
            if cancelled() { break; }
            let step = |phase: &str, run: usize| ui::push("race_step", &json!({ "id": c.id, "phase": phase, "run": run, "of": RUNS }));

            let outcome = (|| -> Result<serde_json::Value> {
                step("load", 0);
                let t0 = Instant::now();
                let mut m = asr::load(&c.dir, threads)?;
                let load_ms = perf::ms(t0.elapsed());

                step("warm", 0);
                asr::transcribe(&mut m, vec![0.0; SR as usize])?;

                let mut times = Vec::with_capacity(RUNS);
                let mut text = String::new();
                for run in 1..=RUNS {
                    if cancelled() { bail!("cancelled"); }
                    step("run", run);
                    let t = Instant::now();
                    let out = asr::transcribe(&mut m, take.as_ref().clone())?;
                    times.push(perf::ms(t.elapsed()));
                    if run == 1 { text = out; }
                }
                times.sort_by(f64::total_cmp);
                let ms = times[RUNS / 2];
                Ok(json!({
                    "id": c.id, "name": c.name, "engine": c.engine, "load_ms": load_ms, "ms": ms,
                    "xrt": audio_ms / ms.max(0.1), "score": wer::align(&reference, &text), "text": text,
                }))
            })();

            match outcome {
                Ok(v) => {
                    info!("race: {} → {} ms, WER {:.3}", c.id, v["ms"], v["score"]["wer"].as_f64().unwrap_or(f64::NAN));
                    ui::push("race_result", &v);
                }
                Err(e) => {
                    warn!("race: {} failed: {e:#}", c.id);
                    ui::push("race_result", &json!({ "id": c.id, "name": c.name, "error": format!("{e:#}") }));
                }
            }
            // model dropped here: one contender in RAM at a time
        }

        ui::push("race_end", &json!({ "cancelled": cancelled() }));
        state.racing.store(false, Ordering::Release);
    });
    spawned.map(|_| ()).map_err(|e| {
        state.racing.store(false, Ordering::Release); // don't leave the flag stuck on a failed spawn
        anyhow!("spawning race thread: {e}")
    })
}
