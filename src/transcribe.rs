// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * transcribe.rs
 *
 * Version: @(#)transcribe.rs 0.5.0 2026
 *
 * Description: The Inbox For Voice Notes.
 *              No time listen to WhatsApp audio? Drop it here. One worker, own
 *              copy of the model, so you still dictate while podcast chew. Long
 *              audio cut in quiet spots, sentence times kept. Out come TXT, MD,
 *              SRT: one document, or one file each. Same engine, CLI too.
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

//   drop · browse · `divinevoice transcribe` ──► queue ──► one worker ──► text
//
// The worker exists only while there's work. It loads its OWN copy of the
// model (the dictation thread keeps its own: you can dictate while a podcast
// chews), keeps a CPU model warm for a minute after the queue runs dry, then
// lets it go. A GPU model is let go at once, and while one is up the dictation
// model is parked off the card, exactly like a Benchmark race.
//
// Long audio is cut into ≤30 s pieces at the quiet spots (dsp::split_at_quiet:
// the breath between words, not the middle of one). Each piece keeps its
// sentence timestamps, shifted to where it sits in the file: that's the SRT.
//
// The worker's exit is decided under the same lock `add()` takes, so a file
// dropped at the wrong millisecond can't be stranded in a queue nobody reads.
//
// Shape:
//   pure:   is_audio, natural_cmp, ordered, render, clock, srt_time, ymd, unique
//   engine: hear() — one file, start to finish (the worker and the CLI share it)
//   glue:   add / stop / retry / remove / clear / copy / save / browse / view

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering as Cmp;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use crate::asr::{self, Seg};
use crate::audio::SR;
use crate::config::{Config, Device};
use crate::state::State;
use crate::{clips, dsp, models, race, ui};

/// What we'll take: everything symphonia + libopus decode.
pub const AUDIO_EXTS: &[&str] = &["ogg", "opus", "oga", "mp3", "m4a", "mp4", "aac", "wav", "flac", "webm", "mka", "mkv"];
/// The longest piece one inference sees. Parakeet's ONNX export runs full
/// attention: an hour in one go is a memory bill, not a transcript. But every
/// cut costs a little (the model ends a piece with a full stop, mid-sentence
/// or not), so voice notes under two minutes go in whole, like dictation does.
const PIECE_S: usize = 120;
/// How long a CPU model stays warm after the queue runs dry.
const LINGER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format { #[default] Txt, Md, Srt }

impl Format {
    pub fn ext(self) -> &'static str { match self { Self::Txt => "txt", Self::Md => "md", Self::Srt => "srt" } }
}

/// Merge order. Name is chronological for WhatsApp's own file names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Order { #[default] Name, Modified, Added }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase { Queued, Decoding, Running, Done, Failed, Stopped }

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id:       u64,
    #[serde(skip)]
    pub path:     PathBuf,
    pub name:     String,
    pub folder:   String,
    pub bytes:    u64,
    /// Unix ms, from the file system.
    pub modified: Option<u64>,
    pub phase:    Phase,
    /// Pieces transcribed / pieces in the file.
    pub done:     usize,
    pub of:       usize,
    pub audio_ms: f64,
    pub asr_ms:   f64,
    pub model:    String,
    pub device:   String,
    pub text:     String,
    /// Sentences with times, for SRT. Heavy for a podcast; the page doesn't need them.
    #[serde(skip)]
    pub segs:     Vec<Seg>,
    pub error:    Option<String>,
}

impl Job {
    fn new(id: u64, path: PathBuf) -> Self {
        let meta = std::fs::metadata(&path).ok();
        Self {
            id,
            name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            folder: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            bytes: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
            path,
            phase: Phase::Queued, done: 0, of: 0, audio_ms: 0.0, asr_ms: 0.0,
            model: String::new(), device: String::new(), text: String::new(), segs: Vec::new(), error: None,
        }
    }

    fn stem(&self) -> &str { Path::new(&self.name).file_stem().and_then(|s| s.to_str()).unwrap_or(&self.name) }
    fn active(&self) -> bool { matches!(self.phase, Phase::Decoding | Phase::Running) }
}

/// The Files tab's half of the world.
#[derive(Default)]
pub struct Files {
    pub jobs: Vec<Job>,
    next:     u64,
    running:  bool,
}

// ── the pure part ────────────────────────────────────────────────────────────

pub fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Case-insensitive, and runs of digits compare as numbers: "voice 9" before
/// "voice 10", whatever the zero-padding. Pure.
pub fn natural_cmp(a: &str, b: &str) -> Cmp {
    fn digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
        let mut s = String::new();
        while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) { s.push(c); it.next(); }
        s
    }
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Cmp::Equal,
            (None, _) => return Cmp::Less,
            (_, None) => return Cmp::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (n, m) = (digits(&mut a), digits(&mut b));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                match n.len().cmp(&m.len()).then_with(|| n.cmp(m)) { Cmp::Equal => {}, o => return o }
            }
            (Some(x), Some(y)) => {
                match x.to_lowercase().cmp(y.to_lowercase()) { Cmp::Equal => { a.next(); b.next(); } o => return o }
            }
        }
    }
}

