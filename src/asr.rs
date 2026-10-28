// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * asr.rs
 *
 * Version: @(#)asr.rs 0.2.0 2026
 *
 * Description: The Ear Brain.
 *              Clip come in, words come out. Parakeet on CPU or GPU, you pick,
 *              and a GPU that lie about working get caught before it load.
 *              Model swap? Drop old brain, load new brain, feed it one second
 *              of silence so your first sentence no pay cold tax.
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

// CPU or GPU is `config.device`. The CPU is the default because it always
// works and a fat desktop CPU is already 25-40× real-time. The GPU (DirectML
// on anything, CUDA on NVIDIA) can be several times faster again, on the
// machines where it comes up at all. Don't guess: race them on the Benchmark
// page, or `divinevoice race --clip <id> --devices cpu,directml`.
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
// Parking: a GPU race on the Benchmark page needs the VRAM the dictation model
// is sitting on (two 2.5 GB models on a 10 GB card that's also running your
// browser = OOM). The race sets `park`; we drop the model and say `parked`;
// the race clears `park` when done, even if it panics; we reload. Clips that
// arrive while parked are dropped with a warning, not queued.
//
// Failure model:
//   Err from an inference → logged, clip dropped, back to Idle.
//   Err from an inference ON A GPU → reload on CPU, run the same clip again.
//     Your sentence survives a VRAM blowout; the UI says why it's on CPU now.
//   panic inside ONNX Runtime → NOT caught here. It unwinds out of `run`, the
//     supervisor in main.rs catches it, waits 2 s, calls `run` again, which
//     reloads from scratch. A poisoned session can't outlive its one clip.

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parakeet_rs::{ExecutionConfig, ParakeetTDT, ParakeetUnified, TimestampMode, Transcriber};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::audio::SR;
use crate::config::{looks_like_model, Device};
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

// ── devices ──────────────────────────────────────────────────────────────────
//
// The trap this section exists for: parakeet-rs registers a GPU provider
// WITHOUT error_on_failure, so if CUDA can't load (driver too old, no cuDNN)
// ONNX Runtime quietly runs on the CPU and you "benchmark the GPU" on the CPU.
// So before using a GPU we build a real session WITH error_on_failure. That's
// the probe. It fails loudly, with the real reason.
//
// "Real session" matters: registering a provider only loads its DLL. The card
// itself isn't touched until a session is created, which is where a too-old
// driver finally says so (CUDA error 35). Hence PROBE_MODEL below.

/// The smallest ONNX model that counts: y = Identity(x), one float. 67 bytes
/// of hand-rolled protobuf (ir 8, opset 13), so the probe needs no file and
/// no `onnx` package. Its only job is to make ORT create the provider for real.
#[cfg(feature = "gpu")]
const PROBE_MODEL: [u8; 67] = [
    0x08, 0x08, 0x3a, 0x3b, 0x0a, 0x10, 0x0a, 0x01, 0x78, 0x12, 0x01, 0x79, 0x22, 0x08, 0x49, 0x64,
    0x65, 0x6e, 0x74, 0x69, 0x74, 0x79, 0x12, 0x05, 0x70, 0x72, 0x6f, 0x62, 0x65, 0x5a, 0x0f, 0x0a,
    0x01, 0x78, 0x12, 0x0a, 0x0a, 0x08, 0x08, 0x01, 0x12, 0x04, 0x0a, 0x02, 0x08, 0x01, 0x62, 0x0f,
    0x0a, 0x01, 0x79, 0x12, 0x0a, 0x0a, 0x08, 0x08, 0x01, 0x12, 0x04, 0x0a, 0x02, 0x08, 0x01, 0x42,
    0x02, 0x10, 0x0d,
];

/// ORT's GPU errors, translated from driver-ese into something you can act on.
/// Pure: string in, string out.
pub fn explain(device: Device, raw: &str) -> String {
    const HINTS: [(&[&str], &str); 4] = [
        (&["CUDA failure 35", "insufficient driver"],
            "NVIDIA driver too old for this CUDA build (needs 580+): update the driver, or use DirectML"),
        (&["cudnn", "cuDNN"],
            "cuDNN 9 not found: install it (or put its DLLs next to divinevoice.exe), or use DirectML"),
        (&["LoadLibrary", "error code 126"],
            "GPU runtime DLLs missing: they have to sit next to divinevoice.exe"),
        (&["bad allocation", "out of memory", "E_OUTOFMEMORY"],
            "out of GPU memory: close something that eats VRAM, pick a smaller model, or use CPU"),
    ];
    let raw = raw.trim();
    let already = HINTS.iter().any(|(_, h)| raw.contains(h)); // explained once is enough
    match HINTS.iter().find(|(needles, _)| needles.iter().any(|n| raw.contains(n))) {
        Some((_, h)) if device != Device::Cpu && !already => format!("{h} ({raw})"),
        _ => raw.to_owned(),
    }
}

/// Can `device` actually come up on this machine? Err = the reason it can't.
pub fn probe(device: Device) -> std::result::Result<(), String> {
    #[cfg(feature = "gpu")]
    {
        use parakeet_rs::ort::{ep, session::Session};
        let fail = |e: &dyn std::fmt::Display| explain(device, &e.to_string());
        let env = ort_env().map_err(|e| fail(&e))?;
        let b = Session::builder(&env).map_err(|e| fail(&e))?;
        let mut b = match device {
            Device::Cpu      => return Ok(()),
            Device::Cuda     => b.with_execution_providers([ep::CUDA::default().build().error_on_failure()]),
            Device::DirectMl => b.with_memory_pattern(false).map_err(|e| fail(&e))?
                .with_parallel_execution(false).map_err(|e| fail(&e))?
                .with_execution_providers([ep::DirectML::default().build().error_on_failure()]),
        }.map_err(|e| fail(&e))?;
        b.commit_from_memory(&PROBE_MODEL).map(drop).map_err(|e| fail(&e))
    }
    #[cfg(not(feature = "gpu"))]
    {
        match device {
            Device::Cpu => Ok(()),
            _ => Err("this build has no GPU support (built with --no-default-features)".into()),
        }
    }
}

/// Every device and whether it works here. Probed once per process: loading
/// CUDA's DLLs to ask the same question twice is a waste of a second.
type Probed = Vec<(Device, std::result::Result<(), String>)>;
static DEVICES: std::sync::OnceLock<Probed> = std::sync::OnceLock::new();

/// Probes on first call (`run` does that at startup, on the ASR thread, so a
/// slow CUDA DLL load never freezes the window), then tells the UI.
pub fn devices() -> &'static [(Device, std::result::Result<(), String>)] {
    let mut fresh = false;
    let d = DEVICES.get_or_init(|| {
        fresh = true;
        Device::ALL.iter().map(|&d| {
            let r = probe(d);
            if let Err(e) = &r { info!("device {}: unavailable — {e}", d.label()); }
            (d, r)
        }).collect()
    });
    if fresh { ui::push_state(); }
    d
}

