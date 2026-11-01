use anyhow::Result;
use crossbeam_channel::Receiver;
use std::time::Instant;
use tracing::warn;

use crate::perf::{Entry, Transcript};
use crate::state::{Mode, State};
use crate::ui;

pub fn record(state: &State, entry: Entry) {
    let max = state.cfg.read().perf_max;
    state.perf.push(entry, max);
    ui::push("perf", &crate::ipc::perf_view(state));
}

/// Placeholder for pasteboard and Core Graphics delivery. It consumes queued
/// text so a future macOS hotkey adapter cannot leave the state machine stuck.
pub fn run(state: &State, texts: &Receiver<Transcript>) -> Result<()> {
    for transcript in texts.iter().take_while(|_| state.alive()) {
        warn!("macOS text delivery is not implemented yet; transcript was not inserted");
        let now = Instant::now();
        record(state, transcript.finish(now, now, "unsupported"));
        state.advance(Mode::Injecting, Mode::Idle);
    }
    Ok(())
}
