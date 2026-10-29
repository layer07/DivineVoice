// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * bus.rs
 *
 * Version: @(#)bus.rs 0.3.1 2026
 *
 * Description: The fbdirector Handshake.
 *              Optional. Call fbdirector on loopback, say "this chord is mine",
 *              ping every two seconds. Rebind hotkey? Hang up, call again with
 *              new claim. Enum order IS the wire format. Touch it, fbdirector
 *              read gibberish.
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

// We connect, claim the scancodes of our hotkey chord so fbdirector stops
// forwarding it to the remote box, and ping every 2 s. Rebind the hotkey and we
// hang up and redial with the new claims.
//
// Wire: [u16 BE length][postcard(BusMsg)]. Variant ORDER is the wire format —
// postcard encodes the enum tag by index. Reorder these and fbdirector will
// receive gibberish and judge you.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::{debug, info, warn};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC};

use crate::binding::{self, Binding};
use crate::state::State;

#[derive(Debug, Serialize, Deserialize)]
enum BusMsg {
    Hello { app: String, claims: Vec<u16> },
    Ping,
    Bye,
}

const TICK: Duration = Duration::from_secs(2);

/// Set-1 scancodes for every key in the chord: modifiers (left-hand codes) +
/// the trigger, mapped through the active layout.
fn claims(b: Binding) -> Vec<u16> {
    const MOD_SC: [(u8, u16); 4] =
        [(binding::CTRL, 0x1D), (binding::SHIFT, 0x2A), (binding::ALT, 0x38), (binding::WIN, 0x5B)];
    let trigger = unsafe { MapVirtualKeyW(b.vk as u32, MAPVK_VK_TO_VSC) } as u16;
    MOD_SC.iter()
        .filter(|(bit, _)| b.mods & bit != 0)
        .map(|&(_, sc)| sc)
        .chain((trigger != 0).then_some(trigger))
        .collect()
}

pub fn run(state: &State) -> anyhow::Result<()> {
    while state.alive() {
        let target = state.cfg.read().fbdirector.clone();
        if let Some(addr) = target {
            match addr.parse::<SocketAddr>() {
                Err(e) => debug!("bus: bad fbdirector address {addr:?}: {e}"),
                Ok(sa) => match session(state, sa) {
                    Ok(())  => info!("bus: session closed"),
                    Err(e) if state.bus_connected.load(Ordering::Acquire) => warn!("bus: lost fbdirector: {e}"),
                    Err(e)  => debug!("bus: {e} (retrying)"),
                },
            }
            state.bus_connected.store(false, Ordering::Release);
        }
        std::thread::sleep(TICK);
    }
    Ok(())
}

fn session(state: &State, addr: SocketAddr) -> anyhow::Result<()> {
    let mut sock = TcpStream::connect_timeout(&addr, TICK)?;
    sock.set_nodelay(true)?;
    sock.set_write_timeout(Some(Duration::from_secs(3)))?;

    let (bound, addr_str) = { let c = state.cfg.read(); (c.hotkey, c.fbdirector.clone()) };
    send(&mut sock, &BusMsg::Hello { app: "divinevoice".into(), claims: claims(bound) })?;
    state.bus_connected.store(true, Ordering::Release);
    info!("bus: claimed {bound} with fbdirector at {addr}");

    // Hang up (cleanly) when the binding or the address changes; run() redials.
    let still_valid = || { let c = state.cfg.read(); c.hotkey == bound && c.fbdirector == addr_str };
    while state.alive() && still_valid() {
        std::thread::sleep(TICK);
        send(&mut sock, &BusMsg::Ping)?;
    }
    let _ = send(&mut sock, &BusMsg::Bye);
    Ok(())
}

fn send(sock: &mut TcpStream, msg: &BusMsg) -> anyhow::Result<()> {
    let body = postcard::to_allocvec(msg)?;
    let len  = u16::try_from(body.len())?;
    sock.write_all(&len.to_be_bytes())?;
    sock.write_all(&body)?;
    Ok(sock.flush()?)
}
