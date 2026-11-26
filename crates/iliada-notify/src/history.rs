//! The Notification Center's history: the last [`MAX`] notifications,
//! kept across restarts in `~/.local/state/iliada/notifications.json`
//! (readable only by the person). The oldest go first when it is full.
//!
//! Notifications from private apps never get here: see
//! [`crate::rules::Decision::keep`].

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::Notification;

/// How many notifications are kept.
pub const MAX: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    /// Oldest first.
    #[serde(default)]
    pub items: Vec<Notification>,
}

/// One app's notifications in the panel, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub app_id: String,
    pub app_name: String,
    pub icon: String,
    pub items: Vec<Notification>,
}

impl History {
    /// Missing or unreadable means empty (a broken file is not fatal: the
    /// next notification starts a new one).
    pub fn load(path: &Path) -> History {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic_write(path, serde_json::to_string(self)?.as_bytes())?;
        Ok(())
    }

    /// Add one (or replace the one with its id, which then counts as new),
    /// dropping the oldest beyond [`MAX`].
    pub fn push(&mut self, n: Notification) {
        self.items.retain(|o| o.id != n.id);
        self.items.push(n);
        if self.items.len() > MAX {
            let extra = self.items.len() - MAX;
            self.items.drain(..extra);
        }
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    pub fn remove(&mut self, id: u32) -> Option<Notification> {
        let i = self.items.iter().position(|n| n.id == id)?;
        Some(self.items.remove(i))
    }

    /// Clear one app's notifications; the ids cleared.
    pub fn clear_app(&mut self, app: &str) -> Vec<u32> {
        let ids: Vec<u32> = self.items.iter().filter(|n| n.app_key() == app).map(|n| n.id).collect();
        self.items.retain(|n| n.app_key() != app);
        ids
    }

    pub fn clear_all(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.items).into_iter().map(|n| n.id).collect()
    }

    /// Newest first, at most `limit`.
    pub fn recent(&self, limit: usize) -> Vec<Notification> {
        self.items.iter().rev().take(limit).cloned().collect()
    }

    /// The largest id kept (new ids start after it, so they never clash
    /// with ones from before a restart).
    pub fn max_id(&self) -> u32 {
        self.items.iter().map(|n| n.id).max().unwrap_or(0)
    }

    /// Grouped by app, the app with the newest notification first, each
    /// group newest first.
    pub fn groups(&self) -> Vec<Group> {
        groups(self.items.iter().rev())
    }
}

/// Group notifications given newest first.
pub fn groups<'a>(newest_first: impl Iterator<Item = &'a Notification>) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for n in newest_first {
        match out.iter_mut().find(|g| g.app_id == n.app_key()) {
            Some(g) => g.items.push(n.clone()),
            None => out.push(Group {
                app_id: n.app_key().to_owned(),
                app_name: n.app_label().to_owned(),
                icon: n.icon.clone(),
                items: vec![n.clone()],
            }),
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn sample(id: u32, app: &str, time: u64) -> Notification {
    Notification {
        id,
        app_id: app.to_owned(),
        app_name: app.to_owned(),
        icon: String::new(),
        summary: format!("#{id}"),
        body: String::new(),
        actions: Vec::new(),
        urgency: crate::Urgency::Normal,
        time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_at_most_200_dropping_the_oldest() {
        let mut h = History::default();
        for i in 1..=250 {
            h.push(sample(i, "app", u64::from(i)));
        }
        assert_eq!(h.items.len(), MAX);
        assert_eq!(h.items.first().map(|n| n.id), Some(51));
        assert_eq!(h.recent(1)[0].id, 250);
        assert_eq!(h.max_id(), 250);
    }

    #[test]
    fn replacing_keeps_one_copy_and_makes_it_newest() {
        let mut h = History::default();
        h.push(sample(1, "a", 1));
        h.push(sample(2, "a", 2));
        let mut again = sample(1, "a", 3);
        again.summary = "updated".into();
        h.push(again);
        assert_eq!(h.items.len(), 2);
        assert_eq!(h.recent(1)[0].summary, "updated");
    }

    #[test]
    fn survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iliada/notifications.json");
        let mut h = History::default();
        h.push(sample(7, "org.example.Mail", 100));
        h.save(&path).unwrap();
        assert_eq!(History::load(&path), h);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Missing or broken: empty, not an error.
        assert!(History::load(&dir.path().join("none.json")).items.is_empty());
        std::fs::write(&path, "not json").unwrap();
        assert!(History::load(&path).items.is_empty());
    }

    #[test]
    fn grouped_by_app_newest_first() {
        let mut h = History::default();
        h.push(sample(1, "mail", 1));
        h.push(sample(2, "chat", 2));
        h.push(sample(3, "mail", 3));
        h.push(sample(4, "calendar", 4));
        let g = h.groups();
        let apps: Vec<&str> = g.iter().map(|g| g.app_id.as_str()).collect();
        assert_eq!(apps, ["calendar", "mail", "chat"]);
        let mail: Vec<u32> = g[1].items.iter().map(|n| n.id).collect();
        assert_eq!(mail, [3, 1]);
    }

    #[test]
    fn clearing() {
        let mut h = History::default();
        for (i, app) in ["a", "b", "a"].into_iter().enumerate() {
            h.push(sample(i as u32 + 1, app, 0));
        }
        assert_eq!(h.clear_app("a"), [1, 3]);
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.remove(2).map(|n| n.id), Some(2));
        assert!(h.remove(2).is_none());
        h.push(sample(9, "c", 0));
        assert_eq!(h.clear_all(), [9]);
        assert!(h.items.is_empty());
    }
}
