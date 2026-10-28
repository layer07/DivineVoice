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
use parakeet_rs::{ExecutionConfig, ParakeetTDT, TimestampMode, Transcriber};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::audio::SR;
use crate::config::looks_like_model;
use crate::perf::{self, Clip, Transcript};
use crate::state::{Mode, ModelStatus, State};
use crate::{models, ui};

pub fn load(dir: &Path, threads: Option<usize>) -> Result<ParakeetTDT> {
    if !looks_like_model(dir) {
        return Err(anyhow!("no Parakeet model in {} — download one from the Models page", dir.display()));
    }
    // No execution provider set = plain CPU. The config only carries threads.
    let exec = threads.map(|n| ExecutionConfig::new().with_intra_threads(n));
    ParakeetTDT::from_pretrained(dir, exec).with_context(|| format!("loading {}", dir.display()))
}

/// One clip in, trimmed text out. Empty string = it heard nothing worth typing.
pub fn transcribe(model: &mut ParakeetTDT, clip: Vec<f32>) -> Result<String> {
    model
        .transcribe_samples(clip, SR, 1, Some(TimestampMode::Sentences))
        .map(|r| r.text.trim().to_owned())
        .map_err(Into::into)
}

/// Human name for whatever's in `dir`: catalogue name, or the folder name.
pub fn model_name(dir: &Path) -> String {
    models::which(dir).map(|s| s.name.to_owned()).unwrap_or_else(|| {
        format!("custom · {}", dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into()))
    })
}

/// (Re)load whatever `model_dir` points at, warm it up, publish to the UI.
fn bring_up(state: &State) -> Option<ParakeetTDT> {
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
            info!("parakeet up: load {load_ms} ms + warm-up {warmup_ms} ms from {shown} ({} threads)",
                threads.map_or("ORT-default".into(), |n| n.to_string()));
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

        let queue_ms = perf::ms(sw.enqueued.elapsed());
        let t0 = Instant::now();
        let result = transcribe(m, pcm);
        let asr_ms = perf::ms(t0.elapsed());
        let was_cold = std::mem::replace(&mut cold, false);

        let (dir, threads, keep, max) = {
            let c = state.cfg.read();
            (c.model_dir.clone(), c.intra_threads, c.history, c.history_max)
        };
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