/// Jobs in merge order. Ties fall back to the order they were added. Pure.
pub fn ordered(jobs: &[Job], order: Order) -> Vec<&Job> {
    let mut v: Vec<&Job> = jobs.iter().collect();
    match order {
        Order::Name     => v.sort_by(|a, b| natural_cmp(&a.name, &b.name).then(a.id.cmp(&b.id))),
        Order::Modified => v.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| natural_cmp(&a.name, &b.name)).then(a.id.cmp(&b.id))),
        Order::Added    => v.sort_by_key(|j| j.id),
    }
    v
}

/// 42 s → "0:42", 723 s → "12:03", 3723 s → "1:02:03". Pure.
pub fn clock(ms: f64) -> String {
    let s = (ms / 1000.0).round().max(0.0) as u64;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
}

/// SubRip's clock: "00:01:02,345". Pure.
pub fn srt_time(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Unix days → (year, month, day), proleptic Gregorian (Howard Hinnant's
/// civil_from_days). For default file names without pulling in a date crate. Pure.
pub fn ymd(unix_days: i64) -> (i64, u32, u32) {
    let z = unix_days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The finished jobs, as one document. Headers: each file's name and length
/// above its text. SRT runs the files back to back on one clock and ignores
/// headers (subtitles have nowhere to put them). Pure.
pub fn render(jobs: &[&Job], fmt: Format, headers: bool) -> String {
    let done: Vec<&&Job> = jobs.iter().filter(|j| j.phase == Phase::Done).collect();
    match fmt {
        Format::Srt => {
            let mut out = String::new();
            let (mut n, mut at) = (0, 0.0);
            for j in done {
                let whole = [Seg { start: 0.0, end: j.audio_ms / 1000.0, text: j.text.clone() }];
                let segs: &[Seg] = if j.segs.is_empty() { &whole } else { &j.segs };
                for s in segs.iter().filter(|s| !s.text.is_empty()) {
                    n += 1;
                    out += &format!("{n}\n{} --> {}\n{}\n\n", srt_time(at + s.start), srt_time(at + s.end.max(s.start)), s.text);
                }
                at += j.audio_ms / 1000.0;
            }
            out
        }
        Format::Txt | Format::Md => {
            let md = fmt == Format::Md;
            let parts: Vec<String> = done.iter().filter_map(|j| {
                let text = if j.text.is_empty() { headers.then(|| "(nothing heard)".to_owned())? } else { j.text.clone() };
                Some(match (headers, md) {
                    (false, _)    => text,
                    (true, false) => format!("{} ({})\n{text}", j.stem(), clock(j.audio_ms)),
                    (true, true)  => format!("### {}\n*{}*\n\n{text}", j.stem(), clock(j.audio_ms)),
                })
            }).collect();
            let mut out = parts.join("\n\n");
            if !out.is_empty() { out.push('\n'); }
            out
        }
    }
}

/// `dir/stem.ext`, or `stem (2).ext`, `(3)`… if that's taken. Never overwrites.
pub fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    (1..).map(|n| dir.join(if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") }))
        .find(|p| !p.exists())
        .expect("an unbounded range finds a free name")
}

// ── the engine ───────────────────────────────────────────────────────────────

/// What one file came out as.
pub struct Heard { pub text: String, pub segs: Vec<Seg>, pub audio_ms: f64, pub asr_ms: f64 }

/// One file, start to finish: decode, cut at the quiet spots, transcribe each
/// piece, stitch text and times. `progress(done, of)` fires after decoding
/// (done = 0) and after every piece. `stop` is checked between pieces:
/// Ok(None) = stopped.
pub fn hear(model: &mut asr::Model, path: &Path, mut progress: impl FnMut(usize, usize), stop: impl Fn() -> bool) -> Result<Option<Heard>> {
    let pcm = clips::decode_file(path)?;
    let audio_ms = pcm.len() as f64 * 1000.0 / SR as f64;
    let pieces = dsp::split_at_quiet(&pcm, PIECE_S * SR as usize, 10 * SR as usize, SR as usize / 10);
    progress(0, pieces.len());
    let t0 = Instant::now();
    let (mut texts, mut segs) = (Vec::new(), Vec::new());
    for (i, r) in pieces.iter().enumerate() {
        if stop() { return Ok(None); }
        let at = r.start as f64 / SR as f64;
        let (text, s) = asr::transcribe_timed(model, pcm[r.clone()].to_vec())?;
        if !text.is_empty() { texts.push(text); }
        segs.extend(s.into_iter().map(|x| Seg { start: x.start + at, end: x.end + at, ..x }));
        progress(i + 1, pieces.len());
    }
    Ok(Some(Heard { text: texts.join(" "), segs, audio_ms, asr_ms: t0.elapsed().as_secs_f64() * 1000.0 }))
}

/// The model folder and device the Files tab wants, from the config.
fn target(cfg: &Config) -> Result<(PathBuf, Device)> {
    let dir = match cfg.files_model.as_str() {
        "" | "active" => cfg.model_dir.clone(),
        id => {
            let s = models::find(id).ok_or_else(|| anyhow!("unknown model {id:?}"))?;
            if !s.installed() { bail!("{} isn't downloaded: get it on the Models page", s.name); }
            s.dir()
        }
    };
    Ok((dir, cfg.files_device.unwrap_or(cfg.device)))
}

// ── the queue ────────────────────────────────────────────────────────────────

/// Paths dropped or picked → audio files. A folder (a WhatsApp export) is
/// opened one level down. Returns (audio, everything else).
fn expand(paths: Vec<PathBuf>) -> (Vec<PathBuf>, usize) {
    let mut found = Vec::new();
    let mut skipped = 0;
    for p in paths {
        if p.is_dir() {
            let mut inside: Vec<PathBuf> = std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).filter(|q| q.is_file()).collect();
            inside.sort();
            for q in inside { if is_audio(&q) { found.push(q) } else { skipped += 1 } }
        } else if is_audio(&p) {
            found.push(p);
        } else {
            skipped += 1;
        }
    }
    (found, skipped)
}

#[derive(Debug, Serialize)]
pub struct Added { pub added: usize, pub already: usize, pub skipped: usize }

/// Queue files and make sure a worker is on them.
pub fn add(state: &Arc<State>, paths: Vec<PathBuf>) -> Added {
    let (found, skipped) = expand(paths);
    let (mut added, mut already) = (0, 0);
    {
        let mut f = state.files.lock();
        for p in found {
            if f.jobs.iter().any(|j| j.path == p && !matches!(j.phase, Phase::Failed | Phase::Stopped)) { already += 1; continue; }
            let id = f.next;
            f.next += 1;
            f.jobs.push(Job::new(id, p));
            added += 1;
        }
    }
    if added > 0 { spawn_worker(state); }
    push(state);
    Added { added, already, skipped }
}

/// Stop: drop the file being heard and everything queued behind it.
pub fn stop(state: &State) {
    {
        let mut f = state.files.lock();
        f.jobs.iter_mut().filter(|j| j.phase == Phase::Queued).for_each(|j| j.phase = Phase::Stopped);
        if f.jobs.iter().any(Job::active) { state.files_stop.store(true, Ordering::Release); }
    }
    push(state);
}

pub fn retry(state: &Arc<State>, id: u64) {
    {
        let mut f = state.files.lock();
        if let Some(j) = f.jobs.iter_mut().find(|j| j.id == id && matches!(j.phase, Phase::Failed | Phase::Stopped | Phase::Done)) {
            *j = Job::new(j.id, j.path.clone());
        }
    }
    spawn_worker(state);
    push(state);
}

/// Forget one file. The one being heard can't go: stop it first.
pub fn remove(state: &State, id: u64) -> Result<()> {
    {
        let mut f = state.files.lock();
        if f.jobs.iter().any(|j| j.id == id && j.active()) { bail!("it's being transcribed: stop it first"); }
        f.jobs.retain(|j| j.id != id);
    }
    push(state);
    Ok(())
}

/// Forget everything that isn't mid-transcription.
pub fn clear(state: &State) {
    state.files.lock().jobs.retain(Job::active);
    push(state);
}

fn spawn_worker(state: &Arc<State>) {
    {
        let mut f = state.files.lock();
        if f.running { return; }
        f.running = true;
    }
    let st = Arc::clone(state);
    let spawned = std::thread::Builder::new().name("files".into()).spawn(move || {
        let _guard = Crashed(&st);
        worker(&st);
    });
    if spawned.is_err() {
        state.files.lock().running = false;
        warn!("files: couldn't start a worker thread");
    }
}

/// Runs only if the worker panics: mark what it was on as failed and let the
/// next add() start a fresh one. A clean exit already did this under the lock.
struct Crashed<'a>(&'a State);

impl Drop for Crashed<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() { return; }
        let mut f = self.0.files.lock();
        f.running = false;
        f.jobs.iter_mut().filter(|j| j.active()).for_each(|j| { j.phase = Phase::Failed; j.error = Some("the transcriber crashed (see the log)".into()); });
        drop(f);
        if !self.0.racing.load(Ordering::Acquire) { self.0.park.store(false, Ordering::Release); }
        push(self.0);
    }
}

