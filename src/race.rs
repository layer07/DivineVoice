// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * race.rs
 *
 * Version: @(#)race.rs 0.3.0 2026
 *
 * Description: The Race Track.
 *              You read once. Every model hears the same take: same breath,
 *              same "uh", same mic hiss. Load, warm up, run three times, keep
 *              the median, ask the judge. One model in RAM at a time, besides
 *              the one that types for you; on GPU even that one step aside.
 *              Winner by accuracy, then by speed.
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
//   race_start  { audio_ms, lineup: [{id, model, name, engine, device}] }
//   race_step   { id, phase: "load" | "warm" | "run", run, of }
//   race_result { id, model, name, engine, device, load_ms, ms, xrt, score, text } | { id, name, device, error }
//   race_end    { cancelled }
//
// A contender is (model, device): "parakeet-ultra@directml". Pick one model
// or all of them, on one device or several, and the board compares exactly
// that. GPUs load STRICT: a GPU that can't come up is a failed row with the
// reason, never CPU numbers wearing a GPU label.

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::audio::SR;
use crate::config::Device;
use crate::models::{self, CATALOGUE};
use crate::state::{ModelStatus, State};
use crate::{asr, config, perf, ui, wer};

const RUNS: usize = 3;

/// Which models to race.
pub enum Scope { Active, All, Only(Vec<String>) }

struct Contender { id: String, model: String, name: String, engine: models::Engine, device: Device, dir: PathBuf }

/// The installed models in scope × the devices asked for. The ACTIVE model
/// runs first (its verdict is what the terminal reveals), and within a model
/// the devices run in the order given, so CPU-vs-GPU land next to each other.
fn lineup(state: &State, scope: &Scope, devices: &[Device]) -> Vec<Contender> {
    let active = state.cfg.read().model_dir.clone();
    let active_id = models::which(&active).map_or("custom", |s| s.id).to_owned();
    let custom = (models::which(&active).is_none() && config::looks_like_model(&active))
        .then(|| ("custom".to_owned(), asr::model_name(&active), models::engine_of(&active), active.clone()));
    let installed = CATALOGUE.iter()
        .filter(|s| s.installed())
        .map(|s| (s.id.to_owned(), s.name.to_owned(), s.engine, s.dir()))
        .chain(custom);
    let wanted = |id: &str| match scope {
        Scope::Active => id == active_id,
        Scope::All => true,
        Scope::Only(ids) => ids.iter().any(|x| x == id),
    };
    let (mut models, rest): (Vec<_>, Vec<_>) = installed.filter(|m| wanted(&m.0)).partition(|m| m.0 == active_id);
    models.extend(rest);
    models.into_iter()
        .flat_map(|(model, name, engine, dir)| devices.iter().map(move |&device| Contender {
            id: format!("{model}@{}", device.label()), model: model.clone(), name: name.clone(), engine, device, dir: dir.clone(),
        }))
        .collect()
}

/// Cleanup that runs however the race thread ends, panics included: give the
/// VRAM back (the ASR thread reloads when `park` drops), clear `racing` so the
/// next race isn't refused forever, and unstick the UI's RACING banner.
struct Lap<'a> { state: &'a State, parked: bool }

impl Drop for Lap<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() { ui::push("race_end", &json!({ "cancelled": true })); }
        if self.parked { self.state.park.store(false, Ordering::Release); }
        self.state.racing.store(false, Ordering::Release);
    }
}

/// Is the dictation model sitting on a GPU right now? Parked counts: back-to-
/// back races must not let it reload onto the card in between.
fn dictation_on_gpu(state: &State) -> bool {
    match &*state.model.read() {
        ModelStatus::Ready { device, .. } => device != Device::Cpu.label(),
        ModelStatus::Parked { .. } => true,
        _ => false,
    }
}

