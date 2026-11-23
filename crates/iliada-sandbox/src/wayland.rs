//! A display socket that tells the compositor its clients are sandboxed.
//!
//! An app given the desktop's own Wayland socket looks, to COSMIC, like any
//! program the person started themselves: it could read the clipboard in
//! the background (data-control), capture the screen, draw over other
//! windows (layer-shell) or type into them (virtual keyboard). Instead,
//! each sandboxed app gets a socket of its own, made with the
//! `wp_security_context_v1` protocol: every connection to it carries the
//! sandbox engine, the app's id and an instance id, and COSMIC withholds
//! those privileged protocols from it. It is the mechanism Flatpak uses.
//!
//! The socket lives as long as the [`SecureDisplay`]: dropping it closes the
//! pipe the compositor watches, so the compositor stops accepting on it,
//! and removes the socket file.

use std::io::PipeWriter;
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::security_context::v1::client::wp_security_context_manager_v1::WpSecurityContextManagerV1;
use wayland_protocols::wp::security_context::v1::client::wp_security_context_v1::WpSecurityContextV1;

/// What COSMIC is told runs the app (anything but COSMIC's own panel
/// engine counts as sandboxed).
pub const SANDBOX_ENGINE: &str = "org.iliada-os.sandbox";

/// A sandboxed app's own display socket; see the module docs.
#[derive(Debug)]
pub struct SecureDisplay {
    path: PathBuf,
    // The compositor listens until this, the pipe's other end, is closed.
    _close: PipeWriter,
    _listener: UnixListener,
}

impl SecureDisplay {
    /// The socket to hand to the app (as its `wayland-0`).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SecureDisplay {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The socket's file name: the app id in safe characters, and this process.
fn socket_name(app_id: &str) -> String {
    let id: String =
        app_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("wayland-{id}-{}", std::process::id())
}

struct State;

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

impl Dispatch<WpSecurityContextManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextManagerV1,
        _: <WpSecurityContextManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpSecurityContextV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpSecurityContextV1,
        _: <WpSecurityContextV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Make a display socket in `dir` for `app_id`, through the compositor at
/// `host` (the desktop's own socket). Fails when the compositor does not
/// offer security contexts.
pub fn secure_display(host: &Path, dir: &Path, app_id: &str) -> Result<SecureDisplay> {
    let conn = Connection::from_socket(
        UnixStream::connect(host).with_context(|| format!("connecting to the display at {}", host.display()))?,
    )?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).context("reading the display's globals")?;
    let qh = queue.handle();
    let manager: WpSecurityContextManagerV1 =
        globals.bind(&qh, 1..=1, ()).context("the display offers no security contexts (wp_security_context_v1)")?;

    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(socket_name(app_id));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("creating {}", path.display()))?;
    let (close_read, close_write) = std::io::pipe()?;

    let context = manager.create_listener(listener.as_fd(), close_read.as_fd(), &qh, ());
    context.set_sandbox_engine(SANDBOX_ENGINE.into());
    context.set_app_id(app_id.into());
    context.set_instance_id(std::process::id().to_string());
    context.commit();
    context.destroy();
    manager.destroy();
    // The compositor has its own copies of both descriptors once it has
    // read the requests.
    queue.roundtrip(&mut State).context("handing the socket to the display")?;
    drop(close_read);
    Ok(SecureDisplay { path, _close: close_write, _listener: listener })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_names_are_safe() {
        let n = socket_name("org.debian/../x y");
        assert!(n.starts_with("wayland-org.debian_.._x_y-"));
        assert!(!n.contains('/'));
    }

    #[test]
    fn no_display_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(secure_display(&dir.path().join("none"), dir.path(), "a.b").is_err());
    }
}
