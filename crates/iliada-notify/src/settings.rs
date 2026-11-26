//! Settings → Notifications, kept in `~/.config/iliada/notifications.toml`
//! like Iliada's other settings (TOML in `~/.config/iliada`):
//!
//! ```toml
//! [do_not_disturb]
//! on = false               # switched on by hand
//! until = 1767225600       # when it ends by itself (iliada dnd on --until/--for)
//! scheduled = true         # every day from `from` to `to` (local time)
//! from = "23:00"
//! to = "07:00"
//! allow_critical = true    # critical notifications still show
//!
//! [apps."org.mozilla.firefox"]
//! name = "Firefox"
//! allow = true             # off: its notifications are dropped
//! banners = true           # pop up on the screen
//! sound = true
//! history = true           # off: never kept in the Notification Center
//! ```
//!
//! The notification service adds each app the first time it sends a
//! notification, so Settings can list them.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub do_not_disturb: DndSettings,
    #[serde(default)]
    pub apps: BTreeMap<String, AppRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DndSettings {
    /// Switched on by hand: the panel, Super+Shift+N, Settings,
    /// `iliada dnd on`, Homer.
    #[serde(default)]
    pub on: bool,
    /// When `on` ends by itself (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    /// On every day from `from` to `to` (local time, "HH:MM"; across
    /// midnight when `to` is earlier).
    #[serde(default)]
    pub scheduled: bool,
    #[serde(default = "default_from")]
    pub from: String,
    #[serde(default = "default_to")]
    pub to: String,
    /// Switched off by hand while the schedule had it on: off until then
    /// (the schedule's end), and on again the next time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<u64>,
    /// Critical notifications (the freedesktop spec's urgency 2: a battery
    /// about to run out) still pop up and play their sound.
    #[serde(default = "yes")]
    pub allow_critical: bool,
}

fn default_from() -> String {
    "23:00".into()
}

fn default_to() -> String {
    "07:00".into()
}

fn yes() -> bool {
    true
}

impl Default for DndSettings {
    fn default() -> Self {
        DndSettings {
            on: false,
            until: None,
            scheduled: false,
            from: default_from(),
            to: default_to(),
            paused_until: None,
            allow_critical: true,
        }
    }
}

/// What one app may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRule {
    /// The name it gave, for Settings.
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Off: its notifications are dropped (no banner, no sound, not kept).
    #[serde(default = "yes")]
    pub allow: bool,
    #[serde(default = "yes")]
    pub banners: bool,
    #[serde(default = "yes")]
    pub sound: bool,
    /// Off: a private app. Its notifications can still pop up, but are
    /// never written to the history.
    #[serde(default = "yes")]
    pub history: bool,
}

impl Default for AppRule {
    fn default() -> Self {
        AppRule { name: String::new(), icon: String::new(), allow: true, banners: true, sound: true, history: true }
    }
}

const HEADER: &str = "\
# Notifications (Settings > Notifications). Iliada's notification service
# reads this file for every notification, so changes apply at once.
#
# [do_not_disturb]: no banners and no sounds while it is on (switched on by
# hand, until a time, or every day on a schedule). Notifications still go
# to the Notification Center (Super+N). allow_critical lets critical ones
# through.
# [apps.\"<id>\"]: each app that has sent a notification. allow = false
# drops its notifications; history = false never keeps them.

";

impl Settings {
    /// Missing or unreadable: the defaults (everything allowed, Do Not
    /// Disturb off).
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn load_user() -> Settings {
        crate::settings_path().map(|p| Settings::load(&p)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::atomic_write(path, format!("{HEADER}{text}").as_bytes())?;
        Ok(())
    }

    /// Change the person's settings: load, change, save.
    pub fn update(change: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let path = crate::settings_path().ok_or_else(|| anyhow::anyhow!("no home folder"))?;
        let mut s = Settings::load(&path);
        change(&mut s);
        s.save(&path)?;
        Ok(s)
    }

    /// The rule for an app (the defaults if it has none).
    pub fn rule(&self, app: &str) -> AppRule {
        self.apps.get(app).cloned().unwrap_or_default()
    }

    /// List an app that sent a notification; whether anything changed.
    pub fn remember(&mut self, app: &str, name: &str, icon: &str) -> bool {
        let mut changed = !self.apps.contains_key(app);
        let rule = self.apps.entry(app.to_owned()).or_default();
        if rule.name.is_empty() && !name.trim().is_empty() {
            rule.name = name.trim().to_owned();
            changed = true;
        }
        if rule.icon.is_empty() && !icon.trim().is_empty() && !icon.contains("://") {
            rule.icon = icon.trim().to_owned();
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iliada/notifications.toml");
        let s = Settings::load(&path);
        assert!(s.do_not_disturb.allow_critical && !s.do_not_disturb.on);
        assert_eq!((s.do_not_disturb.from.as_str(), s.do_not_disturb.to.as_str()), ("23:00", "07:00"));
        let rule = s.rule("anything");
        assert!(rule.allow && rule.banners && rule.sound && rule.history);

        let mut s = Settings::default();
        assert!(s.remember("org.mozilla.firefox", "Firefox", "firefox"));
        assert!(!s.remember("org.mozilla.firefox", "Firefox", "firefox"));
        s.apps.get_mut("org.mozilla.firefox").unwrap().banners = false;
        s.do_not_disturb.scheduled = true;
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back, s);
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("# Notifications"));
    }

    #[test]
    fn a_hand_written_file_with_only_some_keys() {
        let s: Settings = toml::from_str("[apps.\"chat\"]\nsound = false\n").unwrap();
        let r = s.rule("chat");
        assert!(!r.sound && r.allow && r.banners && r.history);
        assert!(s.do_not_disturb.allow_critical);
    }
}
