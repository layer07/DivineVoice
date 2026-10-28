// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * models.rs
 *
 * Version: @(#)models.rs 0.2.0 2026
 *
 * Description: The Model Shop.
 *              Four Parakeet builds, exact byte sizes from HuggingFace. Download
 *              stream to .part, resume with Range, rename when whole. 2.5 GB
 *              never sit in RAM like an animal.
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

// Two engines, both parakeet-rs:
//   Tdt      ParakeetTDT      vocab.txt + encoder-model*.onnx + decoder_joint-model*.onnx
//            → NVIDIA v3, and Moondream's Ultra (a post-trained v3, same graph)
//   Unified  ParakeetUnified  tokenizer.model + encoder*.onnx + decoder_joint*.onnx
//            → NVIDIA parakeet-unified-en (RNNT, English, offline + streaming)
// int8 builds are the same models with quantised weights. Sizes are the exact
// byte counts HuggingFace reports, so "is it installed" is a stat() per file,
// not a hash. `source` credits whoever did the ONNX export: we don't host
// weights, we just know where the good ones live.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine { Tdt, Unified }

#[derive(Debug, Serialize)]
pub struct Spec {
    pub id:       &'static str,
    pub name:     &'static str,
    pub blurb:    &'static str,
    pub langs:    &'static str,
    pub engine:   Engine,
    /// Who exported the ONNX. Shown in the UI; we don't host weights.
    pub source:   &'static str,
    /// 1–5, for the little bars. Vibes, calibrated by ear and by `bench`.
    pub accuracy: u8,
    pub speed:    u8,
    #[serde(skip)]
    pub repo:     &'static str,
    /// Folder inside the repo ("" = repo root), with trailing slash.
    #[serde(skip)]
    pub sub:      &'static str,
    #[serde(skip)]
    pub files:    &'static [(&'static str, u64)],
}

const ISTUPAKOV_V3: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";
const PARAKEET_RS:  &str = "altunenes/parakeet-rs"; // the crate author's own exports
const UNIFIED_INT8: &str = "bobNight/parakeet-unified-en-0.6b-onnx";

// Raced on a 53 s English clip, 8 threads, parakeet-rs 0.4.3 (2026-10-28):
//   v3 fp32       1593 ms p50   1 word error ("hardest steal")
//   Ultra fp32    1567 ms       0
//   Ultra int8    1379 ms       4  ← Masterx's quant dropped/swapped words. Cut.
//   Unified fp32  1730 ms       1  (drops an aside, "Huh?")
//   Unified int8  1474 ms       1  ← quantisation cost it nothing here
// v3 int8 is cut too: Ultra replaces it.

/// First entry = the default for fresh installs.
pub const CATALOGUE: &[Spec] = &[
    Spec {
        id: "parakeet-ultra", name: "Parakeet Ultra",
        blurb: "Moondream's post-trained v3: fewer errors in all 25 languages, same speed. The pick.",
        langs: "25 languages", engine: Engine::Tdt, source: PARAKEET_RS,
        accuracy: 5, speed: 4, repo: PARAKEET_RS, sub: "parakeet-ultra/",
        files: &[("encoder-model.onnx", 3_437_572),
                 ("encoder-model.onnx.data", 2_476_376_064),
                 ("decoder_joint-model.onnx", 72_584_074),
                 ("vocab.txt", 93_939)],
    },
    Spec {
        id: "parakeet-unified-int8", name: "Parakeet Unified · int8",
        blurb: "NVIDIA's newest English model (2026), quantised. Best small download.",
        langs: "English", engine: Engine::Unified, source: UNIFIED_INT8,
        accuracy: 5, speed: 5, repo: UNIFIED_INT8, sub: "",
        files: &[("encoder.int8.onnx", 42_606_669),
                 ("encoder.int8.onnx.data", 611_491_584),
                 ("decoder_joint.int8.onnx", 8_995_064),
                 ("tokenizer.model", 251_056)],
    },
    Spec {
        id: "parakeet-unified", name: "Parakeet Unified",
        blurb: "NVIDIA's newest English model, full precision. Lowest published English WER.",
        langs: "English", engine: Engine::Unified, source: PARAKEET_RS,
        accuracy: 5, speed: 4, repo: PARAKEET_RS, sub: "parakeet-unified-en-0.6b/",
        files: &[("encoder.onnx", 4_137_221),
                 ("encoder.onnx.data", 2_478_047_232),
                 ("decoder_joint.onnx", 35_842_420),
                 ("tokenizer.model", 251_056)],
    },
    Spec {
        id: "parakeet-v3", name: "Parakeet v3",
        blurb: "Stock NVIDIA v3, full precision. The reference everyone benchmarks against.",
        langs: "25 languages", engine: Engine::Tdt, source: ISTUPAKOV_V3,
        accuracy: 4, speed: 4, repo: ISTUPAKOV_V3, sub: "",
        files: &[("encoder-model.onnx", 41_770_866),
                 ("encoder-model.onnx.data", 2_435_420_160),
                 ("decoder_joint-model.onnx", 72_520_893),
                 ("vocab.txt", 93_939)],
    },
];

