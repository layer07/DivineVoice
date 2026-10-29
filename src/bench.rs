// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * bench.rs
 *
 * Version: @(#)bench.rs 0.2.0 2026
 *
 * Description: The Truth Machine.
 *              Decode audio file. Load model. Run cold, run warm, run again.
 *              Print p50, p95, realtime factor. Thread scan find the knee of your
 *              CPU. Then --device cuda|directml, same clip, and the GPU
 *              gets judged by the same machine. No lying in either direction.
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

// Decode any mp3/wav/flac/m4a/ogg → 16 kHz mono, load the model, run one cold
// inference and N warm ones, print load/cold/mean/p50/p95/min/max and the
// real-time factor. With --thread-scan, do all that per thread count and print
// a scaling table so you can find your CPU's knee instead of guessing.
//
// This tool once told us to stop bothering with the GPU. That was CUDA with
// a pile of pain. DirectML on a 3080 then went 8× faster than the CPU on
// parakeet-v3, so now it asks YOUR card: --device cuda|directml. A GPU that
// can't come up is a loud error with the reason, never CPU numbers wearing a
// GPU label.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use crate::audio::SR;
use crate::config::{Config, Device};
use crate::wer::{self, Op};
use crate::{asr, clips};

enum Input { File(PathBuf), Clip(String) }

struct Opts {
    input:     Input,
    reference: Option<PathBuf>,
    model:     PathBuf,
    iters:     usize,
    threads:   Vec<Option<usize>>,
    text:      bool,
    device:    Device,
    lang:      String,
    /// Only the first N seconds: a push-to-talk-sized sentence. Ungraded.
    /// Several ("2,3.5,5") = a different length every run, like real phrases.
    /// That matters: a GPU builds kernels per input shape, a CPU doesn't care.
    first:     Vec<f64>,
    /// Sit idle this long before every warm run, like real dictation does
    /// between sentences. This is what lets a GPU clock down.
    gap_ms:    u64,
    /// The last N ms of each gap are "hotkey held": prime the GPU like the app does.
    press_ms:  u64,
    /// A/B: feed raw lengths on DirectML instead of the padded bucket.
    raw:       bool,
    json:      bool,
}

struct Run { threads: Option<usize>, load_ms: u128, cold_ms: u128, warm_ms: Vec<u128> }

const USAGE: &str = "usage: divinevoice bench <audio-file> [--reference script.txt]\n\
                     \x20      divinevoice bench --clip <id>      (a benchmark clip; graded against its script)\n\
                     \x20  options: --model <dir|catalogue-id>  --iters N  --thread-scan 1,2,4,8  --text\n\
                     \x20           --device cpu|cuda|directml  --lang auto|en|pt|ja…\n\
                     \x20           --first SECS[,SECS…] (sentence-sized slices, cycled)  --gap MS (idle before each run)  --json\n\
                     \x20           --press MS (end of each gap = hotkey held: prime the GPU)  --no-bucket (A/B)";

pub fn run(args: &[String], cfg: &Config) -> Result<()> {
    let o = parse(args, cfg)?;

    let t0 = Instant::now();
    let (pcm, label, script) = match &o.input {
        Input::File(p) => {
            let script = o.reference.as_ref()
                .map(|r| std::fs::read_to_string(r).with_context(|| format!("read {}", r.display())))
                .transpose()?;
            (clips::decode_file(p)?, p.display().to_string(), script)
        }
        Input::Clip(id) => {
            let (pcm, c) = clips::pcm(id)?;
            (pcm, format!("clip {id} ({:?}, {} words)", c.origin, c.words), Some(c.script))
        }
    };
    // A slice is a different exam than the script: no grading.
    let (takes, script) = if o.first.is_empty() {
        (vec![pcm], script)
    } else {
        (o.first.iter().map(|f| pcm[..pcm.len().min((f * SR as f64) as usize)].to_vec()).collect(), None)
    };
    let secs = takes.iter().map(|t| t.len() as f64 / SR as f64).sum::<f64>() / takes.len() as f64;

    println!("\n{} bench ({})\n──────────────────────────", crate::config::APP, o.device.label());
    println!("audio  {label}\nmodel  {} ({:?})\niters  {} warm (+1 cold){}\n",
        o.model.display(), crate::models::engine_of(&o.model), o.iters,
        if script.is_some() { " · graded" } else { "" });
    println!("decoded {secs:.2}s of audio in {} ms{}\n", t0.elapsed().as_millis(),
        if o.gap_ms > 0 { format!(" · idling {} ms before each warm run", o.gap_ms) } else { String::new() });

    let runs: Vec<Run> = o.threads.iter()
        .filter_map(|&t| {
            println!("═══ {} ═══", fmt_threads(t));
            bench_one(t, &o, &takes, script.as_deref())
                .inspect(|r| print_run(r, secs))
                .inspect(|r| if o.json { print_json(r, &o, secs) })
                .inspect_err(|e| println!("  ERROR: {}\n", asr::explain(o.device, &format!("{e:#}"))))
                .ok()
        })
        .collect();

    if runs.len() > 1 { print_table(&runs, secs); }
    Ok(())
}

fn parse(args: &[String], cfg: &Config) -> Result<Opts> {
    let mut file = None;
    let mut clip = None;
    let mut reference = None;
    let mut model = cfg.model_dir.clone();
    let mut iters = 10;
    let mut threads = vec![cfg.threads()];
    let mut text = false;
    let mut device = cfg.device;
    let mut lang = cfg.language.clone();
    let (mut first, mut gap_ms, mut json) = (Vec::new(), 0, false);
    let (mut press_ms, mut raw) = (0, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |flag: &str| it.next().ok_or_else(|| anyhow!("{flag} needs a value"));
        match a.as_str() {
            "--iters" | "-n" => iters = val("--iters")?.parse().context("--iters")?,
            "--thread-scan"  => threads = val("--thread-scan")?
                .split(',')
                .map(|x| x.trim().parse().map(Some))
                .collect::<Result<_, _>>()
                .context("--thread-scan wants e.g. 1,2,4,8")?,
            // A catalogue id ("parakeet-ultra") or any model folder.
            "--model" | "-m" => model = val("--model").map(|v| crate::models::find(v)
                .map_or_else(|| PathBuf::from(v), |s| s.dir()))?,
            "--clip"         => clip = Some(val("--clip")?.clone()),
            "--reference" | "-r" => reference = Some(PathBuf::from(val("--reference")?)),
            "--text" => text = true,
            "--lang" | "-l" => lang = val("--lang")?.to_lowercase(),
            "--first"  => first = val("--first")?.split(',')
                .map(|x| x.trim().parse::<f64>().map(|f| f.max(0.5)))
                .collect::<Result<_, _>>()
                .context("--first wants seconds, e.g. 5 or 1.8,3.1,4.6")?,
            "--gap"    => gap_ms = val("--gap")?.parse().context("--gap wants milliseconds")?,
            "--json"   => json = true,
            "--press"  => press_ms = val("--press")?.parse().context("--press wants milliseconds")?,
            "--no-bucket" => raw = true,
            "--device" | "-d" => device = val("--device").and_then(|v| Device::ALL.into_iter()
                .find(|d| d.label() == v.to_ascii_lowercase())
                .ok_or_else(|| anyhow!("--device wants cpu, cuda or directml, got {v:?}")))?,
            f if f.starts_with('-') => bail!("unknown flag {f}\n{USAGE}"),
            f if file.is_none() => file = Some(PathBuf::from(f)),
            f => bail!("one audio file at a time, got a second: {f}"),
        }
    }
    let input = match (file, clip) {
        (Some(_), Some(_)) => bail!("pick one: an audio file or --clip\n{USAGE}"),
        (Some(f), None) => {
            if !f.is_file() { bail!("not a file: {}", f.display()); }
            Input::File(f)
        }
        (None, Some(id)) => Input::Clip(id),
        (None, None) => {
            let have: Vec<_> = clips::list().into_iter().map(|c| c.id).collect();
            bail!("{USAGE}\nclips available: {}", if have.is_empty() { "(none yet)".into() } else { have.join(", ") });
        }
    };
    if threads.is_empty() { bail!("--thread-scan needs at least one number"); }
    Ok(Opts { input, reference, model, iters: iters.max(1), threads, text, device, lang, first, gap_ms, press_ms, raw, json })
}

/// "  wer   2.1%  (2/97)  steel→steal · −who · +the"
fn print_grade(script: &str, text: &str) {
    let s = wer::align(script, text);
    let misses: Vec<String> = s.ops.iter().filter_map(|o| match o {
        Op::Ok { .. }      => None,
        Op::Sub { r, h }   => Some(format!("{r}→{h}")),
        Op::Del { r }      => Some(format!("−{r}")),
        Op::Ins { h }      => Some(format!("+{h}")),
    }).collect();
    println!("  wer   {:.1}%  ({}/{})  {}", s.wer * 100.0, s.errors, s.ref_words,
        if misses.is_empty() { "perfect".into() } else { misses.join(" · ") });
}

// ── measure ──────────────────────────────────────────────────────────────────

fn ms_of<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_millis())
}

