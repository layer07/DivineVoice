// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * ipc.rs
 *
 * Version: @(#)ipc.rs 0.3.0 2026
 *
 * Description: The Settings Switchboard.
 *              Page send JSON, Rust answer JSON. Config write is one trick:
 *              serialise, poke one key, deserialise. serde do type check for
 *              free, Binding veto the footgun. Error become red toast, never
 *              panic on main thread.
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

// Messages are JSON: {"cmd": "...", ...args}. Each handler returns zero or more
// pre-serialised events to send back. Errors become a red toast in the UI,
// never a panic on the main thread — the main thread owns the tray, and a dead
// tray means a zombie process nobody can quit.
//
// Settings are written with one generic trick: serialise Config to JSON, poke
// the one key, deserialise it back. serde does all the type checking (and
// Binding::sane() vetoes footgun hotkeys), so there's no per-field setter
// boilerplate to rot.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::{Config, HotkeyMode};
use crate::hotkey::{self, Learned};
use crate::models::{self, CATALOGUE};
use crate::state::{Download, ModelStatus, State};
use crate::{audio, config, perf, ui};

pub fn envelope<T: Serialize + ?Sized>(ev: &str, data: &T) -> String {
    json!({ "ev": ev, "data": data }).to_string()
}

fn toast(kind: &str, msg: impl Into<String>) -> String {
    envelope("toast", &json!({ "kind": kind, "msg": msg.into() }))
}

/// The whole world, as the page sees it. `full` adds the slow bits (mic
/// enumeration hits the audio stack), so routine refreshes skip them.
pub fn snapshot(state: &State, full: bool) -> Value {
    let cfg = state.cfg.read().clone();
    // Active = on disk and loadable. A fresh install's config points at the
    // default (Ultra) before it's downloaded: that's a wish, not an active
    // model, and showing it as ACTIVE hid its Download button.
    let picked = models::which(&cfg.model_dir);
    let active = picked.filter(|s| s.installed()).map(|s| s.id);
    let custom = picked.is_none() && config::looks_like_model(&cfg.model_dir);
    let catalogue: Vec<Value> = CATALOGUE.iter().map(|s| {
        let mut v = serde_json::to_value(s).unwrap_or_default();
        v["size"] = s.size().into();
        v["installed"] = s.installed().into();
        v["on_disk"] = s.on_disk().into();
        v
    }).collect();

    let mut v = json!({
        "version":      env!("CARGO_PKG_VERSION"),
        "config":       cfg,
        "status":       { "mode": state.mode(), "bus": state.bus_connected.load(Ordering::Acquire) },
        "model":        *state.model.read(),
        "active_model": active,
        "custom_model": custom,
        "models":       catalogue,
        "download":     *state.download.lock(),
        "interception": state.interception.load(Ordering::Acquire),
        "autostart":    crate::autostart::enabled(),
        "history":      state.history.list(),
        "clips":        crate::clips::list(),
        "unsupported":  crate::asr::UNSUPPORTED.iter()
                            .map(|(e, d, why)| json!({ "engine": e, "device": d, "why": why }))
                            .collect::<Vec<_>>(),
        // null until the ASR thread has probed them; never probe on the UI thread
        "devices":      crate::asr::devices_known().map(|ds| ds.iter()
                            .map(|(d, r)| json!({ "device": d, "ok": r.is_ok(), "reason": r.as_ref().err() }))
                            .collect::<Vec<_>>()),
        "perf":         perf_view(state),
        "paths": {
            "config": Config::path(),
            "home":   config::home(),
            "models": models::root(),
        },
    });
    if full { v["mics"] = audio::list_inputs().into(); }
    v
}

pub fn perf_view(state: &State) -> Value {
    let entries = state.perf.list();
    json!({ "summary": perf::summarize(&entries), "entries": entries })
}

pub fn handle(state: &Arc<State>, raw: &str) -> Vec<String> {
    let msg: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => return vec![toast("error", format!("bad IPC message: {e}"))],
    };
    let cmd = msg["cmd"].as_str().unwrap_or("");
    tracing::debug!("ipc ← {cmd}");
    dispatch(state, cmd, &msg).unwrap_or_else(|e| {
        warn!("ipc {cmd}: {e:#}");
        vec![toast("error", format!("{e:#}"))]
    })
}