/// A loaded model, and what it was loaded for.
struct Loaded { key: (PathBuf, Device, String), model: asr::Model, name: String, on: Device }

fn worker(state: &Arc<State>) {
    let mut loaded: Option<Loaded> = None;
    let mut parked = false;
    let mut idle = Instant::now();
    info!("files: worker up");

    loop {
        // The next job, or the exit: decided under the lock add() takes.
        let next = {
            let mut f = state.files.lock();
            match f.jobs.iter_mut().find(|j| j.phase == Phase::Queued) {
                Some(j) => {
                    j.phase = Phase::Decoding;
                    state.files_stop.store(false, Ordering::Release);
                    Some((j.id, j.path.clone()))
                }
                None => {
                    let on_gpu = loaded.as_ref().is_some_and(|l| l.on != Device::Cpu);
                    if !state.alive() || on_gpu || idle.elapsed() >= LINGER {
                        f.running = false;
                        break;
                    }
                    None
                }
            }
        };
        let Some((id, path)) = next else { std::thread::sleep(Duration::from_millis(200)); continue };
        push(state);

        let outcome = (|| -> Result<Option<Heard>> {
            let (cfg_dir, want, threads, lang) = {
                let c = state.cfg.read();
                let (dir, dev) = target(&c)?;
                (dir, dev, c.threads(), c.language.clone())
            };
            let key = (cfg_dir.clone(), want, lang.clone());
            if loaded.as_ref().map(|l| &l.key) != Some(&key) {
                drop(loaded.take()); // one copy at a time: two 2.5 GB models is a choice
                if want != Device::Cpu && !parked && race::dictation_on_gpu(state) {
                    race::park(state);
                    parked = true;
                }
                let t0 = Instant::now();
                let asr::Loaded { model: mut m, device, note } = asr::load_on(&cfg_dir, threads, want, false, &lang)?;
                if let Some(n) = note { warn!("files: {n}"); }
                asr::warm_up(&mut m)?;
                info!("files: {} up on {} in {:.0?}", asr::model_name(&cfg_dir), device.label(), t0.elapsed());
                loaded = Some(Loaded { key, model: m, name: asr::model_name(&cfg_dir), on: device });
            }
            let l = loaded.as_mut().expect("loaded above");
            set(state, id, |j| { j.model = l.name.clone(); j.device = l.on.label().into(); });
            hear(&mut l.model, &path,
                |done, of| { set(state, id, |j| { j.phase = Phase::Running; j.done = done; j.of = of; }); push(state); },
                || state.files_stop.load(Ordering::Acquire) || !state.alive())
        })();

        set(state, id, |j| match outcome {
            Ok(Some(ref h)) => {
                info!("files: {} → {} chars, {} audio in {:.0} ms", j.name, h.text.chars().count(), clock(h.audio_ms), h.asr_ms);
                j.phase = Phase::Done;
                j.text = h.text.clone();
                j.segs = h.segs.clone();
                j.audio_ms = h.audio_ms;
                j.asr_ms = h.asr_ms;
                j.done = j.of;
            }
            Ok(None) => j.phase = Phase::Stopped,
            Err(ref e) => {
                warn!("files: {}: {e:#}", j.name);
                j.phase = Phase::Failed;
                j.error = Some(asr::explain(Device::Cpu, &format!("{e:#}")));
            }
        });
        idle = Instant::now();
        push(state);
    }

    drop(loaded);
    if parked && !state.racing.load(Ordering::Acquire) { state.park.store(false, Ordering::Release); }
    info!("files: worker down");
    push(state);
}

