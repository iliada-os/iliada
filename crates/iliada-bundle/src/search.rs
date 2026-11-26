//! Search extensions: bundles that add results to Search under a keyword.
//!
//! ```toml
//! # Contents/Info.toml
//! [search]
//! keyword = "gh"                        # 1 to 12 lowercase letters or digits
//! title = "GitHub"                      # shown in results and Settings
//! placeholder = "Search repositories"   # optional
//! ```
//!
//! When the person types the keyword and a space, Search hands what follows
//! (and nothing else) to the extension, which runs in its sandbox (ADR 0009).
//!
//! Keywords are first come, first served: the extension installed first
//! keeps a keyword, a later one that declares the same keyword waits for the
//! person to choose another (Settings → Privacy → Search). Search's own
//! prefixes can never be claimed. The person's choices are kept in
//! `~/.config/iliada/search-extensions.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::{Bundle, BundleError};

/// `[search]` in `Contents/Info.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInfo {
    pub keyword: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// Search's own prefixes: `>` runs a command, `?` searches the web, `w `
/// lists windows, `~` and `/` browse folders.
pub const RESERVED: [&str; 5] = [">", "?", "w", "~", "/"];

/// The longest keyword.
pub const MAX_KEYWORD: usize = 12;

/// Why a keyword can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeywordError {
    /// Not 1 to 12 lowercase letters or digits.
    Invalid,
    /// One of Search's own prefixes.
    Reserved,
    /// Another extension (its title) has it.
    Taken(String),
}

impl std::fmt::Display for KeywordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeywordError::Invalid => write!(f, "a keyword is 1 to {MAX_KEYWORD} lowercase letters or digits"),
            KeywordError::Reserved => write!(f, "Search uses that keyword itself"),
            KeywordError::Taken(by) => write!(f, "{by} uses that keyword"),
        }
    }
}

/// Whether `keyword` has the right form and is not one of Search's own.
pub fn check_keyword(keyword: &str) -> Result<(), KeywordError> {
    if RESERVED.contains(&keyword) {
        return Err(KeywordError::Reserved);
    }
    let ok = (1..=MAX_KEYWORD).contains(&keyword.len())
        && keyword.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok { Ok(()) } else { Err(KeywordError::Invalid) }
}

impl SearchInfo {
    pub fn validate(&self) -> Result<(), BundleError> {
        check_keyword(&self.keyword)
            .map_err(|e| BundleError::Invalid(format!("search.keyword {:?}: {e}", self.keyword)))?;
        let title = self.title.trim();
        if title.is_empty() || title.chars().count() > 40 {
            return Err(BundleError::Invalid("search.title must be 1 to 40 characters".into()));
        }
        if self.placeholder.as_ref().is_some_and(|p| p.chars().count() > 80) {
            return Err(BundleError::Invalid("search.placeholder is longer than 80 characters".into()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The person's choices

/// `~/.config/iliada/search-extensions.toml`: extensions turned off, and
/// keywords chosen instead of the declared ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choices {
    #[serde(default)]
    pub extensions: BTreeMap<String, Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(default = "on")]
    pub enabled: bool,
}

fn on() -> bool {
    true
}

impl Default for Choice {
    fn default() -> Self {
        Choice { keyword: None, enabled: true }
    }
}

impl Choices {
    pub fn default_path() -> Option<PathBuf> {
        crate::policy::Policy::config_home().map(|c| c.join("iliada/search-extensions.toml"))
    }

    /// The file, or no choices when it is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(
            &tmp,
            format!("# Search extensions (Settings → Privacy → Search): turned off, or given another keyword.\n{body}"),
        )?;
        std::fs::rename(&tmp, path)
    }
}

// ---------------------------------------------------------------------------
// Installed extensions

/// An installed extension, before keywords are shared out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub search: SearchInfo,
    /// It declares the internet (`network = true`).
    pub network: bool,
    pub bundle: PathBuf,
    /// When it was installed (seconds since the epoch): earlier keeps a
    /// keyword.
    pub installed: u64,
}

/// An installed extension, as Search and Settings see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Extension {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// The keyword it answers to (the person's choice, or its own).
    pub keyword: String,
    /// The keyword its bundle declares.
    pub declared_keyword: String,
    /// The person has not turned it off.
    pub enabled: bool,
    /// It declares the internet (the permissions file may still say no).
    pub network: bool,
    /// The extension that has this keyword already: this one waits for a
    /// new keyword.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clashes_with: Option<String>,
    #[serde(skip)]
    pub bundle: PathBuf,
}

impl Extension {
    /// Search sends it queries.
    pub fn active(&self) -> bool {
        self.enabled && self.clashes_with.is_none()
    }
}

/// Share keywords out: in install order, the first to want a keyword gets
/// it; a later one clashes. A chosen keyword that is invalid or reserved
/// falls back to the declared one.
pub fn resolve(mut candidates: Vec<Candidate>, choices: &Choices) -> Vec<Extension> {
    candidates.sort_by(|a, b| (a.installed, &a.id).cmp(&(b.installed, &b.id)));
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(candidates.len());
    let mut seen = BTreeSet::new();
    for c in candidates {
        if !seen.insert(c.id.clone()) {
            continue;
        }
        let choice = choices.extensions.get(&c.id).cloned().unwrap_or_default();
        let keyword = choice.keyword.filter(|k| check_keyword(k).is_ok()).unwrap_or_else(|| c.search.keyword.clone());
        let clashes_with = if check_keyword(&keyword).is_err() {
            Some("Search".to_owned())
        } else {
            match taken.get(&keyword) {
                Some(holder) => Some(holder.clone()),
                None => {
                    taken.insert(keyword.clone(), c.search.title.clone());
                    None
                }
            }
        };
        out.push(Extension {
            id: c.id,
            title: c.search.title,
            placeholder: c.search.placeholder,
            keyword,
            declared_keyword: c.search.keyword,
            enabled: choice.enabled,
            network: c.network,
            clashes_with,
            bundle: c.bundle,
        });
    }
    out
}

