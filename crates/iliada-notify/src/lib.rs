//! Iliada's Notification Center (ADR 0010).
//!
//! `iliada-notifyd` owns `org.freedesktop.Notifications` on the session
//! bus. Every notification an app sends goes through it: it keeps the last
//! [`history::MAX`] in the person's state folder, applies the per-app rules
//! and Do Not Disturb, and hands the ones that should pop up to COSMIC's
//! banners. The panel that slides in from the right (Super+N), Settings →
//! Notifications, `iliada dnd` and Homer all read and change the same files
//! through this crate:
//!
//! * `~/.local/state/iliada/notifications.json`: the history ([`history`]);
//! * `~/.config/iliada/notifications.toml`: Do Not Disturb and the per-app
//!   rules ([`settings`], [`dnd`], [`rules`]);
//! * `$XDG_RUNTIME_DIR/iliada-notifyd.sock`: the service, for clearing and
//!   opening notifications ([`proto`]).

pub mod dnd;
pub mod history;
pub mod proto;
pub mod rules;
pub mod settings;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How urgent a notification is (the freedesktop `urgency` hint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    /// The hint's byte: 0 low, 1 normal, 2 critical.
    pub fn from_byte(b: u8) -> Urgency {
        match b {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// A button on a notification. `default` is what clicking the
/// notification itself does, and is not shown as a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
}

/// The action id for clicking the notification itself.
pub const DEFAULT_ACTION: &str = "default";

/// One notification, as the Notification Center keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// The id the app was given (it uses it to replace or close it).
    pub id: u32,
    /// The app's desktop entry id (the `desktop-entry` hint), or its name
    /// when it gave none. Rules and groups are by this.
    pub app_id: String,
    /// The name the app gave.
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub icon: String,
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub urgency: Urgency,
    /// When it arrived (Unix seconds).
    pub time: u64,
}

impl Notification {
    /// What groups and rules go by: the app id, or its name.
    pub fn app_key(&self) -> &str {
        if !self.app_id.trim().is_empty() {
            &self.app_id
        } else if !self.app_name.trim().is_empty() {
            &self.app_name
        } else {
            "unknown"
        }
    }

    /// The name to show for its app.
    pub fn app_label(&self) -> &str {
        if self.app_name.trim().is_empty() { self.app_key() } else { &self.app_name }
    }

    pub fn has_default(&self) -> bool {
        self.actions.iter().any(|a| a.id == DEFAULT_ACTION)
    }

    /// The buttons to show (every action but the default one).
    pub fn buttons(&self) -> impl Iterator<Item = &Action> {
        self.actions.iter().filter(|a| a.id != DEFAULT_ACTION && !a.label.trim().is_empty())
    }

    /// The body without markup (apps may send `<b>`, `<i>`, `<a href>`).
    pub fn plain_body(&self) -> String {
        strip_markup(&self.body)
    }
}

/// Text without the simple markup notifications may carry, with the five
/// XML entities decoded.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The app's id from a `desktop-entry` hint ("firefox.desktop" → "firefox").
pub fn desktop_id(hint: &str) -> String {
    hint.trim().strip_suffix(".desktop").unwrap_or(hint.trim()).to_owned()
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(fallback)))
}

/// `$XDG_STATE_HOME/iliada/notifications.json`.
pub fn history_path() -> Option<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state").map(|d| d.join("iliada/notifications.json"))
}

/// `$XDG_CONFIG_HOME/iliada/notifications.toml`.
pub fn settings_path() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config").map(|d| d.join("iliada/notifications.toml"))
}

/// `$XDG_RUNTIME_DIR/iliada-notifyd.sock`.
pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(|d| PathBuf::from(d).join("iliada-notifyd.sock"))
}

/// Write `bytes` to `path` all at once (a temporary file renamed over
/// it), readable only by the person.
pub(crate) fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_stripped() {
        assert_eq!(strip_markup("<b>Bold</b> &amp; <a href=\"x\">link</a> 1 &lt; 2"), "Bold & link 1 < 2");
        assert_eq!(strip_markup("plain"), "plain");
    }

    #[test]
    fn app_keys_and_buttons() {
        let mut n = Notification {
            id: 1,
            app_id: String::new(),
            app_name: "Thunderbird".into(),
            icon: String::new(),
            summary: "Mail".into(),
            body: String::new(),
            actions: vec![
                Action { id: "default".into(), label: "Open".into() },
                Action { id: "reply".into(), label: "Reply".into() },
            ],
            urgency: Urgency::Normal,
            time: 0,
        };
        assert_eq!(n.app_key(), "Thunderbird");
        n.app_id = desktop_id("org.mozilla.Thunderbird.desktop");
        assert_eq!(n.app_key(), "org.mozilla.Thunderbird");
        assert!(n.has_default());
        assert_eq!(n.buttons().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["reply"]);
        assert_eq!(Urgency::from_byte(2), Urgency::Critical);
        assert_eq!(Urgency::from_byte(7), Urgency::Normal);
    }
}
