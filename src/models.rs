// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * models.rs
 *
 * Version: @(#)models.rs 0.2.0 2026
 *
 * Description: The Model Shop.
 *              Six models, three families, exact byte sizes from HuggingFace.
 *              Every one carries its papers: who made it, license, published
 *              WER, when to pick it, when not to. Download stream to .part,
 *              resume with Range, rename when whole. 3 GB never sit in RAM.
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

// Four engines, all parakeet-rs:
//   Tdt       ParakeetTDT      vocab.txt + encoder-model*.onnx + decoder_joint-model*.onnx
//             → NVIDIA v3, and Moondream's Ultra (a post-trained v3, same graph)
//   Unified   ParakeetUnified  tokenizer.model + encoder*.onnx + decoder_joint*.onnx
//             → NVIDIA parakeet-unified-en (RNNT, English, offline + streaming)
//   Nemotron  Nemotron         same files as Unified + a config.json that says "nemotron"
//             → NVIDIA's cache-aware streaming RNNTs (English, and 3.5 multilingual)
//   Cohere    CohereASR        tokenizer.json + onnx/encoder_model* + onnx/decoder_model_merged*
//             → Cohere Transcribe, a 2B encoder-decoder. Needs a language; no auto-detect
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
pub enum Engine { Tdt, Unified, Nemotron, Cohere }

/// The papers. Everything the ⓘ panel on the Models page shows, so you can
/// pick without opening six browser tabs. Published numbers are the makers'
/// own (different test sets, so compare within a row, not across rows);
/// `ours` is what our Benchmark race measured on the built-in clips.
#[derive(Debug, Serialize)]
pub struct Facts {
    pub by:        &'static str,
    pub released:  &'static str,
    pub params:    &'static str,
    pub arch:      &'static str,
    pub license:   &'static str,
    pub languages: &'static str,
    pub published: &'static str,
    pub ours:      &'static str,
    pub pick:      &'static str,
    pub skip:      &'static str,
    pub card:      &'static str,
}

#[derive(Debug, Serialize)]
pub struct Spec {
    pub id:       &'static str,
    pub name:     &'static str,
    pub blurb:    &'static str,
    pub langs:    &'static str,
    pub engine:   Engine,
    /// Who exported the ONNX. Shown in the UI; we don't host weights.
    pub source:   &'static str,
    /// 1–5, for the little bars. Vibes, calibrated by ear and by `race`.
    pub accuracy: u8,
    pub speed:    u8,
    pub facts:    Facts,
    #[serde(skip)]
    pub repo:     &'static str,
    /// Folder inside the repo ("" = repo root), with trailing slash.
    #[serde(skip)]
    pub sub:      &'static str,
    /// Paths relative to `sub`; may contain a folder ("onnx/…"), mirrored locally.
    #[serde(skip)]
    pub files:    &'static [(&'static str, u64)],
}

const ISTUPAKOV_V3: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";
const PARAKEET_RS:  &str = "altunenes/parakeet-rs"; // the crate author's own exports
const UNIFIED_INT8: &str = "bobNight/parakeet-unified-en-0.6b-onnx";
const COHERE_ONNX:  &str = "onnx-community/cohere-transcribe-03-2026-ONNX";

// Raced on a 53 s English clip, 8 threads, parakeet-rs 0.4.3 (2026-10-28):
//   v3 fp32       1593 ms p50   1 word error ("hardest steal")
//   Ultra fp32    1567 ms       0
//   Ultra int8    1379 ms       4  ← Masterx's quant dropped/swapped words. Cut.
//   Unified fp32  1730 ms       1  (drops an aside, "Huh?")
//   Unified int8  1474 ms       1  ← quantisation cost it nothing here
// v3 int8 is cut too: Ultra replaces it.
// Not shipped: lokkju's int8 Nemotron (PolyForm Shield, a non-compete
// license) and Cohere fp32 (7.6 GB; int8 is the sane download).