/// `takes` cycle through the warm runs (one take = the classic fixed clip).
fn bench_one(threads: Option<usize>, o: &Opts, takes: &[Vec<f32>], script: Option<&str>) -> Result<Run> {
    let (model, load_ms) = ms_of(|| asr::load_on(&o.model, threads, o.device, true, &o.lang));
    let mut model = model?.model;
    if o.raw { model.unbucket(); }
    println!("  load  {load_ms} ms");
    // Same as the app: silence first (on DirectML that picks the fast shape),
    // so "cold" below is your first real sentence, not ORT's setup.
    asr::warm_up(&mut model)?;

    let (text, cold_ms) = ms_of(|| asr::transcribe(&mut model, takes[0].clone()));
    let text = text?;
    println!("  cold  {cold_ms} ms → {:?}", clip(&text, 60));
    if let Some(s) = script { print_grade(s, &text); }
    if o.text { println!("\n--- transcript ---\n{text}\n------------------\n"); }

    let iters = o.iters;
    let warm_ms = (1..=iters)
        .map(|n| {
            let press = o.press_ms.min(o.gap_ms);
            std::thread::sleep(std::time::Duration::from_millis(o.gap_ms - press));
            // "Hotkey held": prime every 700 ms, exactly like asr::run does.
            let held = Instant::now();
            while held.elapsed().as_millis() < press as u128 {
                let _ = asr::prime(&mut model);
                std::thread::sleep(std::time::Duration::from_millis(700).min(
                    std::time::Duration::from_millis(press).saturating_sub(held.elapsed())));
            }
            let (r, ms) = ms_of(|| asr::transcribe(&mut model, takes[n % takes.len()].clone()));
            print!("\r  warm  {n}/{iters}  {ms} ms   ");
            let _ = std::io::stdout().flush();
            r.map(|_| ms)
        })
        .collect::<Result<Vec<_>>>()?;
    println!();
    Ok(Run { threads, load_ms, cold_ms, warm_ms })
}

