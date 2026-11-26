//! What the Notification Center's panel and its top bar button share: the
//! translations, and how to tell the panel to open.

pub mod i18n;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Where the panel listens (`toggle`, `show`, `hide`, one per line).
pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(|d| PathBuf::from(d).join("iliada-notification-center.sock"))
}

/// Tell the running panel `what`; start it (showing) if it is not running.
pub fn tell_panel(what: &str) {
    if let Some(p) = socket_path()
        && let Ok(mut s) = UnixStream::connect(p)
        && writeln!(s, "{what}").is_ok()
    {
        return;
    }
    let _ = std::process::Command::new("iliada-notification-center").arg(what).spawn();
}
