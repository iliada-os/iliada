//! Open windows, for Search: type part of a window's title or its app's
//! name and choose it to switch to it (on any workspace, minimized or not).
//!
//! The search service follows the compositor's list of windows
//! (ext-foreign-toplevel-list) as it changes, so each keystroke only reads
//! the list in memory. Focusing goes through COSMIC's toplevel management
//! (`zcosmic_toplevel_manager_v1.activate`), which also switches to the
//! window's workspace; minimized windows are restored first.
//!
//! The list stays in `iliada-searchd`, which runs outside the sandbox:
//! COSMIC withholds both protocols from sandboxed apps, and their private
//! runtime folder has no way to reach the search service. Protected windows
//! (prompts, Homer's banner) and Search's own are never listed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1};
use cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1};
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1;
use serde::{Deserialize, Serialize};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};

use crate::Engine;

/// One open window, as Search shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub title: String,
    pub app_id: String,
    /// ext-foreign-toplevel-list's identifier: the same while it is open.
    pub identifier: String,
    /// When it was last focused, as a counter (0: not since Search started).
    pub focused: u64,
}

/// Search's own windows (the overlay is a layer surface, so it should never
/// be in the list; this is in case it ever is).
const OWN_APP_IDS: [&str; 2] = ["org.iliada.Search", "iliada-search-overlay"];

/// Whether Search may list a window of this app: not a protected one
/// (prompts, Homer's banner), not Search's own, and not one with no name.
pub fn listable(app_id: &str, title: &str) -> bool {
    if app_id.trim().is_empty() && title.trim().is_empty() {
        return false;
    }
    !iliada_types::toplevel::is_protected(app_id) && !OWN_APP_IDS.iter().any(|o| app_id.eq_ignore_ascii_case(o))
}

/// Search's settings (Settings → Privacy → Search), in
/// `$XDG_CONFIG_HOME/iliada/search.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Show open windows in the results.
    #[serde(default = "on")]
    pub windows: bool,
}

fn on() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config { windows: true }
    }
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        crate::config_dir().map(|d| d.join("search.toml"))
    }

    pub fn load(path: &Path) -> Config {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!("# Search (Settings → Privacy).\n{}", toml::to_string_pretty(self)?);
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }
}

/// What focusing a window needs, filled in by [`watch`]: the display
/// connection, COSMIC's toplevel manager and each window's COSMIC handle.
#[derive(Debug, Default)]
pub struct Focus {
    live: Mutex<Option<Live>>,
}

#[derive(Debug)]
struct Live {
    conn: Connection,
    seat: wl_seat::WlSeat,
    manager: Option<ZcosmicToplevelManagerV1>,
    /// By identifier: the COSMIC handle, and whether it is minimized.
    handles: HashMap<String, (ZcosmicToplevelHandleV1, bool)>,
}

impl Focus {
    /// Bring the window with this identifier to the front, switching to its
    /// workspace and restoring it if it is minimized.
    pub fn focus(&self, identifier: &str) -> Result<()> {
        let guard = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let live = guard.as_ref().context("Search is not following open windows")?;
        let manager = live.manager.as_ref().context("this desktop does not let Search switch windows")?;
        let (handle, minimized) = live.handles.get(identifier).context("that window is no longer open")?;
        if *minimized {
            manager.unset_minimized(handle);
        }
        manager.activate(handle, &live.seat);
        live.conn.flush().context("asking the display to switch windows")?;
        Ok(())
    }
}

/// A window as the compositor reports it.
#[derive(Debug)]
struct Top {
    handle: ExtForeignToplevelHandleV1,
    cosmic: Option<ZcosmicToplevelHandleV1>,
    title: String,
    app_id: String,
    identifier: String,
    minimized: bool,
    activated: bool,
    focused: u64,
    /// Its first `done` arrived (its title and app id are known).
    ready: bool,
}

struct State {
    engine: Arc<Engine>,
    focus: Arc<Focus>,
    info: Option<ZcosmicToplevelInfoV1>,
    tops: Vec<Top>,
    /// Counts focus changes, for "most recently focused first".
    clock: u64,
    /// Something changed since the list was last handed to the engine.
    changed: bool,
}

impl State {
    /// Hand the current list to the engine, and the handles to [`Focus`].
    fn publish(&mut self) {
        let windows: Vec<Window> = self
            .tops
            .iter()
            .filter(|t| t.ready && listable(&t.app_id, &t.title))
            .map(|t| Window {
                title: t.title.clone(),
                app_id: t.app_id.clone(),
                identifier: t.identifier.clone(),
                focused: t.focused,
            })
            .collect();
        let handles = self
            .tops
            .iter()
            .filter(|t| listable(&t.app_id, &t.title))
            .filter_map(|t| Some((t.identifier.clone(), (t.cosmic.clone()?, t.minimized))))
            .collect();
        *self.engine.windows.write().unwrap_or_else(|e| e.into_inner()) = windows;
        if let Some(live) = self.focus.live.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            live.handles = handles;
        }
    }
}