/// Ask the ASR thread to drop its model and wait until it has. It checks
/// every 250 ms, but may be mid-load or mid-clip, hence the generous timeout.
/// If it never answers we race anyway: worst case a GPU row says "out of
/// memory", which is the truth.
fn park(state: &State) {
    state.park.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !state.parked.load(Ordering::Acquire) && Instant::now() < deadline && !state.race_cancel.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The take you just read, if any.
pub fn last_take(state: &State) -> Result<(Arc<Vec<f32>>, String)> {
    let b = state.bench.lock();
    Ok((b.take.clone().ok_or_else(|| anyhow!("record a take first"))?, b.reference.clone()))
}

/// Race `take` (16 kHz mono) through `scope` × `devices`, graded against `reference`.
pub fn start(state: Arc<State>, take: Arc<Vec<f32>>, reference: String, scope: Scope, devices: Vec<Device>) -> Result<()> {
    let devices = if devices.is_empty() { vec![state.cfg.read().device] } else { devices };
    let field = lineup(&state, &scope, &devices);
    if field.is_empty() { bail!("nothing to race: no installed model matches — get one from the Models page"); }
    if state.racing.swap(true, Ordering::AcqRel) { bail!("already racing"); }
    state.race_cancel.store(false, Ordering::Release);

    ui::push("race_start", &json!({
        "audio_ms": take.len() as f64 * 1000.0 / SR as f64,
        "lineup": field.iter().map(|c| json!({
            "id": c.id, "model": c.model, "name": c.name, "engine": c.engine, "device": c.device,
        })).collect::<Vec<_>>()
    }));

    let st = Arc::clone(&state);
    let spawned = std::thread::Builder::new().name("race".into()).spawn(move || {
        let state = st;
        let needs_vram = field.iter().any(|c| c.device != Device::Cpu) && dictation_on_gpu(&state);
        let _lap = Lap { state: &state, parked: needs_vram }; // armed BEFORE parking, so nothing can skip the unpark
        if needs_vram { park(&state); }
        let (threads, lang) = { let c = state.cfg.read(); (c.threads(), c.language.clone()) };
        let audio_ms = take.len() as f64 * 1000.0 / SR as f64;
        let cancelled = || state.race_cancel.load(Ordering::Acquire);

        for c in &field {
            if cancelled() { break; }
            let step = |phase: &str, run: usize| ui::push("race_step", &json!({ "id": c.id, "phase": phase, "run": run, "of": RUNS }));

            let outcome = (|| -> Result<serde_json::Value> {
                step("load", 0);
                let t0 = Instant::now();
                let mut m = asr::load_on(&c.dir, threads, c.device, true, &lang)?.model;
                let load_ms = perf::ms(t0.elapsed());

                step("warm", 0);
                asr::warm_up(&mut m)?;

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
                let ms = { let mut t = times.clone(); t.sort_by(f64::total_cmp); t[RUNS / 2] };
                Ok(json!({
                    "id": c.id, "model": c.model, "name": c.name, "engine": c.engine, "device": c.device,
                    "load_ms": load_ms, "ms": ms, "runs": times, "xrt": audio_ms / ms.max(0.1),
                    "score": wer::align(&reference, &text), "text": text,
                }))
            })();

            match outcome {
                Ok(v) => {
                    info!("race: {} → {} ms, WER {:.3}", c.id, v["ms"], v["score"]["wer"].as_f64().unwrap_or(f64::NAN));
                    ui::push("race_result", &v);
                }
                Err(e) => {
                    warn!("race: {} failed: {e:#}", c.id);
                    ui::push("race_result", &json!({
                        "id": c.id, "model": c.model, "name": c.name, "engine": c.engine, "device": c.device, "error": asr::explain(c.device, &format!("{e:#}")),
                    }));
                }
            }
            // model dropped here: one contender in RAM at a time
        }

        ui::push("race_end", &json!({ "cancelled": cancelled() }));
    });
    spawned.map(|_| ()).map_err(|e| {
        state.racing.store(false, Ordering::Release); // don't leave the flag stuck on a failed spawn
        anyhow!("spawning race thread: {e}")
    })
}