/// What the UI gets: the probe results, or None while they're still cooking.
/// Never blocks, never probes.
pub fn devices_known() -> Option<&'static [(Device, std::result::Result<(), String>)]> {
    DEVICES.get().map(Vec::as_slice)
}

fn device_ok(d: Device) -> std::result::Result<(), String> {
    devices().iter().find(|(x, _)| *x == d).map(|(_, r)| r.clone()).unwrap_or(Ok(()))
}

fn exec_config(threads: Option<usize>, device: Device) -> Result<ExecutionConfig> {
    let base = ExecutionConfig::new().with_environment(ort_env()?);
    let base = match threads { Some(n) => base.with_intra_threads(n), None => base };
    #[cfg(feature = "gpu")]
    let base = {
        use parakeet_rs::ExecutionProvider as Ep;
        match device {
            Device::Cpu => base,
            Device::Cuda => base.with_execution_provider(Ep::Cuda),
            // DirectML insists on sequential execution and no memory-pattern
            // planning; ORT refuses the session otherwise.
            Device::DirectMl => base.with_execution_provider(Ep::DirectML).with_custom_configure(|b| {
                use parakeet_rs::ort::{session::builder::SessionBuilder, Error};
                let plain = |e: Error<SessionBuilder>| -> Error { e.into() };
                b.with_memory_pattern(false).map_err(plain)?.with_parallel_execution(false).map_err(plain)
            }),
        }
    };
    #[cfg(not(feature = "gpu"))]
    let _ = device;
    Ok(base)
}

