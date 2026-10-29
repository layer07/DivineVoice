// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * audio.rs
 *
 * Version: @(#)audio.rs 0.3.1 2026
 *
 * Description: The Mouth Catcher.
 *              Hotkey say start, mic open. Hotkey say stop, mic close. Callback
 *              only downmix: realtime thread do nothing clever, ever. Resample
 *              once, after release. Stopwatch start at release, because that the
 *              moment your brain start waiting.
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

// One thread. Blocks on the hotkey channel. On Start: open the mic, let the
// cpal callback downmix into a Vec at the device's native rate. On Stop (or
// max_ms): drop the stream, resample the whole clip once, ship it.
//
// cpal::Stream is !Send on some backends, so it's born, lives and dies inside
// `record()`. It never meets another thread. Very monastic.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SizedSample, Stream, StreamConfig};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::dsp;
use crate::hotkey::Cmd;
use crate::perf::{self, Clip, Stopwatch};
use crate::state::{Mode, State};

pub const SR: u32 = 16_000;

pub fn run(state: &State, cmds: Receiver<Cmd>, out: Sender<Clip>) -> Result<()> {
    while state.alive() {
        let Ok(cmd) = cmds.recv() else { return Ok(()) };
        if cmd == Cmd::Stop { continue; } // stray release, nothing armed

        info!("hotkey → recording");
        state.force(Mode::Recording);
        let (min_ms, max_ms) = { let c = state.cfg.read(); (c.min_ms, c.max_ms) };

        match record(&cmds, max_ms, state.input_device().as_deref()) {
            Ok((native, rate, released)) => {
                // The stopwatch starts at release; resampling is on the clock.
                let t0 = Instant::now();
                let pcm = dsp::resample(&native, rate, SR);
                let resample_ms = perf::ms(t0.elapsed());
                let ms = pcm.len() as u64 * 1000 / SR as u64;
                if ms < min_ms as u64 {
                    info!("{ms} ms < min_ms {min_ms} — fat-fingered hotkey, discarded");
                    state.advance(Mode::Recording, Mode::Idle);
                } else {
                    info!("captured {ms} ms (resample {resample_ms} ms)");
                    state.advance(Mode::Recording, Mode::Transcribing);
                    let sw = Stopwatch { released, audio_ms: ms as f64, resample_ms, enqueued: Instant::now() };
                    out.send(Clip { pcm, sw }).map_err(|_| anyhow!("transcriber hung up"))?;
                }
            }
            Err(e) => {
                warn!("record: {e:#}");
                state.advance(Mode::Recording, Mode::Idle);
            }
        }
    }
    Ok(())
}

/// Every input device name, deduped, for the tray picker. Best effort.
pub fn list_inputs() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok())
            .fold(Vec::new(), |mut v, n| { if !v.contains(&n) { v.push(n) } v }))
        .unwrap_or_default()
}

/// `Some(needle)` → first device whose name contains it (case-insensitive).
/// No match → warn and use the default; a stale pin should never mean no mic.
fn pick_device(host: &cpal::Host, want: Option<&str>) -> Result<Device> {
    let pinned = want.and_then(|needle| {
        let needle = needle.to_lowercase();
        let hit = host.input_devices().ok()?
            .find(|d| d.name().is_ok_and(|n| n.to_lowercase().contains(&needle)));
        if hit.is_none() { warn!("no mic matches {needle:?} — using system default"); }
        hit
    });
    pinned
        .or_else(|| host.default_input_device())
        .ok_or_else(|| anyhow!("no input device at all. Is a mic plugged in?"))
}

/// Record until Stop (or max_ms). Returns native-rate mono, the rate, and the
/// instant you let go — the zero point for every latency number in perf.rs.
fn record(cmds: &Receiver<Cmd>, max_ms: u32, want: Option<&str>) -> Result<(Vec<f32>, u32, Instant)> {
    let host   = cpal::default_host();
    let device = pick_device(&host, want)?;
    let fmt    = device.default_input_config().context("default_input_config")?;
    let (rate, ch) = (fmt.sample_rate().0, fmt.channels());
    info!("mic: {} @ {rate} Hz × {ch}ch {:?}",
        device.name().unwrap_or_else(|_| "<nameless>".into()), fmt.sample_format());

    let cfg  = StreamConfig { channels: ch, sample_rate: fmt.sample_rate(), buffer_size: cpal::BufferSize::Default };
    let sink = Arc::new(Mutex::new(Vec::<f32>::with_capacity(rate as usize * 30)));

    use cpal::SampleFormat as F;
    let stream = match fmt.sample_format() {
        F::F32 => open::<f32>(&device, &cfg, &sink),
        F::I16 => open::<i16>(&device, &cfg, &sink),
        F::U16 => open::<u16>(&device, &cfg, &sink),
        F::I32 => open::<i32>(&device, &cfg, &sink),
        F::U8  => open::<u8 >(&device, &cfg, &sink),
        F::I8  => open::<i8 >(&device, &cfg, &sink),
        F::F64 => open::<f64>(&device, &cfg, &sink),
        other  => Err(anyhow!("sample format {other:?}? in this economy?")),
    }?;
    stream.play().context("stream.play")?;

    let t0 = Instant::now();
    let cap = Duration::from_millis(max_ms as u64);
    loop {
        match cmds.recv_timeout(Duration::from_millis(50)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Start) => {} // autorepeat leaked through somehow; already recording
            Err(RecvTimeoutError::Timeout) if t0.elapsed() >= cap => {
                info!("max_ms {max_ms} hit — cutting you off");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    let released = Instant::now();
    drop(stream);

    let native = std::mem::take(&mut *sink.lock());
    Ok((native, rate, released))
}

/// One generic stream builder instead of one copy-pasted closure per sample
/// format. The callback converts → downmixes → appends. No allocation beyond
/// the Vec growing, which it mostly won't (pre-sized for 30 s).
fn open<T>(dev: &Device, cfg: &StreamConfig, sink: &Arc<Mutex<Vec<f32>>>) -> Result<Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let ch   = cfg.channels as usize;
    let sink = Arc::clone(sink);
    let mut scratch: Vec<f32> = Vec::new();
    dev.build_input_stream(
        cfg,
        move |data: &[T], _| {
            scratch.clear();
            scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
            sink.lock().extend(dsp::downmix(&scratch, ch));
        },
        |e| warn!("cpal stream error: {e}"),
        None,
    )
    .context("build_input_stream")
}
