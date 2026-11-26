//! Which apps Homer may control (read their windows, press their buttons,
//! type into them, drive them through Chrome's remote control).
//!
//! Answers live in the permissions file (ADR 0007), the same file that
//! describes every other grant:
//!
//! ```toml
//! [homer]
//! # Settings → Homer → Use other apps.
//! control = true
//! # Screenshots and the virtual mouse and keyboard.
//! screen = "ask"   # allow | deny | ask
//!
//! [apps."com.system76.CosmicFiles"]
//! controlled_by_homer = "allow"   # allow | deny | ask
//! ```
//!
//! An app that is not listed is `ask`: the first time Homer wants to use it,
//! the person is asked. `/etc/iliada/permissions.toml` can switch control off
//! for everyone (`[homer] control = false`) or deny one app
//! (`controlled_by_homer = "deny"`); a "no" there always wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::policy::{self, Key, Layer, Policy, Value};

/// What the person decided about Homer controlling one app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Allow,
    Deny,
    /// Not decided yet: Homer asks first.
    #[default]
    Ask,
}

impl Control {
    fn from_value(v: Value) -> Self {
        match v {
            Value::Allow => Control::Allow,
            Value::Deny => Control::Deny,
            _ => Control::Ask,
        }
    }

    fn value(self) -> Value {
        match self {
            Control::Allow => Value::Allow,
            Control::Deny => Value::Deny,
            Control::Ask => Value::Ask,
        }
    }
}

/// The key for the whole screen (screenshots, the virtual mouse and keyboard).
pub const SCREEN: &str = "screen";

/// The system-wide permissions file.
pub const SYSTEM_FILE: &str = policy::SYSTEM_PATH;

/// The person's answers about Homer, read from (and written back to) the
/// permissions file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPolicy {
    /// The global switch: Homer may control other apps at all.
    pub enabled: bool,
    /// The whole screen: screenshots, the virtual mouse and keyboard.
    pub screen: Control,
    pub apps: BTreeMap<String, Control>,
}

impl Default for ControlPolicy {
    fn default() -> Self {
        Self { enabled: true, screen: Control::Ask, apps: BTreeMap::new() }
    }
}

/// What the system-wide file says (only the parts about Homer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemControl {
    /// `[homer] control = false` switches control off for everyone.
    pub off: bool,
    /// `[homer] screen = "deny"`.
    pub screen_denied: bool,
    /// Apps with `controlled_by_homer = "deny"`.
    pub denied: Vec<String>,
}

impl SystemControl {
    pub fn load() -> Self {
        Self::load_from(Path::new(SYSTEM_FILE))
    }

    /// Unreadable or missing means "nothing said", except a broken file,
    /// which means off: better to do nothing than to ignore a "no".
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(_) => Self { off: true, ..Self::default() },
            Ok(text) => {
                let loaded = policy::parse(&text, &path.display().to_string(), Layer::System);
                if loaded.broken {
                    return Self { off: true, ..Self::default() };
                }
                let denied = loaded
                    .apps
                    .iter()
                    .filter(|(_, caps)| caps.get(&Key::ControlledByHomer) == Some(&Value::Deny))
                    .map(|(id, _)| id.clone())
                    .collect();
                Self {
                    off: loaded.homer.control == Some(false),
                    screen_denied: loaded.homer.screen == Some(Value::Deny),
                    denied,
                }
            }
        }
    }
}

impl ControlPolicy {
    /// `$XDG_CONFIG_HOME/iliada/permissions.toml`.
    pub fn default_path() -> Option<PathBuf> {
        Policy::default_user_path()
    }

