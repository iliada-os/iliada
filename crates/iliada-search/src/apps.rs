//! Apps: desktop entries (COSMIC's, Iliada's, and the launcher
//! `iliada-appd` writes for each `.app` bundle) and bundles in
//! `/Applications` without one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Action, Hit, Icon, Kind};

/// One app, ready to match.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub generic: String,
    pub comment: String,
    pub keywords: String,
    pub icon: Option<Icon>,
    pub exec: Vec<String>,
    /// File types it opens (`MimeType=`), for "Open With".
    pub mime: Vec<String>,
}

impl App {
    pub fn hit(&self) -> Hit {
        let subtitle = if self.comment.is_empty() { &self.generic } else { &self.comment };
        Hit {
            kind: Kind::App,
            title: self.name.clone(),
            subtitle: subtitle.clone(),
            icon: self.icon.clone(),
            action: Action::Run(self.exec.clone()),
            key: format!("app:{}", self.id),
            score: 0,
            clip_id: 0,
            pinned: false,
            at: 0,
            source: String::new(),
        }
    }
}

/// Where desktop entries are, most important first.
pub fn entry_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home =
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| crate::home().map(|h| h.join(".local/share")));
    dirs.extend(data_home);
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs.into_iter().map(|d| d.join("applications")).collect()
}

/// Split an `Exec=` line into arguments, without field codes (`%f`, `%U`…).
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            // %f %F %u %U %i %c %k and the deprecated ones are dropped.
            '%' => {
                if chars.next() == Some('%') {
                    cur.push('%');
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        args.push(cur);
    }
    args.retain(|a| !a.is_empty());
    args
}

/// Parse one desktop entry. `None`: not an app shown to people.
pub fn parse_entry(id: &str, text: &str) -> Option<App> {
    let mut in_main = false;
    let mut app = App {
        id: id.to_owned(),
        name: String::new(),
        generic: String::new(),
        comment: String::new(),
        keywords: String::new(),
        icon: None,
        exec: Vec::new(),
        mime: Vec::new(),
    };
    let mut kind_ok = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Type" => kind_ok = v == "Application",
            "Name" => app.name = v.to_owned(),
            "GenericName" => app.generic = v.to_owned(),
            "Comment" => app.comment = v.to_owned(),
            "Keywords" => app.keywords = v.replace(';', " ").trim().to_owned(),
            "Icon" if !v.is_empty() => {
                app.icon = Some(if v.starts_with('/') { Icon::Path(v.into()) } else { Icon::Name(v.to_owned()) })
            }
            "Exec" => app.exec = split_exec(v),
            "MimeType" => app.mime = v.split(';').filter(|m| !m.is_empty()).map(str::to_owned).collect(),
            "NoDisplay" | "Hidden" if v.eq_ignore_ascii_case("true") => return None,
            "OnlyShowIn" if !v.split(';').any(|d| d == "COSMIC") => return None,
            "NotShowIn" if v.split(';').any(|d| d == "COSMIC") => return None,
            _ => {}
        }
    }
    (kind_ok && !app.name.is_empty() && !app.exec.is_empty()).then_some(app)
}

/// Every app: desktop entries from `dirs` (the first with an id wins, as
/// XDG says), then bundles in `bundles` that have no entry.
pub fn load(dirs: &[PathBuf], bundles: &[PathBuf]) -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Some(app) = parse_entry(&id, &text)
            {
                apps.push(app);
            }
        }
    }
    let names: HashSet<String> = apps.iter().map(|a| a.name.to_lowercase()).collect();
    for dir in bundles {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "app") {
                continue;
            }
            let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if names.contains(&name.to_lowercase()) {
                continue;
            }
            apps.push(bundle_app(&path, name));
        }
    }
    apps
}

fn bundle_app(path: &Path, name: String) -> App {
    App {
        id: format!("bundle:{}", path.display()),
        name,
        generic: String::new(),
        comment: crate::tilde(path),
        keywords: String::new(),
        icon: Some(Icon::Name("application-x-executable".into())),
        exec: vec!["iliada-run".into(), path.display().to_string()],
        mime: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines() {
        assert_eq!(split_exec("firefox %u"), vec!["firefox"]);
        assert_eq!(
            split_exec(r#"iliada-run "/Applications/My App.app" -- %F"#),
            vec!["iliada-run", "/Applications/My App.app", "--"]
        );
        assert_eq!(split_exec("printf 100%%"), vec!["printf", "100%"]);
        assert_eq!(split_exec(r#"sh -c "echo \"hi\"""#), vec!["sh", "-c", "echo \"hi\""]);
    }

    #[test]
    fn entries() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[el]=Αρχεία\nComment=Browse files\nIcon=com.system76.CosmicFiles\nExec=cosmic-files %U\nKeywords=folder;manager;\n[Desktop Action new]\nName=New Window\n";
        let a = parse_entry("com.system76.CosmicFiles", text).unwrap();
        assert_eq!(a.name, "Files");
        assert_eq!(a.exec, vec!["cosmic-files"]);
        assert_eq!(a.keywords, "folder manager");
        assert!(parse_entry("x", "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n").is_none());
        assert!(parse_entry("x", "[Desktop Entry]\nType=Link\nName=X\nExec=x\n").is_none());
        assert!(parse_entry("x", "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;\n").is_none());
    }

    #[test]
    fn first_entry_wins_and_bundles_fill_in() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, apps_dir) = (d.path().join("a"), d.path().join("b"), d.path().join("Applications"));
        for p in [&a, &b, &apps_dir.join("Calculator.app"), &apps_dir.join("Paint.app")] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(a.join("x.desktop"), "[Desktop Entry]\nType=Application\nName=Mine\nExec=mine\n").unwrap();
        std::fs::write(b.join("x.desktop"), "[Desktop Entry]\nType=Application\nName=Theirs\nExec=t\n").unwrap();
        std::fs::write(b.join("calc.desktop"), "[Desktop Entry]\nType=Application\nName=Calculator\nExec=c\n").unwrap();
        let apps = load(&[a, b], &[apps_dir]);
        let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Mine", "Calculator", "Paint"]);
        assert_eq!(apps[2].exec[0], "iliada-run");
    }
}