fn set(state: &State, id: u64, f: impl FnOnce(&mut Job)) {
    if let Some(j) = state.files.lock().jobs.iter_mut().find(|j| j.id == id) { f(j) }
}

// ── what the page and the clipboard get ──────────────────────────────────────

/// Everything the Files tab shows, in merge order, plus the merged document
/// exactly as Copy / Save would write it.
pub fn view(state: &State) -> Value {
    let (order, fmt, headers) = { let c = state.cfg.read(); (c.files_order, c.files_format, c.files_headers) };
    let f = state.files.lock();
    let list = ordered(&f.jobs, order);
    json!({
        "jobs":     list,
        "running":  f.running && f.jobs.iter().any(|j| j.active() || j.phase == Phase::Queued),
        "preview":  render(&list, fmt, headers),
        "audio_ms": list.iter().filter(|j| j.phase == Phase::Done).map(|j| j.audio_ms).sum::<f64>(),
        "exts":     AUDIO_EXTS,
    })
}

pub fn push(state: &State) { ui::push("files", &view(state)); }

/// The finished jobs in merge order, cloned out of the lock.
fn finished(state: &State) -> Vec<Job> {
    let order = state.cfg.read().files_order;
    let f = state.files.lock();
    ordered(&f.jobs, order).into_iter().filter(|j| j.phase == Phase::Done).cloned().collect()
}

