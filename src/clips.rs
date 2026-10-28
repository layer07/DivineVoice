// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * clips.rs
 *
 * Version: @(#)clips.rs 0.2.0 2026
 *
 * Description: The Tape Library.
 *              Benchmark clips: audio plus the exact script it reads. Baked
 *              into the exe from bench/ at build time, or dropped in
 *              %APPDATA%\DivineVoice\bench at runtime (yours win on a name
 *              clash). Same tape for everyone, so numbers compare across
 *              machines. Also the decoder: mp3/wav/flac/ogg/m4a to 16 kHz.
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

// A clip is `<id>.<mp3|wav|flac|ogg|m4a>` + `<id>.txt`. The .txt is the
// reference the judge grades against, so it must be EXACTLY what the voice
// says: numbers as words ("three in the morning"), no stage directions.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::audio::SR;
use crate::dsp;

mod baked { include!(concat!(env!("OUT_DIR"), "/clips.rs")); }

const AUDIO: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin { Builtin, User }

#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    pub id:     String,
    pub title:  String,
    pub ext:    String,
    pub origin: Origin,
    pub words:  usize,
    pub script: String,
}

/// %APPDATA%\DivineVoice\bench — your own clips, no rebuild needed.
pub fn user_dir() -> PathBuf { crate::config::home().join("bench") }

/// "devops-hard" → "Devops Hard". Good enough for a tab label.
fn title(id: &str) -> String {
    id.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| { let mut cs = w.chars(); cs.next().map_or_else(String::new, |f| f.to_uppercase().chain(cs).collect()) })
        .collect::<Vec<_>>()
        .join(" ")
}

fn clip(id: &str, ext: &str, origin: Origin, script: &str) -> Clip {
    let script = script.trim().to_owned();
    Clip { id: id.into(), title: title(id), ext: ext.into(), origin,
           words: script.split_whitespace().count(), script }
}

fn user_clips() -> Vec<(Clip, PathBuf)> {
    let mut v: Vec<_> = std::fs::read_dir(user_dir()).into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            if !AUDIO.contains(&ext.as_str()) { return None; }
            let id = p.file_stem()?.to_str()?.to_owned();
            let script = std::fs::read_to_string(p.with_extension("txt")).ok()?;
            Some((clip(&id, &ext, Origin::User, &script), p))
        })
        .collect();
    v.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    v
}

/// Every clip: built-ins first, then yours. Same id → yours replaces ours.
pub fn list() -> Vec<Clip> {
    let user: Vec<Clip> = user_clips().into_iter().map(|(c, _)| c).collect();
    let shadowed: Vec<String> = user.iter().map(|c| c.id.clone()).collect();
    baked::BUILTIN.iter()
        .filter(|(id, ..)| !shadowed.iter().any(|s| s == id))
        .map(|&(id, ext, _, script)| clip(id, ext, Origin::Builtin, script))
        .chain(user)
        .collect()
}

pub fn find(id: &str) -> Option<Clip> { list().into_iter().find(|c| c.id == id) }

/// Raw audio bytes for playback (served to the page at dv://…/clip/<id>).
pub fn audio(id: &str) -> Option<(Cow<'static, [u8]>, &'static str)> {
    let mime = |ext: &str| match ext {
        "mp3" => "audio/mpeg", "wav" => "audio/wav", "flac" => "audio/flac",
        "ogg" => "audio/ogg", _ => "audio/mp4",
    };
    if let Some((c, path)) = user_clips().into_iter().find(|(c, _)| c.id == id) {
        return std::fs::read(path).ok().map(|b| (Cow::Owned(b), mime(&c.ext)));
    }
    baked::BUILTIN.iter().find(|(i, ..)| *i == id).map(|&(_, ext, bytes, _)| (Cow::Borrowed(bytes), mime(ext)))
}

/// The clip as model food: 16 kHz mono f32, plus its script.
pub fn pcm(id: &str) -> Result<(Vec<f32>, Clip)> {
    let c = find(id).ok_or_else(|| anyhow!("no benchmark clip called {id:?}"))?;
    let (bytes, _) = audio(id).ok_or_else(|| anyhow!("clip {id} has no audio"))?;
    let pcm = decode_source(Box::new(Cursor::new(bytes.into_owned())), Some(&c.ext))
        .with_context(|| format!("decoding clip {id}"))?;
    Ok((pcm, c))
}

// ── decode ───────────────────────────────────────────────────────────────────

pub fn decode_file(path: &Path) -> Result<Vec<f32>> {
    let src = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    decode_source(Box::new(src), path.extension().and_then(|s| s.to_str()))
}

fn decode_source(src: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Vec<f32>> {
    let mut hint = Hint::new();
    if let Some(ext) = ext { hint.with_extension(ext); }

    let mut fmt = symphonia::default::get_probe()
        .format(&hint, MediaSourceStream::new(src, Default::default()),
                &FormatOptions::default(), &MetadataOptions::default())
        .context("unrecognised audio format")?
        .format;
    let track = fmt.default_track().ok_or_else(|| anyhow!("no audio track"))?.clone();
    let rate  = track.codec_params.sample_rate.ok_or_else(|| anyhow!("no sample rate"))?;
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("no decoder for this codec")?;

    let mut mono = Vec::with_capacity(rate as usize * 10);
    loop {
        let pkt = match fmt.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(_) | SymErr::ResetRequired) => break, // EOF, symphonia-style
            Err(e) => return Err(e.into()),
        };
        if pkt.track_id() != track.id { continue; }
        match dec.decode(&pkt) {
            Ok(buf) => append_mono(buf, &mut mono),
            Err(SymErr::DecodeError(_)) => continue, // one bad frame isn't the end of the world
            Err(SymErr::IoError(_)) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(dsp::resample(&mono, rate, SR))
}

fn append_mono(buf: AudioBufferRef<'_>, out: &mut Vec<f32>) {
    // Symphonia gives planar buffers in 10 sample types. One macro, ten arms.
    macro_rules! mix { ($b:expr) => {{
        let ch = $b.spec().channels.count().max(1);
        out.extend((0..$b.frames()).map(|i| {
            (0..ch).map(|c| IntoSample::<f32>::into_sample($b.chan(c)[i])).sum::<f32>() / ch as f32
        }));
    }}}
    use AudioBufferRef::*;
    match buf {
        F32(b) => mix!(b), F64(b) => mix!(b),
        S8(b)  => mix!(b), S16(b) => mix!(b), S24(b) => mix!(b), S32(b) => mix!(b),
        U8(b)  => mix!(b), U16(b) => mix!(b), U24(b) => mix!(b), U32(b) => mix!(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title("classic"), "Classic");
        assert_eq!(title("devops-hard"), "Devops Hard");
        assert_eq!(title("ill_met_by_silicon"), "Ill Met By Silicon");
    }

    /// Every baked clip must decode and be roughly as long as its script
    /// suggests (≈ 1–6 words/s). Catches a swapped .txt or a truncated mp3.
    #[test]
    fn baked_clips_decode_and_match_their_scripts() {
        for &(id, ext, bytes, script) in baked::BUILTIN {
            let pcm = decode_source(Box::new(Cursor::new(bytes.to_vec())), Some(ext)).unwrap();
            let secs = pcm.len() as f64 / SR as f64;
            let wps = script.split_whitespace().count() as f64 / secs;
            assert!((1.0..=6.0).contains(&wps), "{id}: {wps:.2} words/s over {secs:.1}s, script/audio mismatch?");
        }
    }
}