// ── report ───────────────────────────────────────────────────────────────────

struct Stats { mean: f64, p50: u128, p95: u128, min: u128, max: u128 }

fn stats(v: &[u128]) -> Stats {
    let mut s = v.to_vec();
    s.sort_unstable();
    let n = s.len();
    if n == 0 { return Stats { mean: 0.0, p50: 0, p95: 0, min: 0, max: 0 }; }
    let pct = |q: f64| s[((n - 1) as f64 * q).round() as usize];
    Stats {
        mean: s.iter().sum::<u128>() as f64 / n as f64,
        p50: pct(0.50), p95: pct(0.95), min: s[0], max: s[n - 1],
    }
}

fn rtf(secs: f64, mean_ms: f64) -> f64 { secs * 1000.0 / mean_ms.max(0.001) }

fn fmt_threads(t: Option<usize>) -> String {
    t.map_or("ORT default threads".into(), |n| format!("{n} thread{}", if n == 1 { "" } else { "s" }))
}

fn print_run(r: &Run, secs: f64) {
    let s = stats(&r.warm_ms);
    println!("  warm  mean {:.1}  p50 {}  p95 {}  min {}  max {}  (ms)", s.mean, s.p50, s.p95, s.min, s.max);
    println!("  → {:.1}× real-time\n", rtf(secs, s.mean));
}

/// One line a script can collect: what the Speed page's reference numbers are made of.
fn print_json(r: &Run, o: &Opts, secs: f64) {
    let s = stats(&r.warm_ms);
    println!("{}", serde_json::json!({
        "model": crate::models::which(&o.model).map_or("custom", |m| m.id), "device": o.device,
        "audio_s": secs, "gap_ms": o.gap_ms, "press_ms": o.press_ms, "bucket": !o.raw, "threads": r.threads,
        "load_ms": r.load_ms, "cold_ms": r.cold_ms,
        "p50_ms": s.p50, "mean_ms": s.mean, "min_ms": s.min, "max_ms": s.max, "xrt": rtf(secs, s.mean),
    }));
}

fn print_table(runs: &[Run], secs: f64) {
    let means: Vec<f64> = runs.iter().map(|r| stats(&r.warm_ms).mean).collect();
    let best = means.iter().enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);

    println!("┌─────────┬──────────┬──────────┬──────────┬──────┬──────────┬───┐");
    println!("│ threads │ load ms  │ cold ms  │ warm avg │ p95  │ realtime │   │");
    println!("├─────────┼──────────┼──────────┼──────────┼──────┼──────────┼───┤");
    runs.iter().zip(&means).enumerate().for_each(|(i, (r, &m))| {
        println!("│ {:>7} │ {:>8} │ {:>8} │ {:>8.1} │ {:>4} │ {:>7.1}× │ {} │",
            r.threads.map_or("auto".into(), |n| n.to_string()), r.load_ms, r.cold_ms, m,
            stats(&r.warm_ms).p95, rtf(secs, m), if i == best { "★" } else { " " });
    });
    println!("└─────────┴──────────┴──────────┴──────────┴──────┴──────────┴───┘");

    // Scaling efficiency vs. single-thread, if you were brave enough to include it.
    if let Some(base) = runs.iter().zip(&means).find(|(r, _)| r.threads == Some(1)).map(|(_, &m)| m) {
        println!("\nvs. 1 thread ({base:.0} ms):");
        runs.iter().zip(&means)
            .filter_map(|(r, &m)| r.threads.filter(|&n| n > 1).map(|n| (n, base / m.max(0.001))))
            .for_each(|(n, x)| println!("  {n:>2} threads → {x:.2}×  ({:.0}% efficient)", x / n as f64 * 100.0));
    }
}

fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}