/// One file's text, or everything finished as one document.
pub fn copy(state: &State, id: Option<u64>) -> Result<usize> {
    let text = match id {
        Some(id) => state.files.lock().jobs.iter().find(|j| j.id == id).map(|j| j.text.clone()).ok_or_else(|| anyhow!("gone"))?,
        None => {
            let (fmt, headers) = { let c = state.cfg.read(); (c.files_format, c.files_headers) };
            render(&finished(state).iter().collect::<Vec<_>>(), fmt, headers)
        }
    };
    if text.trim().is_empty() { bail!("nothing transcribed yet"); }
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone())).context("clipboard")?;
    Ok(text.chars().count())
}

/// Save… : one document (a save dialog) or one file per audio (a folder
/// dialog). Off the UI thread: a modal dialog must never freeze the window.
pub fn save(state: &Arc<State>) {
    let st = Arc::clone(state);
    std::thread::spawn(move || {
        let jobs = finished(&st);
        let toast = |kind: &str, msg: String| ui::push("toast", &json!({ "kind": kind, "msg": msg }));
        let Some(first) = jobs.first() else { return toast("info", "nothing transcribed yet".into()) };
        let (merge, fmt, headers) = { let c = st.cfg.read(); (c.files_merge, c.files_format, c.files_headers) };
        let start = first.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let filter = (fmt.ext().to_uppercase(), [fmt.ext()]);
        let result: Result<Option<String>> = (|| {
            if merge || jobs.len() == 1 {
                let name = if jobs.len() == 1 { format!("{}.{}", first.stem(), fmt.ext()) } else { format!("transcripts {}.{}", today(), fmt.ext()) };
                let Some(p) = rfd::FileDialog::new().set_directory(&start).set_file_name(name).add_filter(&filter.0, &filter.1).save_file() else { return Ok(None) };
                std::fs::write(&p, render(&jobs.iter().collect::<Vec<_>>(), fmt, headers)).with_context(|| format!("write {}", p.display()))?;
                Ok(Some(format!("saved {}", p.display())))
            } else {
                let Some(dir) = rfd::FileDialog::new().set_directory(&start).pick_folder() else { return Ok(None) };
                for j in &jobs {
                    let p = unique(&dir, j.stem(), fmt.ext());
                    std::fs::File::create(&p).and_then(|mut f| f.write_all(render(&[j], fmt, headers).as_bytes()))
                        .with_context(|| format!("write {}", p.display()))?;
                }
                Ok(Some(format!("saved {} files in {}", jobs.len(), dir.display())))
            }
        })();
        match result {
            Ok(Some(msg)) => toast("ok", msg),
            Ok(None) => {}
            Err(e) => toast("error", format!("{e:#}")),
        }
    });
}