    /// Missing means the defaults (on, every app asked first). A file that
    /// cannot be read as a policy means off: better to do nothing than to
    /// ignore a "no" the person wrote by hand.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(_) => Self { enabled: false, ..Self::default() },
            Ok(text) => Self::from_loaded(&policy::parse(&text, &path.display().to_string(), Layer::User)),
        }
    }

    fn from_loaded(loaded: &policy::Loaded) -> Self {
        if loaded.broken {
            return Self { enabled: false, ..Self::default() };
        }
        let mut apps = BTreeMap::new();
        for (id, caps) in &loaded.apps {
            if let Some(v) = caps.get(&Key::ControlledByHomer) {
                let c = Control::from_value(*v);
                if c != Control::Ask {
                    apps.insert(id.clone(), c);
                }
            }
        }
        Self {
            enabled: loaded.homer.control.unwrap_or(true),
            screen: Control::from_value(loaded.homer.screen.unwrap_or(Value::Ask)),
            apps,
        }
    }

    pub fn load_user() -> Self {
        Self::from_policy(&Policy::load())
    }

    fn from_policy(p: &Policy) -> Self {
        let mut c = Self::from_loaded(&p.user);
        if !p.readable() {
            c.enabled = false;
        }
        c
    }

    /// Writes only Homer's keys, and keeps the rest of the permissions file
    /// as it is (comments, other grants, summaries).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut doc = match std::fs::read_to_string(path) {
            Ok(text) => text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut d = toml_edit::DocumentMut::new();
                d["version"] = toml_edit::value(policy::FORMAT_VERSION);
                d
            }
            Err(e) => return Err(e),
        };
        if doc.get("homer").is_none() {
            let mut homer = toml_edit::Table::new();
            homer.decor_mut().set_prefix("\n# Homer may use other apps (Settings > Homer, or `iliada-control`).\n");
            doc["homer"] = toml_edit::Item::Table(homer);
        }
        doc["homer"]["control"] = toml_edit::value(self.enabled);
        doc["homer"]["screen"] = toml_edit::value(self.screen.value().name());
        if doc.get("apps").is_none() {
            doc["apps"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let Some(apps) = doc["apps"].as_table_mut() else {
            return Err(std::io::Error::other("apps must be a table"));
        };
        let mut written = std::collections::BTreeSet::new();
        for (id, c) in &self.apps {
            written.insert(id.clone());
            let entry = apps.entry(id).or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
            let Some(entry) = entry.as_table_mut() else { continue };
            match c {
                Control::Ask => {
                    entry.remove("controlled_by_homer");
                }
                other => {
                    entry["controlled_by_homer"] = toml_edit::value(other.value().name());
                }
            }
        }
        // Forget answers that were cleared (Ask): drop the key if we no longer
        // list the app, but leave the rest of that app's table alone.
        let ids: Vec<String> = apps.iter().map(|(k, _)| k.to_owned()).collect();
        for id in ids {
            if written.contains(&id) {
                continue;
            }
            if let Some(entry) = apps.get_mut(&id).and_then(|i| i.as_table_mut()) {
                entry.remove("controlled_by_homer");
            }
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string())?;
        std::fs::rename(tmp, path)
    }

    pub fn set(&mut self, app: &str, c: Control) {
        if app == SCREEN {
            self.screen = c;
            return;
        }
        if c == Control::Ask {
            self.apps.remove(app);
        } else {
            self.apps.insert(app.to_owned(), c);
        }
    }

    /// The answer for `app`, with the system file applied: off there or for
    /// that app means deny; the person's own "deny" is never overridden.
    pub fn decide(&self, system: &SystemControl, app: &str) -> Control {
        if !self.enabled || system.off {
            return Control::Deny;
        }
        if app == SCREEN {
            return if system.screen_denied { Control::Deny } else { self.screen };
        }
        if system.denied.iter().any(|a| a == app || a == "*") {
            return Control::Deny;
        }
        self.apps.get(app).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlisted_apps_are_asked_and_no_always_wins() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("c/permissions.toml");
        let mut c = ControlPolicy::load(&p);
        assert!(c.enabled);
        let none = SystemControl::default();
        assert_eq!(c.decide(&none, "chrome"), Control::Ask);
        c.set("chrome", Control::Allow);
        c.set("files", Control::Deny);
        c.save(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("controlled_by_homer"), "{text}");
        assert!(text.contains("[homer]"), "{text}");
        let c = ControlPolicy::load(&p);
        assert_eq!(c.decide(&none, "chrome"), Control::Allow);
        assert_eq!(c.decide(&none, "files"), Control::Deny);

        let sys = t.path().join("system.toml");
        std::fs::write(
            &sys,
            "[apps.chrome]\ncontrolled_by_homer = \"deny\"\nnetwork = \"deny\"\n[apps.edit]\ncontrolled_by_homer = \"allow\"\n",
        )
        .unwrap();
        let s = SystemControl::load_from(&sys);
        assert_eq!(c.decide(&s, "chrome"), Control::Deny, "the system file's no wins");
        // The computer's file can only narrow, so a system "allow" is ignored.
        assert_eq!(c.decide(&s, "edit"), Control::Ask);
        assert_eq!(c.decide(&s, "files"), Control::Deny, "the person's no is kept");

        std::fs::write(&sys, "[homer]\ncontrol = false\n").unwrap();
        assert_eq!(c.decide(&SystemControl::load_from(&sys), "edit"), Control::Deny);
    }

    #[test]
    fn the_global_switch_and_a_broken_file_mean_off() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("permissions.toml");
        std::fs::write(&p, "[homer]\ncontrol = false\n").unwrap();
        assert_eq!(ControlPolicy::load(&p).decide(&SystemControl::default(), "x"), Control::Deny);
        std::fs::write(&p, "[homer]\ncontrol = maybe\n").unwrap();
        assert!(!ControlPolicy::load(&p).enabled);
    }

    #[test]
    fn save_keeps_the_rest_of_the_permissions_file() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("permissions.toml");
        std::fs::write(&p, "version = 1\n\n[apps.\"org.gimp.GIMP\"]\nname = \"GIMP\"\nnetwork = \"allow\"\n").unwrap();
        let mut c = ControlPolicy::load(&p);
        c.set("org.gimp.GIMP", Control::Allow);
        c.save(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("name = \"GIMP\""), "{text}");
        assert!(text.contains("network = \"allow\""), "{text}");
        assert!(text.contains("controlled_by_homer = \"allow\""), "{text}");
        assert!(text.contains("control = true"), "{text}");
    }
}
