// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * asr.rs
 *
 * Version: @(#)asr.rs 0.2.0 2026
 *
 * Description: The Ear Brain.
 *              Clip come in, words come out. Parakeet TDT on CPU, on purpose: GPU
 *              is freight train, push-to-talk is bicycle courier. Model swap?
 *              Drop old brain, load new brain, feed it one second of silence so
 *              your first sentence no pay cold tax.
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

// We benchmarked the GPU. For push-to-talk — one short clip, then silence for
// a minute — a fat desktop CPU wins: no clock ramp from idle, no host↔device
// copies, no sync tax on every call. The GPU is a freight train; this job is
// a bicycle courier. Numbers in README.md, live numbers on the Perf page.
//
// The thread owns the model and hot-swaps it when the Models page changes
// `model_dir` (State::model_gen bumps → we reload between clips). No model on
// disk is a normal state, not a crash: we report it and wait.
//
// Cold starts: ONNX Runtime doesn't really finish setting up at load. Memory
// arenas, kernel selection and friends happen on the FIRST inference — which
// used to be your first sentence of the day. So after every load we feed it a
// second of silence and eat that cost ourselves. Perf entries still flag the
// first real clip after a load as `cold`, so the data can prove it worked.
//
// Failure model:
//   Err from an inference → logged, clip dropped, back to Idle.
//   panic inside ONNX Runtime → NOT caught here. It unwinds out of `run`, the
//     supervisor in main.rs catches it, waits 2 s, calls `run` again, which
//     reloads from scratch. A poisoned session can't outlive its one clip.

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parakeet_rs::{ExecutionConfig, ParakeetTDT, ParakeetUnified, TimestampMode, Transcriber};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::audio::SR;
use crate::config::looks_like_model;
use crate::models::{self, Engine};
use crate::perf::{self, Clip, Transcript};
use crate::state::{Mode, ModelStatus, State};
use crate::ui;

/// A loaded model, whichever engine it needs. Both speak parakeet-rs's
/// Transcriber trait; Unified's impl is its OFFLINE path (full context, the
/// low-WER mode), not the chunked streaming one.
pub enum Model {
    Tdt(Box<ParakeetTDT>),
    Unified(Box<ParakeetUnified>),
}

impl Model {
    pub fn engine(&self) -> Engine {
        match self { Self::Tdt(_) => Engine::Tdt, Self::Unified(_) => Engine::Unified }
    }
}

/// One ONNX Runtime environment for the whole process. parakeet-rs 0.4 makes
/// you bring your own (0.3 hid it); it's an Arc inside, so every model swap
/// just clones the handle instead of spinning up a new runtime.
fn ort_env() -> Result<parakeet_rs::ort::environment::Environment> {
    static ENV: std::sync::OnceLock<parakeet_rs::ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(e) = ENV.get() { return Ok(e.clone()); }
    let built = parakeet_rs::ort::init().with_name("divinevoice").build()
        .map_err(|e| anyhow!("ONNX Runtime init: {e}"))?;
    Ok(ENV.get_or_init(|| built).clone())
}

pub fn load(dir: &Path, threads: Option<usize>) -> Result<Model> {
    if !looks_like_model(dir) {
        return Err(anyhow!("no Parakeet model in {} — download one from the Models page", dir.display()));
    }
    // No execution provider set = plain CPU. The config carries the runtime
    // environment and (optionally) the thread count.
    let base = ExecutionConfig::new().with_environment(ort_env()?);
    let exec = Some(match threads { Some(n) => base.with_intra_threads(n), None => base });
    let ctx = || format!("loading {}", dir.display());
    Ok(match models::engine_of(dir) {
        Engine::Tdt     => Model::Tdt(Box::new(ParakeetTDT::from_pretrained(dir, exec).with_context(ctx)?)),
        Engine::Unified => Model::Unified(Box::new(ParakeetUnified::from_pretrained(dir, exec).with_context(ctx)?)),
    })
}

/// One clip in, trimmed text out. Empty string = it heard nothing worth typing.
pub fn transcribe(model: &mut Model, clip: Vec<f32>) -> Result<String> {
    fn go(t: &mut impl Transcriber, clip: Vec<f32>) -> Result<String> {
        t.transcribe_samples(clip, SR, 1, Some(TimestampMode::Sentences))
            .map(|r| r.text.trim().to_owned())
            .map_err(Into::into)
    }
    match model {
        Model::Tdt(m)     => go(m.as_mut(), clip),
        Model::Unified(m) => go(m.as_mut(), clip),
    }
}

/// Human name for whatever's in `dir`: catalogue name, or the folder name.
pub fn model_name(dir: &Path) -> String {
    models::which(dir).map(|s| s.name.to_owned()).unwrap_or_else(|| {
        format!("custom · {}", dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into()))
    })
}

