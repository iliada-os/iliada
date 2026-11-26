//! Putting the sources together: one ranked list per keystroke.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::agora::Offer;
use crate::apps::App;
use crate::extensions::Extension;
use crate::files::FileIndex;
use crate::settings::{ACTIONS, PAGES};
use crate::usage::Usage;
use crate::windows::{Window, listable};
use crate::{Action, Hit, Icon, Kind, actions, calc};

/// Everything the search knows. Each part is swapped or updated on its
/// own (the file index while the home folder is walked, apps when one is
/// installed), behind its own lock.
#[derive(Debug)]
pub struct Engine {
    pub files: RwLock<FileIndex>,
    pub apps: RwLock<Vec<App>>,
    pub offers: RwLock<Vec<Offer>>,
    /// Installed bundle names, lower-case (offers for these are hidden).
    pub installed: RwLock<HashSet<String>>,
    pub usage: Mutex<Usage>,
    pub recent: Option<PathBuf>,
    /// The home folder is being indexed (results may be missing files).
    pub indexing: std::sync::atomic::AtomicBool,
    /// Open windows, in the compositor's order (kept current by
    /// [`crate::windows::watch`]).
    pub windows: RwLock<Vec<Window>>,
    /// Settings → Privacy → Search: show open windows.
    pub show_windows: std::sync::atomic::AtomicBool,
    /// Installed Search extensions (on or off, with their keywords).
    pub extensions: RwLock<Vec<Extension>>,
}

/// Rows of each kind, at most.
const FILES: usize = 12;
const WINDOWS: usize = 8;

/// An open window comes before its app's "open a new window" row when it
/// matches about as well.
const WINDOW_LEAD: i64 = 60;

/// Words that make a query a question or a request for Homer: his row
/// then comes first.
const HOMER_FIRST: [&str; 16] = [
    "remind me",
    "note ",
    "note:",
    "todo ",
    "todo:",
    "how ",
    "what ",
    "why ",
    "when ",
    "where ",
    "who ",
    "can you",
    "please ",
    "help me",
    "explain ",
    "summarize ",
];

/// Folders (as a path's start) shown first in their own folder listing.
fn file_icon(path: &Path, dir: bool) -> Icon {
    if dir {
        return Icon::Name("folder".into());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "bmp" | "tif" | "tiff" => "image-x-generic",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" => "audio-x-generic",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "video-x-generic",
        "pdf" => "application-pdf",
        "doc" | "docx" | "odt" | "rtf" => "x-office-document",
        "xls" | "xlsx" | "ods" | "csv" => "x-office-spreadsheet",
        "ppt" | "pptx" | "odp" => "x-office-presentation",
        "zip" | "tar" | "gz" | "xz" | "zst" | "7z" | "rar" | "deb" => "package-x-generic",
        "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "nix" | "sh" | "toml" | "json" | "html"
        | "css" => "text-x-script",
        "app" => "application-x-executable",
        _ => "text-x-generic",
    };
    Icon::Name(name.into())
}

/// The app a window belongs to: its desktop entry's id is the window's
/// app id, or ends with it (`firefox` → `org.mozilla.firefox`), or the
/// other way round.
fn app_for<'a>(apps: &'a [App], app_id: &str) -> Option<&'a App> {
    let want = app_id.trim_end_matches(".desktop");
    if want.is_empty() {
        return None;
    }
    apps.iter()
        .find(|a| a.id.eq_ignore_ascii_case(want))
        .or_else(|| apps.iter().find(|a| last_part(&a.id).eq_ignore_ascii_case(want)))
        .or_else(|| apps.iter().find(|a| a.id.eq_ignore_ascii_case(last_part(want))))
}

