//! What you open, and how often and how lately (frecency): results you
//! use rise to the top, and the empty search shows them.
//!
//! Kept in `$XDG_STATE_HOME/iliada/search-usage.json`, on this computer
//! only. `iliada-search forget` empties it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Hit;

/// One remembered result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Use {
    pub count: u32,
    /// Seconds since the Unix epoch.
    pub last: u64,
    /// The result as it was shown, to show it again on the empty search.
    pub hit: Hit,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub uses: HashMap<String, Use>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

/// At most this many remembered results (the least used go first).
const KEEP: usize = 500;

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Usage {
    pub fn default_path() -> Option<PathBuf> {
        crate::state_dir().map(|d| d.join("search-usage.json"))
    }

    pub fn load(path: &Path) -> Self {
        let mut u: Usage = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        u.path = Some(path.to_owned());
        u
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, path)
    }

    /// `hit` was chosen.
    pub fn used(&mut self, hit: &Hit) {
        if hit.key.is_empty() {
            return;
        }
        let mut stored = hit.clone();
        stored.score = 0;
        let e = self.uses.entry(hit.key.clone()).or_insert(Use { count: 0, last: 0, hit: stored.clone() });
        e.count = e.count.saturating_add(1);
        e.last = now();
        e.hit = stored;
        if self.uses.len() > KEEP {
            let t = now();
            if let Some(k) = self.uses.iter().min_by_key(|(_, u)| boost_at(u, t)).map(|(k, _)| k.clone()) {
                self.uses.remove(&k);
            }
        }
    }

    pub fn forget(&mut self) {
        self.uses.clear();
    }

    /// How much using `key` lifts it.
    pub fn boost(&self, key: &str) -> i64 {
        self.uses.get(key).map_or(0, |u| boost_at(u, now()))
    }

    /// The most used results, most first.
    pub fn top(&self, n: usize) -> Vec<Hit> {
        let t = now();
        let mut v: Vec<_> = self.uses.values().collect();
        v.sort_by_key(|u| std::cmp::Reverse(boost_at(u, t)));
        v.into_iter().take(n).map(|u| u.hit.clone()).collect()
    }
}

/// Frequency, weighed by how recent: a result used often this week beats
/// one used often last year.
fn boost_at(u: &Use, now: u64) -> i64 {
    let age = now.saturating_sub(u.last);
    let recency = match age {
        a if a < 3600 => 1.0,
        a if a < 86_400 => 0.8,
        a if a < 7 * 86_400 => 0.6,
        a if a < 30 * 86_400 => 0.35,
        _ => 0.15,
    };
    (120.0 * (1.0 + f64::from(u.count)).ln() * recency) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Kind};

    #[test]
    fn remembers_and_ranks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.json");
        let mut u = Usage::load(&path);
        let a = Hit::new(Kind::App, "Files", Action::Run(vec!["cosmic-files".into()])).key("app:files");
        let b = Hit::new(Kind::App, "Terminal", Action::Run(vec!["cosmic-term".into()])).key("app:term");
        u.used(&a);
        u.used(&b);
        u.used(&b);
        assert!(u.boost("app:term") > u.boost("app:files"));
        assert_eq!(u.boost("nothing"), 0);
        u.save().unwrap();
        let u = Usage::load(&path);
        assert_eq!(u.top(5).iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), vec!["Terminal", "Files"]);
        // Results without a key are never remembered.
        let mut u = u;
        u.used(&Hit::new(Kind::Answer, "= 4", Action::Copy("4".into())));
        assert_eq!(u.uses.len(), 2);
        u.forget();
        assert!(u.top(5).is_empty());
    }
}