/// Whether `keyword` is free for extension `id` among `extensions`.
pub fn keyword_free(extensions: &[Extension], id: &str, keyword: &str) -> Result<(), KeywordError> {
    check_keyword(keyword)?;
    match extensions.iter().find(|e| e.id != id && e.keyword == keyword && e.clashes_with.is_none()) {
        Some(holder) => Err(KeywordError::Taken(holder.title.clone())),
        None => Ok(()),
    }
}

/// When a bundle was installed: its folder's birth time where the file
/// system keeps one, else its last change.
fn installed_at(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.created().or_else(|_| m.modified()).ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(u64::MAX, |d| d.as_secs())
}

/// The candidate for an installed bundle, if it is an extension.
pub fn candidate(b: &Bundle) -> Option<Candidate> {
    let search = b.info.search.clone()?;
    Some(Candidate {
        id: b.info.id.as_str().to_owned(),
        search,
        network: b.info.network,
        bundle: b.layout.root().to_owned(),
        installed: installed_at(b.layout.root()),
    })
}

/// Every installed extension (`/Applications`), with the person's choices.
pub fn installed() -> Vec<Extension> {
    crate::policy::installed_bundles().into_iter().filter_map(|(_, f)| f.search).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, keyword: &str, installed: u64) -> Candidate {
        Candidate {
            id: id.into(),
            search: SearchInfo { keyword: keyword.into(), title: id.to_uppercase(), placeholder: None },
            network: false,
            bundle: PathBuf::from(format!("/Applications/{id}.app")),
            installed,
        }
    }

    #[test]
    fn keywords() {
        assert!(check_keyword("gh").is_ok() && check_keyword("tr").is_ok() && check_keyword("color").is_ok());
        assert!(check_keyword("abcdefghijkl").is_ok());
        assert_eq!(check_keyword("abcdefghijklm"), Err(KeywordError::Invalid));
        for bad in ["", "GH", "g h", "gh!", "ünï", "-", "gh "] {
            assert_eq!(check_keyword(bad), Err(KeywordError::Invalid), "{bad:?}");
        }
        for reserved in RESERVED {
            assert_eq!(check_keyword(reserved), Err(KeywordError::Reserved), "{reserved:?}");
        }
    }

    #[test]
    fn first_installed_keeps_the_keyword() {
        let e = resolve(vec![cand("b", "gh", 20), cand("a", "gh", 10), cand("c", "tr", 30)], &Choices::default());
        let by = |id: &str| e.iter().find(|x| x.id == id).unwrap();
        assert!(by("a").active());
        assert_eq!(by("b").clashes_with.as_deref(), Some("A"));
        assert!(!by("b").active());
        assert!(by("c").active());
        // The person gives the newer one another keyword.
        let mut choices = Choices::default();
        choices.extensions.insert("b".into(), Choice { keyword: Some("hub".into()), enabled: true });
        let e = resolve(vec![cand("a", "gh", 10), cand("b", "gh", 20)], &choices);
        assert!(e.iter().all(Extension::active));
        assert_eq!(e[1].keyword, "hub");
        assert_eq!(e[1].declared_keyword, "gh");
        // A reserved choice is ignored.
        choices.extensions.insert("b".into(), Choice { keyword: Some("w".into()), enabled: true });
        let e = resolve(vec![cand("a", "gh", 10), cand("b", "gh", 20)], &choices);
        assert_eq!(e[1].keyword, "gh");
        assert!(!e[1].active());
        // Turned off: not active, but it still holds its keyword.
        choices.extensions.insert("a".into(), Choice { keyword: None, enabled: false });
        let e = resolve(vec![cand("a", "gh", 10), cand("b", "gh", 20)], &choices);
        assert!(!e[0].active() && e[0].clashes_with.is_none());
        assert!(!e[1].active());
    }

    #[test]
    fn free_keywords() {
        let e = resolve(vec![cand("a", "gh", 10), cand("b", "gh", 20)], &Choices::default());
        assert_eq!(keyword_free(&e, "b", "gh"), Err(KeywordError::Taken("A".into())));
        assert_eq!(keyword_free(&e, "a", "gh"), Ok(()));
        assert_eq!(keyword_free(&e, "b", "w"), Err(KeywordError::Reserved));
        assert_eq!(keyword_free(&e, "b", "Hub"), Err(KeywordError::Invalid));
        assert_eq!(keyword_free(&e, "b", "hub"), Ok(()));
    }

    #[test]
    fn choices_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iliada/search-extensions.toml");
        assert_eq!(Choices::load(&path), Choices::default());
        let mut c = Choices::default();
        c.extensions.insert("org.example.gh".into(), Choice { keyword: Some("hub".into()), enabled: false });
        c.save(&path).unwrap();
        assert_eq!(Choices::load(&path), c);
        std::fs::write(&path, "[extensions.\"x\"]\nkeyword = \"k\"\n").unwrap();
        assert!(Choices::load(&path).extensions["x"].enabled, "on unless turned off");
    }
}