/// `org.mozilla.firefox` → `firefox`.
fn last_part(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

/// A result for an open window: its title, its app's name and icon.
pub fn window_hit(w: &Window, app: Option<&App>) -> Hit {
    let name = app.map_or_else(|| w.app_id.clone(), |a| a.name.clone());
    let title = if w.title.trim().is_empty() { name.clone() } else { w.title.clone() };
    let icon = app.and_then(|a| a.icon.clone()).unwrap_or_else(|| {
        Icon::Name(if w.app_id.is_empty() { "application-x-executable".into() } else { w.app_id.clone() })
    });
    let mut h = Hit::new(Kind::Window, title, Action::FocusWindow { identifier: w.identifier.clone() }).subtitle(name);
    h.icon = Some(icon);
    h
}

/// A result for a file or folder.
pub fn file_hit(path: &Path, dir: bool) -> Hit {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let parent = path.parent().map(crate::tilde).unwrap_or_default();
    Hit {
        kind: if dir { Kind::Folder } else { Kind::File },
        title: name,
        subtitle: parent,
        icon: Some(file_icon(path, dir)),
        action: Action::Open(path.to_owned()),
        key: format!("file:{}", path.display()),
        score: 0,
        clip_id: 0,
        pinned: false,
        at: 0,
        source: String::new(),
    }
}

struct Fuzzy {
    matcher: Matcher,
    pattern: Pattern,
    buf: Vec<char>,
}

impl Fuzzy {
    fn new(q: &str) -> Self {
        Fuzzy {
            matcher: Matcher::new(Config::DEFAULT),
            pattern: Pattern::new(q, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy),
            buf: Vec::new(),
        }
    }
    fn score(&mut self, text: &str) -> Option<i64> {
        self.pattern.score(Utf32Str::new(text, &mut self.buf), &mut self.matcher).map(i64::from)
    }
    /// A name: matched closely, or a word in it starts with the query. A
    /// scattered match ("days until" in "LibreOffice Draw") is noise.
    fn close(&mut self, text: &str, lower: &str) -> Option<i64> {
        let s = self.score(text)?;
        let w = word_start(text, lower);
        let close = 12 * lower.chars().filter(|c| !c.is_whitespace()).count() as i64;
        (s >= close || w > 0).then_some(s * 3 + w)
    }
    /// A description or keywords: only when a word starts with the query.
    fn word(&mut self, text: &str, lower: &str) -> Option<i64> {
        let w = word_start(text, lower);
        if w < 250 {
            return None;
        }
        self.score(text).map(|s| s + w / 2)
    }
}

/// Extra points when the query starts the name or one of its words
/// ("term" → Terminal, "edit" → Text Editor).
fn word_start(name: &str, q: &str) -> i64 {
    let n = name.to_lowercase();
    if n.starts_with(q) {
        400
    } else if n.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        250
    } else if n.contains(q) {
        80
    } else {
        0
    }
}

impl Engine {
    pub fn new(files: FileIndex, apps: Vec<App>, offers: Vec<Offer>, usage: Usage, recent: Option<PathBuf>) -> Self {
        Engine {
            files: RwLock::new(files),
            apps: RwLock::new(apps),
            offers: RwLock::new(offers),
            installed: RwLock::new(HashSet::new()),
            usage: Mutex::new(usage),
            recent,
            indexing: std::sync::atomic::AtomicBool::new(false),
            windows: RwLock::new(Vec::new()),
            show_windows: std::sync::atomic::AtomicBool::new(true),
            extensions: RwLock::new(Vec::new()),
        }
    }

