// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * tray.rs
 *
 * Version: @(#)tray.rs 0.3.1 2026
 *
 * Description: The Little Light.
 *              Four colour tiles, one per mode. Left click open Settings. Right
 *              click for people who like menus. Quit live here, and only here.
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

//   DivineVoice — idle (paste · Ctrl+Space)   ← status, greyed out
//   Settings…
//   Open log folder
//   ───────────
//   Quit
//
// The icon is a status light: one 32×32 RGBA tile per Mode, baked into the exe.

use anyhow::Result;
use std::sync::atomic::Ordering;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::config::APP;
use crate::state::{Mode, State};

pub enum Action { Settings, Logs, Quit }

pub struct Tray {
    icon:     TrayIcon,
    status:   MenuItem,
    settings: MenuId,
    logs:     MenuId,
    quit:     MenuId,
}

impl Tray {
    pub fn new(state: &State) -> Result<Self> {
        let menu     = Menu::new();
        let status   = MenuItem::new(status_line(state), false, None);
        let settings = MenuItem::new("Settings…", true, None);
        let logs     = MenuItem::new("Open log folder", true, None);
        let quit     = MenuItem::new("Quit", true, None);
        menu.append_items(&[&status, &PredefinedMenuItem::separator(), &settings, &logs,
                            &PredefinedMenuItem::separator(), &quit])?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(APP)
            .with_icon(tile(Mode::Idle))
            .build()?;

        Ok(Self { icon, status, settings: settings.id().clone(), logs: logs.id().clone(), quit: quit.id().clone() })
    }

    pub fn action(&self, id: &MenuId) -> Option<Action> {
        [(&self.settings, Action::Settings), (&self.logs, Action::Logs), (&self.quit, Action::Quit)]
            .into_iter()
            .find_map(|(mine, a)| (mine == id).then_some(a))
    }

    pub fn refresh(&self, state: &State) {
        let s = status_line(state);
        let _ = self.icon.set_icon(Some(tile(state.mode())));
        let _ = self.icon.set_tooltip(Some(&s));
        self.status.set_text(&s);
    }
}

fn status_line(state: &State) -> String {
    let c = state.cfg.read();
    let fb = match (c.fbdirector.is_some(), state.bus_connected.load(Ordering::Acquire)) {
        (false, _)    => "",
        (true, true)  => " · fb:ok",
        (true, false) => " · fb:--",
    };
    format!("{APP} — {} ({} · {}{fb})", state.mode().label(), c.inject_mode.short(), c.hotkey)
}

// Four 32×32 RGBA tiles back to back: Idle, Recording, Transcribing, Injecting.
static TILES: &[u8] = include_bytes!("../icons/mic_tiles.bin");
const TILE: usize = 32 * 32 * 4;
const _: () = assert!(TILES.len() == 4 * TILE, "mic_tiles.bin must be 4 tiles of 32x32 RGBA");

pub fn rgba(m: Mode) -> Vec<u8> {
    let i = m as usize;
    TILES[i * TILE..(i + 1) * TILE].to_vec()
}

fn tile(m: Mode) -> Icon { Icon::from_rgba(rgba(m), 32, 32).expect("tile is valid RGBA") }
