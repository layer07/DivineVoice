// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * bench.rs
 *
 * Version: @(#)bench.rs 0.2.0 2026
 *
 * Description: The Truth Machine.
 *              Decode audio file. Load model. Run cold, run warm, run again.
 *              Print p50, p95, realtime factor. Thread scan find the knee of your
 *              CPU. This the tool that told the GPU to go home.
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
// This is the tool that told us to stop bothering with the GPU.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::audio::SR;
use crate::config::Config;
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
}

struct Run { threads: Option<usize>, load_ms: u128, cold_ms: u128, warm_ms: Vec<u128> }

const USAGE: &str = "usage: divinevoice bench <audio-file> [--reference script.txt]\n\
                     \x20      divinevoice bench --clip <id>      (a benchmark clip; graded against its script)\n\
                     \x20  options: --model <dir|catalogue-id>  --iters N  --thread-scan 1,2,4,8  --text";

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
    let secs = pcm.len() as f64 / SR as f64;

    println!("\n{} bench (CPU)\n──────────────────────────", crate::config::APP);
    println!("audio  {label}\nmodel  {} ({:?})\niters  {} warm (+1 cold){}\n",
        o.model.display(), crate::models::engine_of(&o.model), o.iters,
        if script.is_some() { " · graded" } else { "" });
    println!("decoded {secs:.2}s of audio in {} ms\n", t0.elapsed().as_millis());

    let runs: Vec<Run> = o.threads.iter()
        .filter_map(|&t| {
            println!("═══ {} ═══", fmt_threads(t));
            bench_one(t, &pcm, o.iters, &o.model, o.text, script.as_deref())
                .inspect(|r| print_run(r, secs))
                .inspect_err(|e| println!("  ERROR: {e:#}\n"))
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
    Ok(Opts { input, reference, model, iters: iters.max(1), threads, text })
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

fn bench_one(
    threads: Option<usize>, pcm: &[f32], iters: usize, dir: &Path, full_text: bool, script: Option<&str>,
) -> Result<Run> {
    let (model, load_ms) = ms_of(|| asr::load(dir, threads));
    let mut model = model?;
    println!("  load  {load_ms} ms");

    let (text, cold_ms) = ms_of(|| asr::transcribe(&mut model, pcm.to_vec()));
    let text = text?;
    println!("  cold  {cold_ms} ms → {:?}", clip(&text, 60));
    if let Some(s) = script { print_grade(s, &text); }
    if full_text { println!("\n--- transcript ---\n{text}\n------------------\n"); }

    let warm_ms = (1..=iters)
        .map(|n| {
            let (r, ms) = ms_of(|| asr::transcribe(&mut model, pcm.to_vec()));
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