    /// The extension a query is for: it starts with an active extension's
    /// keyword and a space. Returns the extension and what follows (the
    /// only thing it is ever told).
    pub fn route(&self, query: &str) -> Option<(Extension, String)> {
        let (keyword, rest) = query.trim_start().split_once(' ')?;
        if iliada_bundle::search::RESERVED.contains(&keyword) {
            return None;
        }
        let ext = self
            .extensions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|e| e.active() && e.keyword == keyword)?
            .clone();
        Some((ext, rest.trim().to_owned()))
    }

    /// The results for `query`, best first.
    pub fn search(&self, query: &str, homer: bool, limit: usize) -> Vec<Hit> {
        let limit = limit.clamp(1, 50);
        // `w ` lists open windows only.
        if let Some(words) = query.trim_start().strip_prefix("w ") {
            return self.windows_only(words.trim(), limit);
        }
        // `<keyword> `: an extension answers (`Request::ExtensionQuery`).
        if self.route(query).is_some() {
            return Vec::new();
        }
        let q = query.trim();
        if q.is_empty() {
            return self.empty(limit);
        }
        if let Some(cmd) = q.strip_prefix('>') {
            return self.command(cmd.trim());
        }
        if let Some(words) = q.strip_prefix('?') {
            let mut v = vec![web_hit(words.trim())];
            if homer && !words.trim().is_empty() {
                v.push(homer_hit(words.trim()));
            }
            return v;
        }
        if q.starts_with('/') || q.starts_with("~/") || q == "~" {
            return browse(q, limit);
        }
        let lower = q.to_lowercase();
        let mut hits = Vec::new();
        if let Some((shown, copy)) = calc::answer(q) {
            hits.push(
                Hit::new(Kind::Answer, shown, Action::Copy(copy)).subtitle("Enter copies the answer").score(100_000),
            );
        }
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let mut fz = Fuzzy::new(q);

        let apps = self.apps.read().unwrap_or_else(|e| e.into_inner());
        for app in apps.iter() {
            let name = fz.close(&app.name, &lower);
            let other = [&app.generic, &app.keywords, &app.comment]
                .iter()
                .filter(|t| !t.is_empty())
                .filter_map(|t| fz.word(t, &lower))
                .max();
            let best = match (name, other) {
                (Some(n), Some(o)) => n.max(o),
                (Some(n), None) => n,
                (None, Some(o)) => o,
                _ => continue,
            };
            let mut h = app.hit();
            h.score = 1200 + best + usage.boost(&h.key);
            hits.push(h);
        }
        if self.show_windows.load(std::sync::atomic::Ordering::Relaxed) {
            let mut found = Vec::new();
            for w in self.windows.read().unwrap_or_else(|e| e.into_inner()).iter() {
                if !listable(&w.app_id, &w.title) {
                    continue;
                }
                let app = app_for(&apps, &w.app_id);
                let mut h = window_hit(w, app);
                let Some(best) = fz.close(&w.title, &lower).max(fz.close(&h.subtitle, &lower)) else { continue };
                let boost = app.map_or(0, |a| usage.boost(&format!("app:{}", a.id)));
                h.score = 1200 + WINDOW_LEAD + best + boost;
                found.push(h);
            }
            found.sort_by_key(|h| std::cmp::Reverse(h.score));
            found.truncate(WINDOWS);
            hits.extend(found);
        }
        drop(apps);
        for page in PAGES {
            if let Some(s) = fz.close(page.title, &lower).or_else(|| fz.word(page.words, &lower)) {
                let mut h = page.hit();
                h.score = 1000 + s + usage.boost(&h.key);
                hits.push(h);
            }
        }
        for a in ACTIONS {
            if let Some(s) = fz.close(a.title, &lower).or_else(|| fz.word(a.words, &lower)) {
                let mut h = a.hit();
                h.score = 950 + s + usage.boost(&h.key);
                hits.push(h);
            }
        }
        if q.chars().count() >= 2 {
            let files = self.files.read().unwrap_or_else(|e| e.into_inner());
            for f in files.search(q, FILES) {
                let path = files.path(f.entry);
                let mut h = file_hit(&path, files.is_dir(f.entry));
                h.score = 800 + f.score + usage.boost(&h.key);
                hits.push(h);
            }
            // agora: only good matches (the name starts with, or has a
            // word starting with, what is typed).
            let installed = self.installed.read().unwrap_or_else(|e| e.into_inner());
            for o in self.offers.read().unwrap_or_else(|e| e.into_inner()).iter() {
                if installed.contains(&o.name.to_lowercase()) {
                    continue;
                }
                let w = word_start(&o.name, &lower);
                if w >= 250
                    && let Some(s) = fz.score(&o.name)
                {
                    hits.push(o.hit().score(700 + s + w / 2));
                }
            }
        }
        // Things chosen before that no source has now (a file outside the
        // home, a removed index entry that came back).
        for h in usage.top(50) {
            if let Some(s) = fz.close(&h.title, &lower) {
                let b = usage.boost(&h.key);
                hits.push(Hit { score: 900 + s + b, ..h });
            }
        }
        drop(usage);

        hits.sort_by_key(|h| std::cmp::Reverse(h.score));
        let mut seen = HashSet::new();
        hits.retain(|h| h.key.is_empty() || seen.insert(h.key.clone()));
        hits.truncate(limit);

        if homer {
            let first = HOMER_FIRST.iter().any(|w| lower.starts_with(w)) || q.ends_with('?');
            // An answer or an agora app is a good find too.
            let weak = hits.first().is_none_or(|h| h.score < 1300 && !matches!(h.kind, Kind::Agora | Kind::Answer));
            if first {
                hits.insert(0, homer_hit(q));
                hits.truncate(limit);
            } else if weak {
                if hits.len() >= limit {
                    hits.pop();
                }
                hits.push(homer_hit(q));
            }
        }
        if hits.len() < limit && q.chars().count() >= 2 {
            hits.push(web_hit(q));
        }
        hits
    }

    /// `w `: open windows only. With nothing after it, every open window,
    /// the most recently focused first (then in the compositor's order).
    fn windows_only(&self, q: &str, limit: usize) -> Vec<Hit> {
        if !self.show_windows.load(std::sync::atomic::Ordering::Relaxed) {
            return Vec::new();
        }
        let apps = self.apps.read().unwrap_or_else(|e| e.into_inner());
        let windows = self.windows.read().unwrap_or_else(|e| e.into_inner());
        let open = windows.iter().filter(|w| listable(&w.app_id, &w.title));
        if q.is_empty() {
            let mut v: Vec<&Window> = open.collect();
            v.sort_by_key(|w| std::cmp::Reverse(w.focused));
            return v.into_iter().take(limit).map(|w| window_hit(w, app_for(&apps, &w.app_id))).collect();
        }
        let lower = q.to_lowercase();
        let mut fz = Fuzzy::new(q);
        let mut hits: Vec<Hit> = open
            .filter_map(|w| {
                let mut h = window_hit(w, app_for(&apps, &w.app_id));
                h.score = fz.close(&w.title, &lower).max(fz.close(&h.subtitle, &lower))?;
                Some(h)
            })
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.score));
        hits.truncate(limit);
        hits
    }

    /// Nothing typed: what you use most, then recent files.
    fn empty(&self, limit: usize) -> Vec<Hit> {
        let mut hits = self.usage.lock().unwrap_or_else(|e| e.into_inner()).top(limit.min(8));
        hits.retain(|h| h.path().is_none_or(Path::exists));
        let mut seen: HashSet<String> = hits.iter().map(|h| h.key.clone()).collect();
        if let Some(r) = &self.recent {
            for p in crate::recent::load(r, limit) {
                let h = file_hit(&p, p.is_dir())
                    .subtitle(format!("Recent · {}", p.parent().map(crate::tilde).unwrap_or_default()));
                if hits.len() < limit && seen.insert(h.key.clone()) {
                    hits.push(h);
                }
            }
        }
        hits
    }

    fn command(&self, cmd: &str) -> Vec<Hit> {
        if cmd.is_empty() {
            return vec![
                Hit::new(
                    Kind::Command,
                    "Type a command to run in Terminal",
                    Action::Run(actions::terminal_command("")),
                )
                .icon_name("utilities-terminal"),
            ];
        }
        vec![
            Hit::new(Kind::Command, format!("Run “{cmd}”"), Action::Run(actions::terminal_command(cmd)))
                .subtitle("In Terminal")
                .icon_name("utilities-terminal"),
        ]
    }

    /// Apps that open `path` (by its file type), its default first.
    pub fn open_with(&self, path: &Path) -> Vec<Hit> {
        let mime = mime_type(path);
        let wild = mime.as_deref().and_then(|m| m.split_once('/')).map(|(t, _)| format!("{t}/*"));
        let apps = self.apps.read().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<Hit> = apps
            .iter()
            .filter(|a| {
                a.mime.iter().any(|m| Some(m.as_str()) == mime.as_deref() || Some(m.as_str()) == wild.as_deref())
            })
            .map(|a| {
                let mut argv = a.exec.clone();
                argv.push(path.display().to_string());
                Hit { action: Action::Run(argv), subtitle: "Open With".into(), ..a.hit() }
            })
            .collect();
        v.sort_by(|a, b| a.title.cmp(&b.title));
        v
    }

    /// Choosing `hit` is remembered.
    pub fn used(&self, hit: &Hit) {
        let mut u = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        u.used(hit);
        let _ = u.save();
    }
}