/// Follow the open windows until the display connection closes. Run on its
/// own thread.
pub fn watch(engine: Arc<Engine>, focus: Arc<Focus>) -> Result<()> {
    let conn = Connection::connect_to_env().context("no display to list windows on")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).context("reading the display's globals")?;
    let qh = queue.handle();
    let _list: ExtForeignToplevelListV1 =
        globals.bind(&qh, 1..=1, ()).context("the display does not list windows (ext_foreign_toplevel_list_v1)")?;
    // Version 2 has get_cosmic_toplevel, which ties COSMIC's handle (state,
    // activating) to each listed window.
    let info: Option<ZcosmicToplevelInfoV1> = globals.bind(&qh, 2..=3, ()).ok();
    let manager: Option<ZcosmicToplevelManagerV1> = globals.bind(&qh, 1..=4, ()).ok();
    if info.is_none() || manager.is_none() {
        tracing::warn!("the display has no COSMIC toplevel management: open windows are listed but cannot be focused");
    }
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ()).context("the display has no seat")?;
    *focus.live.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Live { conn: conn.clone(), seat, manager, handles: HashMap::new() });
    let mut state = State { engine, focus, info, tops: Vec::new(), clock: 0, changed: false };
    tracing::info!("following open windows");
    loop {
        queue.blocking_dispatch(&mut state).context("window list events")?;
        if std::mem::take(&mut state.changed) {
            state.publish();
        }
    }
}

/// COSMIC's state array: 32-bit values in the machine's byte order.
fn states(raw: &[u8]) -> (bool, bool) {
    let mut minimized = false;
    let mut activated = false;
    for chunk in raw.as_chunks::<4>().0 {
        let v = u32::from_ne_bytes(*chunk);
        match zcosmic_toplevel_handle_v1::State::try_from(v) {
            Ok(zcosmic_toplevel_handle_v1::State::Minimized) => minimized = true,
            Ok(zcosmic_toplevel_handle_v1::State::Activated) => activated = true,
            _ => {}
        }
    }
    (minimized, activated)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZcosmicToplevelManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZcosmicToplevelManagerV1,
        _: <ZcosmicToplevelManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        list: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                let cosmic = state.info.as_ref().map(|info| info.get_cosmic_toplevel(&toplevel, qh, ()));
                state.tops.push(Top {
                    handle: toplevel,
                    cosmic,
                    title: String::new(),
                    app_id: String::new(),
                    identifier: String::new(),
                    minimized: false,
                    activated: false,
                    focused: 0,
                    ready: false,
                });
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                tracing::warn!("the display stopped listing windows");
                list.destroy();
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(i) = state.tops.iter().position(|t| &t.handle == handle) else { return };
        let top = &mut state.tops[i];
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => top.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => top.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => top.identifier = identifier,
            ext_foreign_toplevel_handle_v1::Event::Done => {
                top.ready = true;
                state.changed = true;
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                let top = state.tops.remove(i);
                if let Some(c) = top.cosmic {
                    c.destroy();
                }
                top.handle.destroy();
                state.changed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZcosmicToplevelInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zcosmic_toplevel_info_v1::Event::Done = event {
            state.changed = true;
        }
    }

    // Only sent before version 2; declared so an old compositor cannot
    // crash the service.
    wayland_client::event_created_child!(State, ZcosmicToplevelInfoV1, [
        zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (ZcosmicToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let zcosmic_toplevel_handle_v1::Event::State { state: raw } = event else { return };
        let Some(top) = state.tops.iter_mut().find(|t| t.cosmic.as_ref() == Some(handle)) else { return };
        let (minimized, activated) = states(&raw);
        if activated && !top.activated {
            state.clock += 1;
            top.focused = state.clock;
        }
        top.minimized = minimized;
        top.activated = activated;
        state.changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_and_own_windows_are_never_listed() {
        assert!(listable("firefox", "Inbox"));
        assert!(listable("", "Untitled program"));
        assert!(!listable("org.iliada.Prompt", "Allow Camera?"));
        assert!(!listable("org.iliada.HomerBanner", "Homer is using your computer"));
        assert!(!listable("xdg-desktop-portal-cosmic", "Share your screen"));
        assert!(!listable("org.iliada.Search", "Search"));
        assert!(!listable("", " "));
    }

    #[test]
    fn cosmic_states() {
        let raw: Vec<u8> = [0u32, 2].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(states(&raw), (false, true));
        let raw: Vec<u8> = [1u32].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(states(&raw), (true, false));
        assert_eq!(states(&[]), (false, false));
    }

    #[test]
    fn settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iliada/search.toml");
        assert!(Config::load(&path).windows, "on unless turned off");
        Config { windows: false }.save(&path).unwrap();
        assert!(!Config::load(&path).windows);
        std::fs::write(&path, "").unwrap();
        assert!(Config::load(&path).windows);
    }
}
