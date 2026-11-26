//! Accessibility for sandboxed apps, so screen readers and Homer can read
//! and press their controls (Homer's app control, ADR 0007).
//!
//! A sandbox has no D-Bus session bus. It gets one filtered connection
//! instead: an `xdg-dbus-proxy` to the accessibility bus that lets the app
//! do only what an accessible app needs (put itself in the registry, ask
//! which events are wanted), with the same rules as Flatpak. Calls *to* the
//! app (reading its controls) pass, as for any app on that bus.
//!
//! The proxy's pid is written to `$XDG_RUNTIME_DIR/iliada/a11y/<pid>` with
//! the app's id and name: on the accessibility bus the app shows up as the
//! proxy, and that is how `iliada-control` knows which app it is.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use command_fds::{CommandFdExt, FdMapping};

/// Where the app finds it inside the sandbox.
pub const GUEST_SOCKET: &str = "/run/user/app/at-spi/bus";

/// What a sandboxed app may say on the accessibility bus (Flatpak's rules).
pub const FILTER: [&str; 9] = [
    "--filter",
    "--sloppy-names",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.Socket.Embed@/org/a11y/atspi/accessible/root",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.Socket.Unembed@/org/a11y/atspi/accessible/root",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.Registry.GetRegisteredEvents@/org/a11y/atspi/registry",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.DeviceEventController.GetKeystrokeListeners@/org/a11y/atspi/registry/deviceeventcontroller",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.DeviceEventController.GetDeviceEventListeners@/org/a11y/atspi/registry/deviceeventcontroller",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.DeviceEventController.NotifyListenersSync@/org/a11y/atspi/registry/deviceeventcontroller",
    "--call=org.a11y.atspi.Registry=org.a11y.atspi.DeviceEventController.NotifyListenersAsync@/org/a11y/atspi/registry/deviceeventcontroller",
];

/// The environment that turns accessibility on in GTK, Qt and Chromium.
pub fn env() -> Vec<(String, String)> {
    [
        ("AT_SPI_BUS_ADDRESS", format!("unix:path={GUEST_SOCKET}")),
        ("ACCESSIBILITY_ENABLED", "1".into()),
        ("QT_LINUX_ACCESSIBILITY_ALWAYS_ON", "1".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// A running proxy; stops (and its files go) when dropped, or when this
/// process ends however it ends.
#[derive(Debug)]
pub struct Proxy {
    child: Child,
    pub socket: PathBuf,
    map: PathBuf,
    // Closing it stops the proxy (`--fd`).
    _sync: std::io::PipeReader,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.map);
    }
}

/// Whether the person lets Homer use this app (and so its controls are
/// readable): not when they said no, or switched it off.
pub fn wanted(app_id: &str) -> bool {
    use iliada_bundle::control::{Control, ControlPolicy, SystemControl};
    let policy = ControlPolicy::load_user();
    policy.enabled && policy.decide(&SystemControl::load(), app_id) != Control::Deny
}

/// The accessibility bus's address on this desktop.
fn bus_address() -> Option<String> {
    if let Ok(a) = std::env::var("AT_SPI_BUS_ADDRESS")
        && !a.is_empty()
    {
        return Some(a);
    }
    let out = Command::new("busctl")
        .args(["--user", "call", "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_busctl_string(&String::from_utf8_lossy(&out.stdout))
}

/// `s "unix:path=/run/user/1000/at-spi/bus,guid=…"` → the string.
fn parse_busctl_string(out: &str) -> Option<String> {
    let s = out.trim().strip_prefix("s ")?.trim();
    let s = s.strip_prefix('"')?.strip_suffix('"')?;
    (!s.is_empty()).then(|| s.to_owned())
}

/// Starts the proxy for an app (`None`: no accessibility bus here, or no
/// xdg-dbus-proxy; the app then runs without accessibility, as before).
pub fn start(runtime: &Path, app_id: &str, app_name: &str) -> Option<Proxy> {
    let address = bus_address()?;
    let dir = runtime.join("iliada/a11y");
    std::fs::create_dir_all(&dir).ok()?;
    let socket = dir.join(format!("{app_id}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let (reader, writer) = std::io::pipe().ok()?;
    let mut cmd = Command::new("xdg-dbus-proxy");
    cmd.arg("--fd=3").arg(&address).arg(&socket).args(FILTER);
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    cmd.fd_mappings(vec![FdMapping { parent_fd: writer.into(), child_fd: 3 }]).ok()?;
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("no accessibility for sandboxed apps: {e}");
            return None;
        }
    };
    // `cmd` still holds our copy of the write end: dropped, so the read
    // below ends if the proxy fails before it is ready.
    drop(cmd);
    let mut reader = reader;
    let mut ready = [0u8; 1];
    let map = dir.join(child.id().to_string());
    let mut proxy = Proxy { child, socket, map, _sync: reader.try_clone().ok()? };
    if reader.read(&mut ready).ok()? != 1 {
        let _ = proxy.child.wait();
        return None;
    }
    std::fs::write(&proxy.map, format!("{app_id}\n{app_name}\n")).ok()?;
    proxy._sync = reader;
    Some(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_address_is_read_from_busctl() {
        assert_eq!(
            parse_busctl_string("s \"unix:path=/run/user/1000/at-spi/bus_0,guid=abc\"\n").as_deref(),
            Some("unix:path=/run/user/1000/at-spi/bus_0,guid=abc")
        );
        assert_eq!(parse_busctl_string(""), None);
        assert!(env().iter().any(|(k, v)| k == "AT_SPI_BUS_ADDRESS" && v == "unix:path=/run/user/app/at-spi/bus"));
    }
}