fn dispatch(state: &Arc<State>, cmd: &str, m: &Value) -> Result<Vec<String>> {
    let str_arg = |k: &str| m[k].as_str().ok_or_else(|| anyhow!("{cmd}: missing \"{k}\""));
    let refreshed = |extra: Vec<String>| -> Vec<String> {
        std::iter::once(envelope("state", &snapshot(state, false))).chain(extra).collect()
    };

    Ok(match cmd {
        "hello" => vec![envelope("hello", &snapshot(state, true))],
        "jserror" => { warn!("ui js: {}", m["msg"].as_str().unwrap_or("?")); vec![] }
        "mics"  => vec![envelope("mics", &audio::list_inputs())],

        "set" => {
            let key = str_arg("key")?;
            set(state, key, m["value"].clone())?;
            refreshed(vec![toast("ok", "saved")])
        }

        // Benchmark tab. Arm = next clip is an exam against `reference`.
        "bench_arm" => {
            let reference = str_arg("reference")?.to_owned();
            let mut b = state.bench.lock();
            *b = crate::state::Bench { armed: true, reference, take: None };
            vec![]
        }
        "bench_disarm" => { state.disarm_bench(); vec![] }
        // Race a benchmark clip ({clip: id}) or, without one, your last read-along take.
        //   models:  "active" (default) | "all" | ["parakeet-ultra", ...]
        //   devices: ["cpu", "directml", ...]  (default: the configured device)
        "race" => {
            let (take, reference) = match m["clip"].as_str() {
                Some(id) => {
                    let (pcm, c) = crate::clips::pcm(id)?;
                    (Arc::new(pcm), c.script)
                }
                None => crate::race::last_take(state)?,
            };
            let scope = match &m["models"] {
                Value::String(s) if s == "all" => crate::race::Scope::All,
                Value::Array(ids) => crate::race::Scope::Only(ids.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()),
                _ => crate::race::Scope::Active,
            };
            let devices: Vec<crate::config::Device> = serde_json::from_value(m["devices"].clone()).unwrap_or_default();
            crate::race::start(Arc::clone(state), take, reference, scope, devices)?;
            vec![]
        }
        "race_cancel" => { state.race_cancel.store(true, Ordering::Release); vec![] }

        "autostart" => {
            let on = m["on"].as_bool().ok_or_else(|| anyhow!("autostart: on"))?;
            crate::autostart::set(on)?;
            refreshed(vec![toast("ok", if on { "starts with Windows" } else { "won't start with Windows" })])
        }

        "learn" => { hotkey::learn(); vec![] }
        "learn_cancel" => { hotkey::cancel_learn(); vec![] }

        "model_use" => {
            let spec = models::find(str_arg("id")?).ok_or_else(|| anyhow!("unknown model"))?;
            if !spec.installed() { bail!("{} isn't downloaded yet", spec.name); }
            set(state, "model_dir", json!(spec.dir()))?;
            retry_if_down(state);
            refreshed(vec![toast("ok", format!("switching to {}…", spec.name))])
        }
        "model_dir" => {
            let dir = PathBuf::from(str_arg("path")?.trim().trim_matches('"'));
            if !config::looks_like_model(&dir) {
                bail!("no Parakeet model in {} (need encoder*.onnx + decoder_joint*.onnx + vocab.txt or tokenizer.model)", dir.display());
            }
            set(state, "model_dir", json!(dir))?;
            retry_if_down(state);
            refreshed(vec![toast("ok", "custom model folder set")])
        }
        "model_download" => {
            start_download(state, str_arg("id")?)?;
            refreshed(vec![])
        }
        "model_cancel" => {
            state.dl_cancel.store(true, Ordering::Release);
            vec![toast("info", "cancelling… partial file kept for resume")]
        }
        "model_delete" => {
            let spec = models::find(str_arg("id")?).ok_or_else(|| anyhow!("unknown model"))?;
            if models::which(&state.cfg.read().model_dir).is_some_and(|a| a.id == spec.id) {
                bail!("that's the active model — switch to another one first");
            }
            if state.download.lock().as_ref().is_some_and(|d| d.id == spec.id && d.error.is_none()) {
                bail!("it's still downloading — cancel first");
            }
            spec.delete()?;
            *state.download.lock() = None;
            refreshed(vec![toast("ok", format!("deleted {}", spec.name))])
        }

        "history_delete" => {
            state.history.remove(m["id"].as_u64().ok_or_else(|| anyhow!("history_delete: id"))?);
            refreshed(vec![])
        }
        "history_clear" => { state.history.clear(); refreshed(vec![toast("ok", "history wiped")]) }
        "perf_clear"    => { state.perf.clear(); refreshed(vec![toast("ok", "benchmarks wiped — fresh baseline")]) }
        "copy" => {
            let text = match m["id"].as_u64() {
                Some(id) => state.history.get(id).map(|e| e.text).ok_or_else(|| anyhow!("gone"))?,
                None => str_arg("text")?.to_owned(),
            };
            arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).context("clipboard")?;
            vec![toast("ok", "copied")]
        }

        "open" => {
            let p = match str_arg("what")? {
                "config" => Config::path(),
                "logs" | "home" => config::home(),
                "models" => { let _ = std::fs::create_dir_all(models::root()); models::root() }
                "model_dir" => state.cfg.read().model_dir.clone(),
                other => bail!("open what? {other}"),
            };
            open_path(&p);
            vec![]
        }
        "url" => {
            let u = str_arg("url")?;
            if !u.starts_with("https://") { bail!("only https links"); }
            let _ = std::process::Command::new("explorer.exe").arg(u).spawn();
            vec![]
        }

        other => bail!("unknown command {other:?}"),
    })
}