/// A file's type, as the desktop knows it (`xdg-mime`), from its name when
/// that fails.
fn mime_type(path: &Path) -> Option<String> {
    if let Ok(out) = std::process::Command::new("xdg-mime").args(["query", "filetype"]).arg(path).output()
        && out.status.success()
    {
        let m = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !m.is_empty() {
            return Some(m);
        }
    }
    if path.is_dir() {
        return Some("inode/directory".into());
    }
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(
        match ext.as_str() {
            "txt" | "md" => "text/plain",
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "html" => "text/html",
            _ => return None,
        }
        .into(),
    )
}

pub fn homer_hit(q: &str) -> Hit {
    Hit::new(Kind::Homer, format!("Ask Homer “{q}”"), Action::AskHomer(q.to_owned()))
        .subtitle("Homer answers, or does it for you")
        .icon_name("org.iliada.Assistant")
}

pub fn web_hit(q: &str) -> Hit {
    let mut url = String::from("https://duckduckgo.com/?q=");
    for b in q.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => url.push(b as char),
            b' ' => url.push('+'),
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    let title = if q.is_empty() { "Search the web".to_owned() } else { format!("Search the web for “{q}”") };
    Hit::new(Kind::Web, title, Action::Url(url)).subtitle("DuckDuckGo").icon_name("web-browser").score(-1)
}

