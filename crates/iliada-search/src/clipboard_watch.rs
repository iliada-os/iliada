//! Watch the clipboard through the data-control protocol.
//!
//! Only an unsandboxed program can bind this (COSMIC withholds it from
//! sandboxed display sockets). The search service records each copy in
//! [`clipboard::Store`]. Primary selection (select-to-copy) is ignored.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_source_v1::ZwlrDataControlSourceV1;

use crate::clipboard::{Incoming, Store, choose_mime, is_secret};

/// The app id of the focused window, from `lswt`, when it reports one.
pub fn focused_app() -> Option<String> {
    let out = Command::new("lswt").arg("-j").stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    focused_app_from_json(&String::from_utf8_lossy(&out.stdout))
}

/// `lswt -j`: looks for `activated`, `focused`, or a `state` list.
pub fn focused_app_from_json(text: &str) -> Option<String> {
    iliada_types::toplevel::focused_app_id(text)
}

struct State {
    store: Arc<Mutex<Store>>,
    conn: Connection,
    mimes: HashMap<ZwlrDataControlOfferV1, Vec<String>>,
    selection: Option<ZwlrDataControlOfferV1>,
}

impl State {
    fn take_offer(&mut self, offer: ZwlrDataControlOfferV1, mimes: Vec<String>) {
        if is_secret(&mimes) {
            offer.destroy();
            return;
        }
        let Some(mime) = choose_mime(&mimes) else {
            offer.destroy();
            return;
        };
        let data = match receive(&self.conn, &offer, &mime) {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("clipboard offer not read: {e:#}");
                offer.destroy();
                return;
            }
        };
        offer.destroy();
        let incoming = Incoming { mimes, mime, data, source: focused_app() };
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = store.record(incoming);
    }
}

fn receive(conn: &Connection, offer: &ZwlrDataControlOfferV1, mime: &str) -> Result<Vec<u8>> {
    let (mut rx, tx) = std::io::pipe().context("clipboard pipe")?;
    offer.receive(mime.to_owned(), tx.as_fd());
    drop(tx);
    conn.flush().context("asking the display for the copy")?;
    let mut data = Vec::new();
    rx.read_to_end(&mut data).context("reading the copy")?;
    Ok(data)
}

/// Watch the clipboard until the connection closes. Run on its own thread.
pub fn watch(store: Arc<Mutex<Store>>) -> Result<()> {
    let conn = Connection::connect_to_env().context("no display to watch the clipboard on")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).context("reading the display's globals")?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .context("the display offers no clipboard manager (zwlr_data_control_manager_v1)")?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ()).context("the display has no seat")?;
    let _device = manager.get_data_device(&seat, &qh, ());
    let mut state = State { store, conn: conn.clone(), mimes: HashMap::new(), selection: None };
    loop {
        queue.blocking_dispatch(&mut state).context("clipboard display events")?;
        if let Some(offer) = state.selection.take() {
            let mimes = state.mimes.remove(&offer).unwrap_or_default();
            state.take_offer(offer, mimes);
        }
    }
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
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Name { .. } = event {
            let _ = seat;
        }
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlSourceV1,
        _: <ZwlrDataControlSourceV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.mimes.insert(id, Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if let Some(old) = state.selection.take() {
                    old.destroy();
                }
                state.selection = id;
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.mimes.remove(&offer);
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: None } => {}
            zwlr_data_control_device_v1::Event::Finished => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.mimes.entry(offer.clone()).or_default().push(mime_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_window_is_read_from_lswt() {
        let json = r#"{"toplevels":[
            {"title":"Inbox","app-id":"firefox","state":["maximized"]},
            {"title":"Notes","app-id":"com.system76.CosmicEdit","activated":true}
        ]}"#;
        assert_eq!(focused_app_from_json(json).as_deref(), Some("com.system76.CosmicEdit"));
        let json = r#"{"toplevels":[{"app_id":"org.keepassxc.KeePassXC","focused":true}]}"#;
        assert_eq!(focused_app_from_json(json).as_deref(), Some("org.keepassxc.KeePassXC"));
        assert!(focused_app_from_json(r#"{"toplevels":[{"app-id":"a"}]}"#).is_none());
    }
}