/// First entry = the default for fresh installs.
pub const CATALOGUE: &[Spec] = &[
    Spec {
        id: "parakeet-ultra", name: "Parakeet Ultra",
        blurb: "Moondream's post-trained v3: fewer errors in all 25 languages, same speed. The pick.",
        langs: "25 languages", engine: Engine::Tdt, source: PARAKEET_RS,
        accuracy: 5, speed: 4, repo: PARAKEET_RS, sub: "parakeet-ultra/",
        facts: Facts {
            by: "Moondream, post-training NVIDIA's v3", released: "2026-09", params: "600M",
            arch: "FastConformer TDT (token-and-duration transducer)", license: "CC-BY-4.0",
            languages: "25 European, auto-detected per clip (incl. Russian, Ukrainian)",
            published: "English 5.80% avg WER on the Open ASR sets (v3: 6.26) · FLEURS 25 languages 9.55% (v3: 11.62)",
            ours: "AlterMann 3.4% · Leftmost 2.6% WER · CPU 26–39× real-time · DirectML 31–32×",
            pick: "The all-rounder. Multilingual, auto-detects, fast on a CPU, best accuracy we've measured.",
            skip: "English only and short on disk or bandwidth: Unified int8 is a quarter of the size.",
            card: "https://huggingface.co/moondream/parakeet-ultra",
        },
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
        facts: Facts {
            by: "NVIDIA · int8 by bobNight", released: "2026-04", params: "600M",
            arch: "FastConformer RNNT, offline + buffered streaming", license: "NVIDIA Open Model License",
            languages: "English only",
            published: "5.91% offline avg WER on the Open ASR sets (v2: 6.04, v3: 6.34). fp32 figure",
            ours: "AlterMann 3.4% · Leftmost 3.8% WER · CPU 45× real-time · DirectML 25–36×",
            pick: "English only, slow internet or small disk. A quarter of the size, and quantising cost it nothing in our race.",
            skip: "You ever speak anything but English.",
            card: "https://huggingface.co/nvidia/parakeet-unified-en-0.6b",
        },
        files: &[("encoder.int8.onnx", 42_606_669),
                 ("encoder.int8.onnx.data", 611_491_584),
                 ("decoder_joint.int8.onnx", 8_995_064),
                 ("tokenizer.model", 251_056)],
    },
    Spec {
        id: "parakeet-unified", name: "Parakeet Unified",
        blurb: "NVIDIA's newest English model, full precision. Lowest published English WER of the Parakeets.",
        langs: "English", engine: Engine::Unified, source: PARAKEET_RS,
        accuracy: 5, speed: 4, repo: PARAKEET_RS, sub: "parakeet-unified-en-0.6b/",
        facts: Facts {
            by: "NVIDIA", released: "2026-04", params: "600M",
            arch: "FastConformer RNNT, offline + buffered streaming", license: "NVIDIA Open Model License",
            languages: "English only",
            published: "5.91% offline avg WER on the Open ASR sets (v2: 6.04, v3: 6.34)",
            ours: "AlterMann 3.4% · Leftmost 3.8% WER · CPU 34–39× real-time · DirectML 17–20×",
            pick: "English, and you want the reference weights rather than a quantised copy.",
            skip: "Disk matters: the int8 build matched it word for word in our race at a quarter of the size.",
            card: "https://huggingface.co/nvidia/parakeet-unified-en-0.6b",
        },
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
        facts: Facts {
            by: "NVIDIA", released: "2025-08", params: "600M",
            arch: "FastConformer TDT (token-and-duration transducer)", license: "CC-BY-4.0",
            languages: "25 European, auto-detected per clip",
            published: "6.34% English avg WER on the Open ASR sets",
            ours: "AlterMann 3.4% · Leftmost 2.6% WER · CPU 40× real-time · DirectML 123–145×: the GPU's favourite",
            pick: "You want the stock reference, e.g. to compare against other tools that ship it.",
            skip: "You want accuracy: Ultra is this exact model, post-trained, with fewer errors everywhere.",
            card: "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3",
        },
        files: &[("encoder-model.onnx", 41_770_866),
                 ("encoder-model.onnx.data", 2_435_420_160),
                 ("decoder_joint-model.onnx", 72_520_893),
                 ("vocab.txt", 93_939)],
    },
    Spec {
        id: "cohere-transcribe", name: "Cohere Transcribe · int8",
        blurb: "Cohere's 2B model, #1 on the Open ASR leaderboard at release. Fewest errors in our race. Slow: 14 languages, set yours.",
        langs: "14 languages", engine: Engine::Cohere, source: COHERE_ONNX,
        accuracy: 5, speed: 1, repo: COHERE_ONNX, sub: "",
        facts: Facts {
            by: "Cohere Labs · ONNX by onnx-community", released: "2026-03", params: "2B",
            arch: "Conformer encoder + Transformer decoder (attention encoder-decoder)", license: "Apache-2.0",
            languages: "en fr de it es pt el nl pl zh ja ko vi ar. NO auto-detect: set Core ▸ language (auto = English)",
            published: "5.42% avg WER, #1 on the Open ASR leaderboard on 2026-03-26 · LibriSpeech clean 1.25%, other 2.37%",
            ours: "AlterMann 1.7% · Leftmost 2.6% WER: the best we've measured · CPU ~2× real-time on long clips, but a 5 s sentence takes 5.2 s · DirectML not supported",
            pick: "Accuracy above everything, one language at a time, and patience (or an NVIDIA card on CUDA). Fewest errors in our race. Covers Chinese, Japanese, Korean, Arabic, Vietnamese.",
            skip: "Speed matters: ~20× slower than Parakeet on CPU, and it won't run on DirectML. Also skip if you switch languages mid-sentence; its card warns it may transcribe background noise.",
            card: "https://huggingface.co/CohereLabs/cohere-transcribe-03-2026",
        },
        files: &[("onnx/encoder_model_quantized.onnx", 1_420_203),
                 ("onnx/encoder_model_quantized.onnx_data", 2_096_573_952),
                 ("onnx/encoder_model_quantized.onnx_data_1", 774_430_720),
                 ("onnx/decoder_model_merged_quantized.onnx", 196_039),
                 ("onnx/decoder_model_merged_quantized.onnx_data", 195_903_488),
                 ("tokenizer.json", 1_152_395)],
    },
    Spec {
        id: "nemotron-3.5", name: "Nemotron 3.5",
        blurb: "NVIDIA's 2026 multilingual streaming model: 40 locales incl. Japanese, Chinese, Arabic, Hindi.",
        langs: "40 languages", engine: Engine::Nemotron, source: PARAKEET_RS,
        accuracy: 5, speed: 2, repo: PARAKEET_RS, sub: "nemotron-3.5-asr-streaming-0.6b-onnx/",
        facts: Facts {
            by: "NVIDIA", released: "2026-06", params: "600M",
            arch: "Cache-aware FastConformer RNNT + language-ID prompt (streaming)", license: "OpenMDW-1.1",
            languages: "40 locales, 32 ready out of the box. Auto-detects, more accurate pinned (Core ▸ language)",
            published: "FLEURS English 7.9–9.4% WER depending on latency · the 19 transcription-ready locales average 8.8–10.4%",
            ours: "AlterMann 1.7% · Leftmost 2.6% WER: tied best · CPU 7–8× real-time · DirectML 4–5×, slower than CPU",
            pick: "Your language isn't one of Ultra's 25 (Turkish, Arabic, Hindi, Japanese, Korean, Chinese, Vietnamese…), or you want top English accuracy and can spare ~0.5 s per sentence.",
            skip: "Speed matters: ~5× slower than Parakeet on CPU, and DirectML doesn't help it.",
            card: "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b",
        },
        files: &[("encoder.onnx", 4_161_952),
                 ("encoder.onnx.data", 2_495_361_024),
                 ("decoder_joint.onnx", 97_653_235),
                 ("tokenizer.model", 406_554),
                 ("config.json", 2_995)],
    },
    Spec {
        id: "nemotron-en", name: "Nemotron Streaming · English",
        blurb: "NVIDIA's 2026 low-latency English model, trained on 530k hours. Built for live captions.",
        langs: "English", engine: Engine::Nemotron, source: PARAKEET_RS,
        accuracy: 4, speed: 2, repo: PARAKEET_RS, sub: "nemotron-speech-streaming-en-0.6b/",
        facts: Facts {
            by: "NVIDIA", released: "2026-03", params: "600M",
            arch: "Cache-aware FastConformer RNNT (streaming)", license: "NVIDIA Open Model License",
            languages: "English only",
            published: "6.93% avg WER on the Open ASR sets at 1.12 s chunks, 7.07% at 0.56 s (scored without punctuation)",
            ours: "AlterMann 1.7% · Leftmost 3.8% WER · CPU 8× real-time · DirectML slower than CPU",
            pick: "Curiosity, or a head start: it's the engine for live, as-you-talk dictation, which isn't in DivineVoice yet.",
            skip: "Push-to-talk today: Nemotron 3.5 is as fast and more accurate, Parakeet is 5× faster. DirectML doesn't help it.",
            card: "https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b",
        },
        files: &[("encoder.onnx", 4_026_060),
                 ("encoder.onnx.data", 2_477_522_944),
                 ("decoder_joint.onnx", 35_842_420),
                 ("tokenizer.model", 251_056),
                 ("config.json", 980)],
    },
];