/// (Re)load whatever `model_dir` points at, warm it up, publish to the UI.
fn bring_up(state: &State) -> Option<Model> {
    let (dir, threads) = { let c = state.cfg.read(); (c.model_dir.clone(), c.threads()) };
    let shown = dir.display().to_string();
    let publish = |s: ModelStatus| { *state.model.write() = s; ui::push_state(); };

    if !looks_like_model(&dir) {
        info!("no model at {shown} — waiting for one");
        publish(ModelStatus::Missing { dir: shown });
        return None;
    }

    publish(ModelStatus::Loading { dir: shown.clone() });
    let t0 = Instant::now();
    match load(&dir, threads) {
        Ok(mut m) => {
            let load_ms = t0.elapsed().as_millis() as u64;
            let t1 = Instant::now();
            if let Err(e) = transcribe(&mut m, vec![0.0; SR as usize]) { warn!("warm-up inference failed: {e:#}"); }
            let warmup_ms = t1.elapsed().as_millis() as u64;
            info!("parakeet up ({:?}): load {load_ms} ms + warm-up {warmup_ms} ms from {shown} ({} threads)",
                m.engine(), threads.map_or("ORT-default".into(), |n| n.to_string()));
            publish(ModelStatus::Ready { dir: shown, load_ms, warmup_ms });
            Some(m)
        }
        Err(e) => {
            warn!("model load failed: {e:#}");
            publish(ModelStatus::Failed { dir: shown, error: format!("{e:#}") });
            None
        }
    }
}

pub fn run(state: &State, clips: &Receiver<Clip>, out: &Sender<Transcript>) -> Result<()> {
    let mut gen = state.model_gen.load(Ordering::Acquire);
    let mut model = bring_up(state);
    let mut cold = true;

    while state.alive() {
        // Wake up regularly to notice a model swap even when nobody's talking.
        let clip = match clips.recv_timeout(Duration::from_millis(250)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let now = state.model_gen.load(Ordering::Acquire);
        if now != gen {
            gen = now;
            drop(model.take()); // free the old one first: two 2.5 GB models at once is a choice
            model = bring_up(state);
            cold = true;
        }

        let Some(Clip { pcm, sw }) = clip else { continue };
        let Some(m) = model.as_mut() else {
            warn!("dropping a clip: no model loaded");
            state.advance(Mode::Transcribing, Mode::Idle);
            continue;
        };

        // Benchmark tab armed? Then this clip is an exam, not dictation: keep
        // the audio for races and route the verdict to the judge.
        let exam = {
            let b = state.bench.lock();
            b.armed.then(|| (b.reference.clone(), std::sync::Arc::new(pcm.clone())))
        };

        let queue_ms = perf::ms(sw.enqueued.elapsed());
        let t0 = Instant::now();
        let result = transcribe(m, pcm);
        let asr_ms = perf::ms(t0.elapsed());
        let was_cold = std::mem::replace(&mut cold, false);

        let (dir, threads, keep, max) = {
            let c = state.cfg.read();
            (c.model_dir.clone(), c.intra_threads, c.history, c.history_max)
        };

        if let Some((reference, audio)) = exam {
            state.advance(Mode::Transcribing, Mode::Idle);
            grade(state, &reference, audio, result, &sw, asr_ms, &model_name(&dir));
            continue;
        }
        let ts = |text: String| Transcript {
            text, sw: sw.clone(), queue_ms, asr_ms, cold: was_cold, model: model_name(&dir), threads,
        };

        match result {
            Ok(text) if text.is_empty() => {
                info!("heard nothing ({asr_ms} ms)");
                state.advance(Mode::Transcribing, Mode::Idle);
                // Still a benchmark: the model did the work, there was just nothing to type.
                let now = Instant::now();
                crate::inject::record(state, ts(text).finish(now, now, "—"));
            }
            Ok(text) => {
                info!("transcribed {} chars in {asr_ms} ms{}", text.chars().count(), if was_cold { " (cold)" } else { "" });
                debug!("text: {text:?}");
                if keep {
                    let e = state.history.push(text.clone(), sw.audio_ms as u64, asr_ms as u64, max);
                    ui::push("history_add", &e);
                }
                state.advance(Mode::Transcribing, Mode::Injecting);
                out.send(ts(text)).map_err(|_| anyhow!("injector hung up"))?;
            }
            Err(e) => {
                warn!("transcribe: {e:#}");
                state.advance(Mode::Transcribing, Mode::Idle);
            }
        }
    }
    Ok(())
}

/// Score a Benchmark-tab take and hand the verdict to the page. The audio is
/// parked in State so "Race" can replay the exact same take through every
/// model: same breath, same "uh", same mic hiss.
fn grade(
    state: &State, reference: &str, audio: std::sync::Arc<Vec<f32>>,
    result: Result<String>, sw: &perf::Stopwatch, asr_ms: f64, model: &str,
) {
    state.bench.lock().take = Some(audio);
    let payload = match result {
        Ok(text) => {
            let score = crate::wer::align(reference, &text);
            info!("bench take: {:.1}% WER ({}/{}) in {asr_ms} ms", score.wer * 100.0, score.errors, score.ref_words);
            let words = text.split_whitespace().count() as f64;
            serde_json::json!({
                "text": text, "score": score, "model": model,
                "audio_ms": sw.audio_ms, "asr_ms": asr_ms,
                "latency_ms": perf::ms(sw.released.elapsed()),
                "wpm": if sw.audio_ms > 0.0 { words / (sw.audio_ms / 60_000.0) } else { 0.0 },
            })
        }
        Err(e) => serde_json::json!({ "error": format!("{e:#}") }),
    };
    ui::push("bench_take", &payload);
}