/// Browse… : the native open dialog, many files at once.
pub fn browse(state: &Arc<State>) {
    let st = Arc::clone(state);
    std::thread::spawn(move || {
        if let Some(paths) = rfd::FileDialog::new().add_filter("Audio", AUDIO_EXTS).pick_files() {
            let r = add(&st, paths);
            ui::push("files_added", &r);
        }
    });
}

/// Show the original in the file manager, selected where the OS can.
pub fn reveal(state: &State, id: u64) -> Result<()> {
    let path = state.files.lock().jobs.iter().find(|j| j.id == id).map(|j| j.path.clone()).ok_or_else(|| anyhow!("gone"))?;
    // explorer parses its own command line: /select,"C:\a b\x.ogg" or nothing.
    #[cfg(target_os = "windows")]
    let r = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn()
    };
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(&path)).spawn();
    r.map(drop).context("opening the file manager")
}

/// The original audio, for the page's ▶ (served at dv://…/file/<id>).
pub fn audio(state: &State, id: u64) -> Option<(Vec<u8>, &'static str)> {
    let path = state.files.lock().jobs.iter().find(|j| j.id == id).map(|j| j.path.clone())?;
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "ogg" | "opus" | "oga" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "webm" => "audio/webm",
        _ => "audio/x-matroska",
    };
    std::fs::read(path).ok().map(|b| (b, mime))
}

fn today() -> String {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (y, m, d) = ymd(days);
    format!("{y}-{m:02}-{d:02}")
}

// ── `divinevoice transcribe` ─────────────────────────────────────────────────

const USAGE: &str = "usage: divinevoice transcribe <file|folder>... [options]\n\
                     \x20  --model <id|dir>      default: the Files tab's model (else the active one)\n\
                     \x20  --device cpu|cuda|directml\n\
                     \x20  --lang auto|en|pt|…\n\
                     \x20  --format txt|md|srt   default txt\n\
                     \x20  --order name|modified|given   default name\n\
                     \x20  --no-headers          just the text\n\
                     \x20  --out <path>          a file: everything merged into it · a folder: one file per audio\n\
                     \x20                        default: merged, to stdout";