/// Which engine a model folder needs. A catalogue folder says so itself; for
/// anything else we read the files:
///   tokenizer.json + decoder_model_merged*  → Cohere
///   config.json naming a nemotron           → Nemotron (same files as Unified otherwise,
///                                             and the encoder graphs share input names, so
///                                             the export's config is the only honest tell)
///   tokenizer.model                         → Unified (TDT exports never ship one)
///   else                                    → TDT
pub fn engine_of(dir: &Path) -> Engine {
    which(dir).map_or_else(|| sniff(dir), |s| s.engine)
}

fn sniff(dir: &Path) -> Engine {
    let merged = |d: PathBuf| fs::read_dir(d).into_iter().flatten().flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("decoder_model_merged"));
    let config = fs::read_to_string(dir.join("config.json")).unwrap_or_default().to_lowercase();
    if dir.join("tokenizer.json").is_file() && (merged(dir.to_path_buf()) || merged(dir.join("onnx"))) {
        Engine::Cohere
    } else if config.contains("\"model_name\"") && config.contains("nemotron") {
        Engine::Nemotron
    } else if dir.join("tokenizer.model").is_file() {
        Engine::Unified
    } else {
        Engine::Tdt
    }
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
        if let Some(up) = final_path.parent() { fs::create_dir_all(up)?; } // "onnx/…"
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
                Engine::Tdt      => assert!(has(s, "vocab.txt") && !has(s, "tokenizer.model"), "{}", s.id),
                Engine::Unified  => assert!(has(s, "tokenizer.model") && !has(s, "config.json"), "{}", s.id),
                Engine::Nemotron => assert!(has(s, "tokenizer.model") && has(s, "config.json"), "{}", s.id),
                Engine::Cohere   => assert!(has(s, "tokenizer.json"), "{}", s.id),
            }
            let f = &s.facts;
            assert!([f.by, f.released, f.params, f.arch, f.license, f.languages, f.published, f.pick, f.skip]
                .iter().all(|x| !x.is_empty()), "{}: every model carries its papers", s.id);
            assert!(f.card.starts_with("https://huggingface.co/"), "{}", s.id);
        }
    }

    /// Custom folders: the files decide. Nemotron and Unified ship identical
    /// file names; only the export's config.json tells them apart.
    #[test]
    fn sniff_tells_the_families_apart() {
        let base = std::env::temp_dir().join(format!("dv-sniff-{}", std::process::id()));
        let mk = |name: &str, files: &[(&str, &str)]| {
            let d = base.join(name);
            for (f, body) in files {
                let p = d.join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, body).unwrap();
            }
            d
        };
        let tdt = mk("tdt", &[("vocab.txt", "")]);
        let uni = mk("uni", &[("tokenizer.model", "")]);
        let nem = mk("nem", &[("tokenizer.model", ""), ("config.json", r#"{"model_name": "nemotron-3.5-asr"}"#)]);
        let coh = mk("coh", &[("tokenizer.json", "{}"), ("onnx/decoder_model_merged_quantized.onnx", "")]);
        assert_eq!(sniff(&tdt), Engine::Tdt);
        assert_eq!(sniff(&uni), Engine::Unified);
        assert_eq!(sniff(&nem), Engine::Nemotron);
        assert_eq!(sniff(&coh), Engine::Cohere);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn which_matches_case_and_slashes() {
        let d = CATALOGUE[1].dir();
        let shouty = PathBuf::from(format!("{}\\", d.to_string_lossy().to_uppercase()));
        assert_eq!(which(&shouty).map(|s| s.id), Some(CATALOGUE[1].id));
        assert!(which(Path::new(r"A:\elsewhere")).is_none());
    }
}