/// A loaded model plus the device it ACTUALLY runs on, and why, if that's
/// not the one you asked for.
pub struct Loaded {
    pub model:  Model,
    pub device: Device,
    pub note:   Option<String>,
}

/// Load `dir` on `want`. `strict`: a GPU that won't come up is an error
/// (benchmarks: never pass CPU numbers off as GPU numbers). Not strict: fall
/// back to CPU and say why (dictation: typing beats purity).
pub fn load_on(dir: &Path, threads: Option<usize>, want: Device, strict: bool) -> Result<Loaded> {
    if !looks_like_model(dir) {
        return Err(anyhow!("no Parakeet model in {} — download one from the Models page", dir.display()));
    }
    let (device, note) = match device_ok(want) {
        Ok(()) => (want, None),
        Err(why) if strict => return Err(anyhow!("{} unavailable: {why}", want.label())),
        Err(why) => (Device::Cpu, Some(format!("{} unavailable, running on CPU: {why}", want.label()))),
    };
    let exec = Some(exec_config(threads, device)?);
    let ctx = |e: parakeet_rs::Error| anyhow!("loading {} on {}: {}", dir.display(), device.label(), explain(device, &e.to_string()));
    let model = match models::engine_of(dir) {
        Engine::Tdt     => Model::Tdt(Box::new(ParakeetTDT::from_pretrained(dir, exec).map_err(ctx)?)),
        Engine::Unified => Model::Unified(Box::new(ParakeetUnified::from_pretrained(dir, exec).map_err(ctx)?)),
    };
    Ok(Loaded { model, device, note })
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
/// `demoted` = the GPU just died on us: load on CPU and show this as the reason.
/// Returns the folder it ACTUALLY loaded too: labels come from that, not from
/// a config that may have moved on mid-clip.
fn bring_up(state: &State, demoted: Option<String>) -> Option<(Model, Device, PathBuf)> {
    let (dir, threads, want) = { let c = state.cfg.read(); (c.model_dir.clone(), c.threads(), c.device) };
    let want = if demoted.is_some() { Device::Cpu } else { want };
    let shown = dir.display().to_string();
    let publish = |s: ModelStatus| { *state.model.write() = s; ui::push_state(); };

    if !looks_like_model(&dir) {
        info!("no model at {shown} — waiting for one");
        publish(ModelStatus::Missing { dir: shown });
        return None;
    }

    publish(ModelStatus::Loading { dir: shown.clone() });
    let t0 = Instant::now();
    match load_on(&dir, threads, want, false) {
        Ok(Loaded { model: mut m, device, note }) => {
            let note = demoted.or(note);
            let load_ms = t0.elapsed().as_millis() as u64;
            let t1 = Instant::now();
            if let Err(e) = transcribe(&mut m, vec![0.0; SR as usize]) { warn!("warm-up inference failed: {e:#}"); }
            let warmup_ms = t1.elapsed().as_millis() as u64;
            info!("parakeet up ({:?} on {}): load {load_ms} ms + warm-up {warmup_ms} ms from {shown} ({} threads)",
                m.engine(), device.label(), threads.map_or("ORT-default".into(), |n| n.to_string()));
            if let Some(n) = &note { warn!("{n}"); }
            publish(ModelStatus::Ready { dir: shown, load_ms, warmup_ms, device: device.label().into(), note });
            Some((m, device, dir))
        }
        Err(e) => {
            warn!("model load failed: {e:#}");
            publish(ModelStatus::Failed { dir: shown, error: format!("{e:#}") });
            None
        }
    }
}

pub fn run(state: &State, clips: &Receiver<Clip>, out: &Sender<Transcript>) -> Result<()> {
    devices(); // probe the GPUs here, off the UI thread, before anyone asks
    let mut gen = state.model_gen.load(Ordering::Acquire);
    let mut model = bring_up(state, None);
    let mut cold = true;

    while state.alive() {
        // Wake up regularly to notice a model swap even when nobody's talking.
        let clip = match clips.recv_timeout(Duration::from_millis(250)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        // A GPU race wants the VRAM: let go of the model until it's over.
        if state.park.load(Ordering::Acquire) {
            if let Some((m, _, dir)) = model.take() {
                drop(m); // gone BEFORE we say so: the race starts loading the moment we do
                info!("model parked: a GPU race needs the VRAM");
                *state.model.write() = ModelStatus::Parked { dir: dir.display().to_string() };
                ui::push_state();
            }
            state.parked.store(true, Ordering::Release);
            if clip.is_some() {
                warn!("dropping a clip: model parked for a benchmark race");
                state.advance(Mode::Transcribing, Mode::Idle);
            }
            continue;
        }
        let unparked = state.parked.swap(false, Ordering::AcqRel);

        let now = state.model_gen.load(Ordering::Acquire);
        if now != gen || unparked {
            gen = now;
            drop(model.take()); // free the old one first: two 2.5 GB models at once is a choice
            model = bring_up(state, None);
            cold = true;
        }

        let Some(Clip { pcm, sw }) = clip else { continue };
        let Some((m, on, loaded)) = model.as_mut() else {
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
        let (on, mut name) = (*on, model_name(loaded));
        let spare = (on != Device::Cpu).then(|| pcm.clone());
        let mut result = transcribe(m, pcm);
        // The GPU fell over mid-clip (VRAM ran out, driver hiccup). Don't eat
        // what you just said: drop to CPU, run the same clip again, and say why.
        // The next settings change tries the GPU again.
        if let (Some(err), Some(pcm)) = (result.as_ref().err().map(|e| format!("{e:#}")), spare) {
            let why = format!("{} failed mid-clip, switched to CPU: {}", on.label(), explain(on, &err));
            warn!("{why}");
            drop(model.take());
            model = bring_up(state, Some(why));
            if let Some((m, _, dir)) = model.as_mut() { name = model_name(dir); result = transcribe(m, pcm); }
        }
        let asr_ms = perf::ms(t0.elapsed());
        let was_cold = std::mem::replace(&mut cold, false);

        let (threads, keep, max) = { let c = state.cfg.read(); (c.intra_threads, c.history, c.history_max) };

        if let Some((reference, audio)) = exam {
            state.advance(Mode::Transcribing, Mode::Idle);
            grade(state, &reference, audio, result, &sw, asr_ms, &name);
            continue;
        }
        let ts = |text: String| Transcript {
            text, sw: sw.clone(), queue_ms, asr_ms, cold: was_cold, model: name.clone(), threads,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_errors_get_translated() {
        let e = explain(Device::Cuda, "CUDA failure 35: (null) ; GPU=-1 ; expr=cudaSetDevice(info_.device_id);");
        assert!(e.starts_with("NVIDIA driver too old"), "{e}");
        assert!(e.contains("cudaSetDevice"), "raw error kept for the bug report: {e}");
        let e = explain(Device::DirectMl, "running Attention node. Status Message: bad allocation");
        assert!(e.starts_with("out of GPU memory"), "{e}");
    }

    #[test]
    fn cpu_and_unknown_errors_pass_through() {
        assert_eq!(explain(Device::Cpu, "bad allocation"), "bad allocation"); // your RAM, not VRAM
        assert_eq!(explain(Device::Cuda, "the model is haunted"), "the model is haunted");
        let once = explain(Device::Cuda, "CUDA failure 35");
        assert_eq!(explain(Device::Cuda, &format!("cuda unavailable: {once}")), format!("cuda unavailable: {once}"));
    }
}