/// Which engine a model folder needs. tokenizer.model is the tell: TDT
/// exports never ship one (some Unified exports ship a vocab.txt too, so we
/// can't go by that).
pub fn engine_of(dir: &Path) -> Engine {
    if dir.join("tokenizer.model").is_file() { Engine::Unified } else { Engine::Tdt }
}

pub fn find(id: &str) -> Option<&'static Spec> { CATALOGUE.iter().find(|s| s.id == id) }

pub fn root() -> PathBuf { crate::config::home().join("models") }

impl Spec {
    pub fn dir(&self) -> PathBuf { root().join(self.id) }
    pub fn size(&self) -> u64 { self.files.iter().map(|(_, n)| n).sum() }

    pub fn installed(&self) -> bool {
        let d = self.dir();
        self.files.iter().all(|(f, n)| fs::metadata(d.join(f)).is_ok_and(|m| m.len() == *n))
    }

    /// Bytes already on disk (finished files + .part leftovers), for resume
    /// and for the progress bar's starting point.
    pub fn on_disk(&self) -> u64 {
        let d = self.dir();
        self.files.iter().map(|(f, n)| {
            let len = |p: PathBuf| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            len(d.join(f)).max(len(part(&d, f))).min(*n)
        }).sum()
    }

    pub fn delete(&self) -> Result<()> {
        let d = self.dir();
        if d.exists() { fs::remove_dir_all(&d).with_context(|| format!("rm -r {}", d.display()))?; }
        Ok(())
    }
}

/// Which catalogue entry (if any) lives at `dir`.
pub fn which(dir: &Path) -> Option<&'static Spec> {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    CATALOGUE.iter().find(|s| norm(&s.dir()) == norm(dir))
}

fn part(dir: &Path, file: &str) -> PathBuf { dir.join(format!("{file}.part")) }

/// Download everything `spec` needs. Resumes .part files with HTTP Range,
/// verifies sizes, renames into place. `progress(done, total)` fires at most
/// ~10×/s. Set `cancel` and it bails within one 256 KiB chunk.
pub fn download(spec: &Spec, cancel: &AtomicBool, progress: impl Fn(u64, u64)) -> Result<()> {
    let dir = spec.dir();
    fs::create_dir_all(&dir)?;
    let total = spec.size();
    let mut done_before = 0u64;
    let mut last_tick = Instant::now() - Duration::from_secs(1);

    for &(file, size) in spec.files {
        let final_path = dir.join(file);
        if fs::metadata(&final_path).is_ok_and(|m| m.len() == size) {
            done_before += size;
            continue;
        }

        let tmp = part(&dir, file);
        let have = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0).min(size);
        let url = format!("https://huggingface.co/{}/resolve/main/{}{file}", spec.repo, spec.sub);
        info!("models: {file} ← {url} (resume at {have})");

        let mut req = ureq::get(&url).header("User-Agent", concat!("DivineVoice/", env!("CARGO_PKG_VERSION")));
        if have > 0 { req = req.header("Range", format!("bytes={have}-")); }
        let resp = req.call().with_context(|| format!("GET {file}"))?;

        // 206 = resume honoured. 200 = server ignored Range; start over.
        let (mut out, mut got) = match resp.status().as_u16() {
            206 => (OpenOptions::new().append(true).open(&tmp)?, have),
            200 => (File::create(&tmp)?, 0),
            s   => bail!("{file}: HTTP {s}"),
        };

        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) { bail!("cancelled"); }
            let n = body.read(&mut buf).with_context(|| format!("reading {file}"))?;
            if n == 0 { break; }
            out.write_all(&buf[..n])?;
            got += n as u64;
            if last_tick.elapsed() >= Duration::from_millis(100) {
                last_tick = Instant::now();
                progress(done_before + got, total);
            }
        }
        out.flush()?;
        drop(out);

        if got != size {
            bail!("{file}: got {got} bytes, expected {size} — truncated, will resume next try");
        }
        fs::rename(&tmp, &final_path).map_err(|e| anyhow!("rename {}: {e}", tmp.display()))?;
        done_before += size;
    }
    progress(total, total);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_sane() {
        let mut ids: Vec<_> = CATALOGUE.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOGUE.len());
        assert!(CATALOGUE.iter().all(|s| (1..=5).contains(&s.accuracy) && (1..=5).contains(&s.speed)));
        assert!(CATALOGUE.iter().all(|s| s.sub.is_empty() || s.sub.ends_with('/')));
    }

    /// Each entry ships exactly the vocabulary file its engine loads; that's
    /// also what engine_of() keys on after download.
    #[test]
    fn catalogue_files_match_engine() {
        let has = |s: &Spec, f: &str| s.files.iter().any(|(n, _)| *n == f);
        for s in CATALOGUE {
            match s.engine {
                Engine::Tdt     => assert!(has(s, "vocab.txt") && !has(s, "tokenizer.model"), "{}", s.id),
                Engine::Unified => assert!(has(s, "tokenizer.model"), "{}", s.id),
            }
        }
    }

    #[test]
    fn which_matches_case_and_slashes() {
        let d = CATALOGUE[1].dir();
        let shouty = PathBuf::from(format!("{}\\", d.to_string_lossy().to_uppercase()));
        assert_eq!(which(&shouty).map(|s| s.id), Some(CATALOGUE[1].id));
        assert!(which(Path::new(r"A:\elsewhere")).is_none());
    }
}