/// The Files tab without the window. Progress on stderr, text on stdout (or --out).
pub fn cli(args: &[String], cfg: &Config) -> Result<()> {
    let mut cfg = cfg.clone();
    let (mut paths, mut out, mut given) = (Vec::new(), None::<PathBuf>, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |flag: &str| it.next().cloned().ok_or_else(|| anyhow!("{flag} needs a value\n{USAGE}"));
        match a.as_str() {
            "--model" | "-m" => {
                let v = val("--model")?;
                if models::find(&v).is_some() { cfg.files_model = v } else { cfg.files_model = "active".into(); cfg.model_dir = PathBuf::from(v) }
            }
            "--device" | "-d" => {
                let v = val("--device")?;
                cfg.files_device = Some(Device::ALL.into_iter().find(|d| d.label() == v.to_ascii_lowercase()).ok_or_else(|| anyhow!("--device wants cpu, cuda or directml"))?);
            }
            "--lang" | "-l" => cfg.language = val("--lang")?.to_lowercase(),
            "--format" | "-f" => cfg.files_format = serde_json::from_value(json!(val("--format")?.to_lowercase())).map_err(|_| anyhow!("--format wants txt, md or srt"))?,
            "--order" => match val("--order")?.as_str() {
                "given" => given = true,
                o => cfg.files_order = serde_json::from_value(json!(o)).map_err(|_| anyhow!("--order wants name, modified or given"))?,
            },
            "--no-headers" => cfg.files_headers = false,
            "--out" | "-o" => out = Some(PathBuf::from(val("--out")?)),
            "-h" | "--help" => { println!("{USAGE}"); return Ok(()) }
            f if f.starts_with('-') => bail!("unknown flag {f}\n{USAGE}"),
            f => paths.push(PathBuf::from(f)),
        }
    }
    let (found, skipped) = expand(paths);
    if found.is_empty() { bail!("no audio files given ({skipped} skipped)\n{USAGE}"); }
    if skipped > 0 { eprintln!("skipping {skipped} file(s) that aren't audio"); }

    let mut jobs: Vec<Job> = found.into_iter().enumerate().map(|(i, p)| Job::new(i as u64, p)).collect();
    if !given { jobs = ordered(&jobs, cfg.files_order).into_iter().cloned().collect(); }
    let (dir, want) = target(&cfg)?;
    let t0 = Instant::now();
    let asr::Loaded { model: mut m, device, note } = asr::load_on(&dir, cfg.threads(), want, false, &cfg.language)?;
    if let Some(n) = note { eprintln!("note: {n}"); }
    asr::warm_up(&mut m)?;
    eprintln!("{} on {} · loaded in {:.1?}", asr::model_name(&dir), device.label(), t0.elapsed());

    let n = jobs.len();
    for (i, j) in jobs.iter_mut().enumerate() {
        let tag = format!("[{}/{n}] {}", i + 1, j.name);
        let heard = hear(&mut m, &j.path, |d, of| { eprint!("\r{tag}  {d}/{of}   "); let _ = std::io::stderr().flush(); }, || false);
        match heard {
            Ok(Some(h)) => {
                eprintln!("\r{tag}  {} → {} words in {:.1} s ({:.0}× real-time)", clock(h.audio_ms), h.text.split_whitespace().count(),
                    h.asr_ms / 1000.0, h.audio_ms / h.asr_ms.max(1.0));
                (j.text, j.segs, j.audio_ms, j.asr_ms, j.phase) = (h.text, h.segs, h.audio_ms, h.asr_ms, Phase::Done);
            }
            Ok(None) => unreachable!("the CLI never stops"),
            Err(e) => { eprintln!("\r{tag}  FAILED: {e:#}"); j.phase = Phase::Failed; }
        }
    }

    let (fmt, headers) = (cfg.files_format, cfg.files_headers && n > 1);
    match out {
        Some(d) if d.is_dir() => {
            for j in jobs.iter().filter(|j| j.phase == Phase::Done) {
                let p = unique(&d, j.stem(), fmt.ext());
                std::fs::write(&p, render(&[j], fmt, false)).with_context(|| format!("write {}", p.display()))?;
                eprintln!("wrote {}", p.display());
            }
        }
        Some(p) => {
            std::fs::write(&p, render(&jobs.iter().collect::<Vec<_>>(), fmt, headers)).with_context(|| format!("write {}", p.display()))?;
            eprintln!("wrote {}", p.display());
        }
        None => print!("{}", render(&jobs.iter().collect::<Vec<_>>(), fmt, headers)),
    }
    let failed = jobs.iter().filter(|j| j.phase == Phase::Failed).count();
    if failed > 0 { bail!("{failed} of {n} file(s) failed"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, name: &str, ms: f64, text: &str, segs: Vec<Seg>) -> Job {
        Job { phase: Phase::Done, audio_ms: ms, text: text.into(), segs, ..Job::new(id, PathBuf::from(format!("/voice/{name}"))) }
    }
    fn seg(start: f64, end: f64, text: &str) -> Seg { Seg { start, end, text: text.into() } }

    #[test]
    fn natural_order_reads_numbers_as_numbers() {
        let mut v = vec!["voice 10.ogg", "Voice 9.ogg", "voice 9b.ogg", "PTT-20261103-WA0002.opus", "PTT-20261103-WA0010.opus"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["PTT-20261103-WA0002.opus", "PTT-20261103-WA0010.opus", "Voice 9.ogg", "voice 9b.ogg", "voice 10.ogg"]);
        assert_eq!(natural_cmp("a007", "a7"), Cmp::Equal);
        assert_eq!(natural_cmp("WhatsApp Audio 2026-11-03 at 9.05.12", "WhatsApp Audio 2026-11-03 at 14.22.31"), Cmp::Less);
    }

    #[test]
    fn ordering_modes() {
        let mut a = job(0, "b.ogg", 1000.0, "", vec![]); a.modified = Some(20);
        let mut b = job(1, "a.ogg", 1000.0, "", vec![]); b.modified = Some(30);
        let mut c = job(2, "c.ogg", 1000.0, "", vec![]); c.modified = Some(10);
        let jobs = [a, b, c];
        let ids = |o| ordered(&jobs, o).iter().map(|j| j.id).collect::<Vec<_>>();
        assert_eq!(ids(Order::Name), [1, 0, 2]);
        assert_eq!(ids(Order::Modified), [2, 0, 1]);
        assert_eq!(ids(Order::Added), [0, 1, 2]);
    }

    #[test]
    fn clocks() {
        assert_eq!(clock(42_000.0), "0:42");
        assert_eq!(clock(723_400.0), "12:03");
        assert_eq!(clock(3_723_000.0), "1:02:03");
        assert_eq!(srt_time(62.345), "00:01:02,345");
        assert_eq!(srt_time(3725.0), "01:02:05,000");
    }

    #[test]
    fn calendar() {
        assert_eq!(ymd(0), (1970, 1, 1));
        assert_eq!(ymd(20_760), (2026, 11, 3));
        assert_eq!(ymd(11_016), (2000, 2, 29)); // leap day, century rule
        assert_eq!(ymd(-1), (1969, 12, 31));
    }

    #[test]
    fn txt_and_md_with_and_without_headers() {
        let a = job(0, "PTT-1.opus", 42_000.0, "Hello there.", vec![]);
        let b = job(1, "PTT-2.opus", 73_000.0, "On my way.", vec![]);
        let mut q = job(2, "PTT-3.opus", 0.0, "", vec![]); q.phase = Phase::Queued; // not done: never rendered
        let all = [&a, &b, &q];
        assert_eq!(render(&all, Format::Txt, true), "PTT-1 (0:42)\nHello there.\n\nPTT-2 (1:13)\nOn my way.\n");
        assert_eq!(render(&all, Format::Txt, false), "Hello there.\n\nOn my way.\n");
        assert_eq!(render(&all, Format::Md, true), "### PTT-1\n*0:42*\n\nHello there.\n\n### PTT-2\n*1:13*\n\nOn my way.\n");
        assert_eq!(render(&[&q], Format::Txt, true), "");
    }

    #[test]
    fn silence_shows_only_with_headers() {
        let e = job(0, "quiet.ogg", 3000.0, "", vec![]);
        assert_eq!(render(&[&e], Format::Txt, true), "quiet (0:03)\n(nothing heard)\n");
        assert_eq!(render(&[&e], Format::Txt, false), "");
    }

    #[test]
    fn srt_runs_files_back_to_back_on_one_clock() {
        let a = job(0, "a.ogg", 10_000.0, "One. Two.", vec![seg(0.5, 2.0, "One."), seg(3.0, 4.25, "Two.")]);
        let b = job(1, "b.ogg", 5_000.0, "Three", vec![]); // no sentence times (Nemotron): one cue
        assert_eq!(render(&[&a, &b], Format::Srt, true),
            "1\n00:00:00,500 --> 00:00:02,000\nOne.\n\n\
             2\n00:00:03,000 --> 00:00:04,250\nTwo.\n\n\
             3\n00:00:10,000 --> 00:00:15,000\nThree\n\n");
    }

    #[test]
    fn audio_by_extension() {
        for p in ["a.ogg", "b.OPUS", "c.m4a", "d.mp3", "e.webm", "f.wav", "g.flac"] { assert!(is_audio(Path::new(p)), "{p}"); }
        for p in ["a.txt", "b.jpg", "noext", "c.ogg.part"] { assert!(!is_audio(Path::new(p)), "{p}"); }
    }

    #[test]
    fn unique_never_overwrites() {
        let d = std::env::temp_dir().join(format!("dv-unique-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(unique(&d, "note", "txt"), d.join("note.txt"));
        std::fs::write(d.join("note.txt"), "x").unwrap();
        std::fs::write(d.join("note (2).txt"), "x").unwrap();
        assert_eq!(unique(&d, "note", "txt"), d.join("note (3).txt"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn folders_open_one_level_and_skip_the_rest() {
        let d = std::env::temp_dir().join(format!("dv-expand-{}", std::process::id()));
        std::fs::create_dir_all(d.join("deeper")).unwrap();
        for f in ["b.opus", "a.ogg", "chat.txt", "deeper/x.ogg"] { std::fs::write(d.join(f), "").unwrap(); }
        let (found, skipped) = expand(vec![d.clone(), d.join("missing.mp3"), d.join("chat.txt")]);
        assert_eq!(found, vec![d.join("a.ogg"), d.join("b.opus"), d.join("missing.mp3")]);
        assert_eq!(skipped, 2); // chat.txt inside, chat.txt dropped; deeper/ not opened
        let _ = std::fs::remove_dir_all(&d);
    }
}
