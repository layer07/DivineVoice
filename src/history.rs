// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * history.rs
 *
 * Version: @(#)history.rs 0.3.1 2026
 *
 * Description: The Memory Stone.
 *              Last N things you said, only if you asked. Off by default:
 *              plaintext log of every word is a bad default. Small file, rewrite
 *              whole thing every change. fsync cheaper than cleverness.
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

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Unix milliseconds. Doubles as the id; you can't talk twice in one ms.
    pub id:        u64,
    pub text:      String,
    pub audio_ms:  u64,
    pub asr_ms:    u64,
}

pub struct History {
    path:  PathBuf,
    items: Mutex<VecDeque<Entry>>, // newest first
}

impl History {
    pub fn load() -> Self {
        let path = crate::config::home().join("history.json");
        let items = std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, items: Mutex::new(items) }
    }

    pub fn list(&self) -> Vec<Entry> { self.items.lock().iter().cloned().collect() }

    pub fn push(&self, text: String, audio_ms: u64, asr_ms: u64, max: usize) -> Entry {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let e = Entry { id, text, audio_ms, asr_ms };
        self.mutate(|v| { v.push_front(e.clone()); v.truncate(max.max(1)); });
        e
    }

    pub fn remove(&self, id: u64) { self.mutate(|v| v.retain(|e| e.id != id)) }
    pub fn clear(&self)           { self.mutate(VecDeque::clear) }
    pub fn trim(&self, max: usize) { self.mutate(|v| v.truncate(max.max(1))) }

    pub fn get(&self, id: u64) -> Option<Entry> { self.items.lock().iter().find(|e| e.id == id).cloned() }

    fn mutate(&self, f: impl FnOnce(&mut VecDeque<Entry>)) {
        let mut v = self.items.lock();
        f(&mut v);
        let json = serde_json::to_string(&*v).unwrap_or_else(|_| "[]".into());
        if let Err(e) = std::fs::write(&self.path, json) { warn!("history: write {}: {e}", self.path.display()); }
    }
}
