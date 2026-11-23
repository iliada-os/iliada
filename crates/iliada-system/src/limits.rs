//! `/etc/iliada/permissions.toml`: the computer's limits on what apps may
//! use (ADR 0007). Only an administrator changes it, through this service,
//! so every change is a commit: recorded, backed up and undoable. A limit
//! only takes access away; each person's own answers live in their
//! `~/.config/iliada/permissions.toml`.

use anyhow::{Result, bail};
use iliada_bundle::policy::{self, Key, Layer, Value};
use toml_edit::{DocumentMut, Item, Table, value};

const HEADER: &str = "\
# Limits on what apps may use on this computer, for everyone on it.
#
# Each person decides what apps may use in their own
# ~/.config/iliada/permissions.toml; a limit here always wins over it, and
# can only take access away:
#   \"deny\"        never, whatever anyone answers
#   \"read-only\"   a folder may be read but not changed
# `[all]` applies to every app; `[apps.\"<id>\"]` to one.
# Change it with `iliada permissions limit <app|all> <capability> deny|read-only|none`
# (administrators). Every change is recorded and backed up.
# Format: docs/permissions-schema.md.

version = 1
";

/// Set (or, with `None`, remove) one limit in the file's text. `app` is an
/// app id, or `all`. Returns the new text and a description of the change.
pub fn set(text: &str, app: &str, key: &str, limit: Option<&str>) -> Result<(String, String)> {
    let Some(k) = Key::parse(key) else {
        let known: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
        bail!("{key:?} is not a capability (one of: {})", known.join(", "));
    };
    let limit = match limit {
        None | Some("none") => None,
        Some(s) => match Value::parse(s) {
            Some(v @ Value::Deny) => Some(v),
            Some(v @ Value::ReadOnly) if k.folder().is_some() => Some(v),
            _ => bail!("a limit is \"deny\"{} or \"none\"", if k.folder().is_some() { ", \"read-only\"" } else { "" }),
        },
    };
    let valid_id = !app.is_empty()
        && app.len() <= 255
        && app.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    if !valid_id {
        bail!("{app:?} is not an app id");
    }
    let start = if text.trim().is_empty() { HEADER } else { text };
    let mut doc: DocumentMut = start.parse()?;
    let table: &mut Table = if app == "all" {
        doc.entry("all")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[all] is not a table"))?
    } else {
        let apps = doc.entry("apps").or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        let apps = apps.as_table_mut().ok_or_else(|| anyhow::anyhow!("apps is not a table"))?;
        apps.entry(app)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("apps.{app} is not a table"))?
    };
    match limit {
        Some(v) => {
            table.insert(k.name(), value(v.name()));
        }
        None => {
            table.remove(k.name());
        }
    }
    let out = doc.to_string();
    let check = policy::parse(&out, "permissions.toml", Layer::System);
    if check.broken {
        bail!("the computer's permissions file would not be readable: {}", check.problems[0].message);
    }
    let who = if app == "all" { "every app".to_owned() } else { app.to_owned() };
    let what = match limit {
        Some(v) => format!("Limit {} to {v} for {who}", k.name()),
        None => format!("Remove the limit on {} for {who}", k.name()),
    };
    Ok((out, what))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_written_readably_and_removed() {
        let (t, what) = set("", "all", "network", Some("deny")).unwrap();
        assert_eq!(what, "Limit network to deny for every app");
        assert!(t.starts_with("# Limits on what apps may use"), "{t}");
        let (t, _) = set(&t, "org.gimp.GIMP", "files.pictures", Some("read-only")).unwrap();
        assert!(
            t.contains("[apps.\"org.gimp.GIMP\"]") || t.contains("[apps.'org.gimp.GIMP']") || t.contains("[apps.org"),
            "{t}"
        );
        let loaded = policy::parse(&t, "x", Layer::System);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(loaded.apps["*"][&Key::Network], Value::Deny);
        let (t, _) = set(&t, "all", "network", None).unwrap();
        assert!(!policy::parse(&t, "x", Layer::System).apps["*"].contains_key(&Key::Network));
    }

    #[test]
    fn only_narrowing_limits_are_accepted() {
        assert!(set("", "all", "camera", Some("allow")).is_err());
        assert!(set("", "all", "camera", Some("read-only")).is_err());
        assert!(set("", "all", "teleport", Some("deny")).is_err());
        assert!(set("", "bad id!", "camera", Some("deny")).is_err());
        assert!(set("apps = 1\n", "x.y", "camera", Some("deny")).is_err());
    }
}
