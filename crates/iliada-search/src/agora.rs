//! agora apps not installed yet, so "gimp" offers to install GIMP.
//!
//! The signed catalogue is downloaded and checked by agora's engine
//! ([`iliada_store`]), at start and every few hours, and a copy of what
//! the search needs is kept in `$XDG_CACHE_HOME/iliada/search-agora.json`
//! so a restarted service has it at once.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Action, Hit, Kind};

/// What the search keeps of a catalogue entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub category: String,
}

impl Offer {
    pub fn hit(&self) -> Hit {
        let summary = if self.summary.is_empty() { "In agora".to_owned() } else { self.summary.clone() };
        Hit::new(
            Kind::Agora,
            self.name.clone(),
            Action::Run(vec!["iliada-search".into(), "install".into(), self.id.clone(), self.name.clone()]),
        )
        .subtitle(format!("{summary} · Install from agora"))
        .icon_name("org.iliada.Agora")
    }
}

pub fn cache_path() -> Option<PathBuf> {
    crate::cache_dir().map(|d| d.join("search-agora.json"))
}

pub fn load_cache(path: &Path) -> Vec<Offer> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_cache(path: &Path, offers: &[Offer]) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(offers) {
        let _ = std::fs::write(path, b);
    }
}

/// Download and verify the catalogue (blocking; call off the main path).
pub fn fetch() -> anyhow::Result<Vec<Offer>> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let catalog = rt.block_on(async {
        let store = iliada_store::Store::new(iliada_store::StoreConfig::from_env()?)?;
        store.catalog().await
    })?;
    Ok(catalog
        .apps
        .into_iter()
        .map(|a| Offer { id: a.id, name: a.name, summary: a.summary, category: a.category })
        .collect())
}

/// Bundle names in these folders (`GIMP.app` → "gimp"), to leave installed
/// apps out of the offers.
pub fn installed(dirs: &[PathBuf]) -> std::collections::HashSet<String> {
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "app").then(|| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))?
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_install() {
        let o = Offer {
            id: "org.gimp.GIMP".into(),
            name: "GIMP".into(),
            summary: "Edit photos".into(),
            category: "Graphics".into(),
        };
        let h = o.hit();
        assert_eq!(h.subtitle, "Edit photos · Install from agora");
        assert_eq!(
            h.action,
            Action::Run(vec!["iliada-search".into(), "install".into(), "org.gimp.GIMP".into(), "GIMP".into()])
        );
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("GIMP.app")).unwrap();
        assert!(installed(&[d.path().to_owned()]).contains("gimp"));
        let c = d.path().join("c.json");
        save_cache(&c, std::slice::from_ref(&o));
        assert_eq!(load_cache(&c), vec![o]);
    }
}
