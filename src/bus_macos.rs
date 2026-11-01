use crate::state::State;

/// fbdirector claims Windows scan codes. It has no macOS implementation yet.
pub fn run(_state: &State) -> anyhow::Result<()> { Ok(()) }