/// Generic config write: JSON round-trip for type checking, clamp to sane
/// ranges, apply side effects, persist.
fn set(state: &State, key: &str, value: Value) -> Result<()> {
    // Read-modify-write of the whole config. The UI thread and the download
    // thread both call this; without the lock the slower one quietly undoes
    // the faster one.
    static SET: parking_lot::Mutex<()> = parking_lot::const_mutex(());
    let _one_at_a_time = SET.lock();
    let old = state.cfg.read().clone();
    let mut v = serde_json::to_value(&old)?;
    if v.get(key).is_none() { bail!("no such setting {key:?}"); }
    v[key] = value;
    let new: Config = serde_json::from_value(v).map_err(|e| anyhow!("{key}: {e}"))?;
    let new = clamp(new);

    if new.hotkey != old.hotkey || new.hotkey_mode != old.hotkey_mode {
        hotkey::set_binding(new.hotkey, new.hotkey_mode == HotkeyMode::Toggle);
        info!("hotkey → {} ({:?})", new.hotkey, new.hotkey_mode);
    }
    let reload = new.model_dir != old.model_dir || new.intra_threads != old.intra_threads
        || new.device != old.device || new.language != old.language;
    if new.history_max < old.history_max { state.history.trim(new.history_max); }
    if new.perf_max < old.perf_max { state.perf.trim(new.perf_max); }

    new.save().context("saving config.toml")?;
    *state.cfg.write() = new;
    if reload { state.reload_model(); }
    Ok(())
}

/// Picking the model that's already configured changes nothing, so `set`
/// doesn't reload. If that model is Failed or Missing, picking it again means
/// "try again": reload anyway. Ready/Loading/Parked are left alone.
fn retry_if_down(state: &State) {
    if matches!(&*state.model.read(), ModelStatus::Failed { .. } | ModelStatus::Missing { .. }) {
        state.reload_model();
    }
}

fn clamp(mut c: Config) -> Config {
    c.min_ms          = c.min_ms.clamp(0, 5_000);
    c.max_ms          = c.max_ms.clamp(1_000, 600_000).max(c.min_ms + 100);
    c.paste_settle_ms = c.paste_settle_ms.clamp(0, 2_000);
    c.type_delay_ms   = c.type_delay_ms.clamp(0, 100);
    c.intra_threads   = c.intra_threads.min(64);
    c.history_max     = c.history_max.clamp(1, 10_000);
    c.perf_max        = c.perf_max.clamp(8, 4_096);
    c.input_device    = c.input_device.filter(|s| !s.trim().is_empty());
    c.language        = Some(c.language.trim().to_lowercase()).filter(|l| !l.is_empty()).unwrap_or_else(|| "auto".into());
    c.fbdirector      = c.fbdirector.filter(|s| !s.trim().is_empty());
    c.skip_hwids      = c.skip_hwids.into_iter().map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
    c
}

