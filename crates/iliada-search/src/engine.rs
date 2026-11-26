//! Putting the sources together: one ranked list per keystroke.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::agora::Offer;
use crate::apps::App;
use crate::files::FileIndex;
use crate::settings::{ACTIONS, PAGES};
use crate::usage::Usage;
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
}

/// Rows of each kind, at most.
const FILES: usize = 12;

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
        }
    }

    /// The results for `query`, best first.
    pub fn search(&self, query: &str, homer: bool, limit: usize) -> Vec<Hit> {
        let limit = limit.clamp(1, 50);
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

        for app in self.apps.read().unwrap_or_else(|e| e.into_inner()).iter() {
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