/// `/` and `~/`: what is in a folder, narrowed by what follows the last
/// slash. Hidden entries only when that starts with a dot.
fn browse(q: &str, limit: usize) -> Vec<Hit> {
    let expanded = match q.strip_prefix('~') {
        Some(rest) => match crate::home() {
            Some(h) => format!("{}{rest}", h.display()),
            None => return Vec::new(),
        },
        None => q.to_owned(),
    };
    let (dir, part) = match expanded.rfind('/') {
        Some(i) => (&expanded[..=i], &expanded[i + 1..]),
        None => return Vec::new(),
    };
    let part = part.to_lowercase();
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(bool, bool, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let l = name.to_lowercase();
            if name.starts_with('.') && !part.starts_with('.') {
                return None;
            }
            if !l.contains(&part) {
                return None;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            Some((!l.starts_with(&part), !is_dir, l, e.path()))
        })
        .collect();
    v.sort();
    let mut hits: Vec<Hit> = Vec::new();
    if part.is_empty() {
        hits.push(file_hit(Path::new(dir), true).subtitle("This folder"));
    }
    hits.extend(v.into_iter().take(limit).map(|(_, file, _, p)| file_hit(&p, !file)));
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::parse_entry;

    fn engine() -> Engine {
        let mut files = FileIndex::new("/home/a");
        for (p, d) in [("Documents", true), ("Documents/terminal-notes.md", false), ("firefox-download.zip", false)] {
            files.insert(&Path::new("/home/a").join(p), d);
        }
        let apps = [
            "Terminal|cosmic-term|Use the command line",
            "Files|cosmic-files|Browse files",
            "Firefox|firefox|Web Browser",
        ]
        .iter()
        .map(|s| {
            let p: Vec<_> = s.split('|').collect();
            parse_entry(
                p[1],
                &format!("[Desktop Entry]\nType=Application\nName={}\nExec={}\nComment={}\n", p[0], p[1], p[2]),
            )
            .unwrap()
        })
        .collect();
        let offers = vec![Offer {
            id: "org.gimp.GIMP".into(),
            name: "GIMP".into(),
            summary: "Photos".into(),
            category: String::new(),
        }];
        let dir = tempfile::tempdir().unwrap();
        let usage = Usage::load(&dir.path().join("u.json"));
        Engine::new(files, apps, offers, usage, None)
    }

    fn window(title: &str, app_id: &str, identifier: &str, focused: u64) -> Window {
        Window { title: title.into(), app_id: app_id.into(), identifier: identifier.into(), focused }
    }

    fn with_windows() -> Engine {
        let e = engine();
        *e.windows.write().unwrap() = vec![
            window("Inbox (3) - Mail", "firefox", "w1", 2),
            window("notes.md", "cosmic-term", "w2", 0),
            window("Allow Firefox to use the camera?", "org.iliada.Prompt", "w3", 3),
            window("Rust docs", "org.mozilla.firefox", "w4", 5),
        ];
        e
    }

    fn titles(e: &Engine, q: &str, homer: bool) -> Vec<String> {
        e.search(q, homer, 8).into_iter().map(|h| h.title).collect()
    }

    #[test]
    fn apps_before_files() {
        let e = engine();
        let t = titles(&e, "term", false);
        assert_eq!(t[0], "Terminal");
        assert!(t.contains(&"terminal-notes.md".to_owned()));
        assert_eq!(titles(&e, "fire", false)[0], "Firefox");
        assert_eq!(titles(&e, "gimp", false)[0], "GIMP");
    }

    #[test]
    fn answers_and_settings() {
        let e = engine();
        assert_eq!(titles(&e, "2+3", false)[0], "= 5");
        assert_eq!(titles(&e, "wifi", false)[0], "Wi-Fi");
        assert!(titles(&e, "dark", false).contains(&"Dark Mode".to_owned()));
    }

    #[test]
    fn homer_row() {
        let e = engine();
        // Nothing good found: Homer is offered (when he is on).
        assert!(titles(&e, "qwxz", true).iter().any(|t| t.starts_with("Ask Homer")));
        assert!(!titles(&e, "qwxz", false).iter().any(|t| t.starts_with("Ask Homer")));
        // A good match: no Homer row.
        assert!(!titles(&e, "terminal", true).iter().any(|t| t.starts_with("Ask Homer")));
        // A question or a reminder: Homer first.
        assert!(titles(&e, "remind me to call mum at 6", true)[0].starts_with("Ask Homer"));
        assert!(titles(&e, "is it raining in Nicosia?", true)[0].starts_with("Ask Homer"));
    }

    #[test]
    fn prefixes() {
        let e = engine();
        let h = e.search("> ls -l", false, 8);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].kind, Kind::Command);
        let h = e.search("?rust iced", false, 8);
        assert_eq!(h[0].action, Action::Url("https://duckduckgo.com/?q=rust+iced".into()));
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("Alpha")).unwrap();
        std::fs::write(d.path().join("alpine.txt"), "").unwrap();
        std::fs::write(d.path().join(".hidden"), "").unwrap();
        let h = e.search(&format!("{}/al", d.path().display()), false, 8);
        assert_eq!(h.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), vec!["Alpha", "alpine.txt"]);
        let h = e.search(&format!("{}/", d.path().display()), false, 8);
        assert_eq!(h.len(), 3);
        // `w `: open windows only, the most recently focused first; never a
        // protected one.
        let e = with_windows();
        let h = e.search("w ", false, 8);
        assert!(h.iter().all(|h| h.kind == Kind::Window));
        assert_eq!(
            h.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
            ["Rust docs", "Inbox (3) - Mail", "notes.md"]
        );
        let h = e.search("w term", false, 8);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].action, Action::FocusWindow { identifier: "w2".into() });
        assert!(e.search("w camera", false, 8).is_empty());
        // "w" alone is an ordinary search.
        assert!(e.search("w", false, 8).iter().any(|h| h.kind != Kind::Window));
    }

    fn extension(id: &str, keyword: &str, enabled: bool, clashes_with: Option<&str>) -> Extension {
        Extension {
            id: id.into(),
            title: id.to_uppercase(),
            placeholder: None,
            keyword: keyword.into(),
            declared_keyword: keyword.into(),
            enabled,
            network: false,
            clashes_with: clashes_with.map(str::to_owned),
            bundle: format!("/Applications/{id}.app").into(),
        }
    }

    #[test]
    fn keywords_route_to_extensions() {
        let e = engine();
        *e.extensions.write().unwrap() = vec![
            extension("color", "color", true, None),
            extension("gh", "gh", true, None),
            extension("hub", "gh", true, Some("GH")),
            extension("off", "tr", false, None),
        ];
        let route = |q: &str| e.route(q).map(|(x, rest)| (x.id, rest));
        assert_eq!(route("color #ff0000"), Some(("color".into(), "#ff0000".into())));
        assert_eq!(route("  gh  ripgrep tools "), Some(("gh".into(), "ripgrep tools".into())));
        assert_eq!(route("gh "), Some(("gh".into(), String::new())));
        // Only the keyword and a space: "gh" alone, "ghost" and "GH x" are
        // ordinary searches.
        assert_eq!(route("gh"), None);
        assert_eq!(route("ghost town"), None);
        assert_eq!(route("GH x"), None);
        // Turned off: an ordinary search.
        assert_eq!(route("tr hello"), None);
        assert!(!e.search("tr hello", false, 8).is_empty());
        // An extension query is never searched anywhere else.
        assert!(e.search("color red", false, 8).is_empty());
        // Search's own prefixes win, whatever an extension claims.
        *e.extensions.write().unwrap() = vec![extension("bad", "w", true, None)];
        assert_eq!(route("w firefox"), None);
    }

    #[test]
    fn open_windows_come_before_their_app() {
        let e = with_windows();
        let h = e.search("firefox", false, 8);
        assert_eq!(h[0].kind, Kind::Window);
        assert_eq!(h[0].subtitle, "Firefox");
        let app = h.iter().position(|h| h.kind == Kind::App && h.title == "Firefox").unwrap();
        assert!(h[..app].iter().all(|h| h.kind == Kind::Window));
        assert_eq!(app, 2, "both Firefox windows, then the app");
        // By title.
        assert_eq!(e.search("inbox", false, 8)[0].title, "Inbox (3) - Mail");
        // The prompt never shows, even when its title matches.
        assert!(!e.search("camera", false, 8).iter().any(|h| h.kind == Kind::Window));
        // Turned off in Settings: no windows at all.
        e.show_windows.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(!e.search("firefox", false, 8).iter().any(|h| h.kind == Kind::Window));
        assert!(e.search("w ", false, 8).is_empty());
    }

    #[test]
    fn usage_lifts_and_fills_the_empty_search() {
        let e = engine();
        let h = e.search("files", false, 8).into_iter().find(|h| h.title == "Files").unwrap();
        let before = h.score;
        let dir = tempfile::tempdir().unwrap();
        *e.usage.lock().unwrap() = Usage::load(&dir.path().join("u.json"));
        e.used(&h);
        let after = e.search("files", false, 8).into_iter().find(|h| h.title == "Files").unwrap().score;
        assert!(after > before);
        assert_eq!(e.search("", false, 8)[0].title, "Files");
    }
}
