use anyhow::Result;
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::warn;

use crate::binding::Binding;
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Learned {
    Got(Binding),
    Cancelled,
}

static LEARNING: AtomicBool = AtomicBool::new(false);
static LEARN_CB: OnceLock<Box<dyn Fn(Learned) + Send + Sync>> = OnceLock::new();

pub fn set_binding(_binding: Binding, _toggle: bool) {}

pub fn on_learned(f: impl Fn(Learned) + Send + Sync + 'static) {
    let _ = LEARN_CB.set(Box::new(f));
}

pub fn learn() { LEARNING.store(true, Ordering::Release) }

pub fn cancel_learn() {
    if LEARNING.swap(false, Ordering::AcqRel) {
        if let Some(callback) = LEARN_CB.get() { callback(Learned::Cancelled); }
    }
}

/// Placeholder for the Core Graphics event-tap adapter. Keeping this worker
/// alive means the rest of the app can start and report its macOS limitation.
pub fn run(state: &State, _tx: Sender<Cmd>) -> Result<()> {
    warn!("global hotkeys are not implemented on macOS yet");
    while state.alive() { std::thread::sleep(Duration::from_millis(100)); }
    Ok(())
}
