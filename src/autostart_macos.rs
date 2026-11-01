use anyhow::{bail, Result};

pub fn enabled() -> bool { false }

pub fn set(_on: bool) -> Result<()> {
    bail!("macOS autostart is not implemented yet")
}