fn start_download(state: &Arc<State>, id: &str) -> Result<()> {
    let spec = models::find(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
    {
        let mut dl = state.download.lock();
        if dl.as_ref().is_some_and(|d| d.error.is_none()) { bail!("one download at a time, greedy"); }
        *dl = Some(Download { id: spec.id.into(), done: spec.on_disk(), total: spec.size(), error: None });
    }
    state.dl_cancel.store(false, Ordering::Release);

    let st = Arc::clone(state);
    std::thread::Builder::new().name("download".into()).spawn(move || {
        let progress = |done, total| {
            let snap = {
                let mut dl = st.download.lock();
                if let Some(d) = dl.as_mut() { d.done = done; d.total = total; }
                dl.clone()
            };
            ui::push("download", &snap);
        };
        match models::download(spec, &st.dl_cancel, progress) {
            Ok(()) => {
                info!("models: {} installed", spec.id);
                *st.download.lock() = None;
                // Config already points at this one (fresh install, default
                // model): the path didn't change, so nothing would tell the
                // ASR thread it exists now. Poke it, or it sits at "no model"
                // until you restart.
                let active = st.cfg.read().model_dir.clone();
                if models::which(&active).is_some_and(|a| a.id == spec.id) {
                    st.reload_model();
                } else if !config::looks_like_model(&active) {
                    // First model on a fresh install? Use it — nobody downloads
                    // 650 MB to admire it.
                    let _ = set(&st, "model_dir", json!(spec.dir()));
                }
                ui::push("toast", &json!({ "kind": "ok", "msg": format!("{} ready", spec.name) }));
            }
            Err(e) => {
                warn!("models: {} failed: {e:#}", spec.id);
                if let Some(d) = st.download.lock().as_mut() { d.error = Some(format!("{e:#}")); }
                ui::push("toast", &json!({ "kind": "error", "msg": format!("{}: {e:#}", spec.name) }));
            }
        }
        ui::push_state();
    }).context("spawning download thread")?;
    Ok(())
}

/// Hotkey learn results arrive on the hook thread; ship them to the page.
pub fn on_learned(l: Learned) {
    let payload = match l {
        Learned::Cancelled => json!({ "cancelled": true }),
        Learned::Got(b) => match b.sane() {
            Ok(())   => json!({ "binding": b.to_string() }),
            Err(why) => json!({ "binding": b.to_string(), "error": why }),
        },
    };
    ui::push("learned", &payload);
}

pub fn open_path(p: &Path) {
    let prog = if p.is_file() { "notepad.exe" } else { "explorer.exe" };
    let _ = std::process::Command::new(prog).arg(p).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the real dispatcher end to end against a throwaway %APPDATA%.
    /// One test, sequential, because APPDATA is process-global.
    #[test]
    fn settings_roundtrip_through_ipc() {
        let tmp = std::env::temp_dir().join(format!("dv-ipc-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("APPDATA", &tmp);
        let st = Arc::new(State::new(Config::default()));
        let call = |j: Value| handle(&st, &j.to_string());
        let is_err = |out: &[String]| out.iter().any(|s| s.contains(r#""kind":"error""#));

        // good hotkey → applied + persisted
        assert!(!is_err(&call(json!({"cmd": "set", "key": "hotkey", "value": "Ctrl+Shift+F13"}))));
        assert_eq!(st.cfg.read().hotkey.to_string(), "Ctrl+Shift+F13");
        let on_disk = std::fs::read_to_string(Config::path()).unwrap();
        assert!(on_disk.contains("Ctrl+Shift+F13"), "{on_disk}");

        // key-bricking hotkey → refused, old one kept
        assert!(is_err(&call(json!({"cmd": "set", "key": "hotkey", "value": "A"}))));
        assert_eq!(st.cfg.read().hotkey.to_string(), "Ctrl+Shift+F13");

        // wrong type / unknown key / unknown command → refused, no panic
        assert!(is_err(&call(json!({"cmd": "set", "key": "min_ms", "value": "lots"}))));
        assert!(is_err(&call(json!({"cmd": "set", "key": "turbo", "value": true}))));
        assert!(is_err(&call(json!({"cmd": "self_destruct"}))));
        assert!(is_err(&handle(&st, "not json")));

        // clamping
        call(json!({"cmd": "set", "key": "max_ms", "value": 50}));
        assert!(st.cfg.read().max_ms >= 1_000);

        // optional strings: "" means unset
        call(json!({"cmd": "set", "key": "fbdirector", "value": ""}));
        assert!(st.cfg.read().fbdirector.is_none());

        // model_dir change bumps the generation so asr reloads
        let gen = st.model_gen.load(Ordering::Acquire);
        call(json!({"cmd": "set", "key": "intra_threads", "value": 3}));
        assert_eq!(st.model_gen.load(Ordering::Acquire), gen + 1);

        // snapshot is serialisable and carries the catalogue
        let snap = snapshot(&st, false);
        assert_eq!(snap["models"].as_array().unwrap().len(), CATALOGUE.len());
        assert_eq!(snap["config"]["hotkey"], "Ctrl+Shift+F13");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
