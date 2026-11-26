//! Saved conversations: each one is a file in
//! `~/.local/share/iliada/conversations/<id>.json`, readable only by its
//! owner. It keeps what was shown (the transcript) and what the model needs
//! to continue (the messages).
//!
//! Each conversation knows where it was started ([`Source`]: the Homer app,
//! the quick panel, search, or a Terminal folder) and what it is ([`Kind`]):
//! a **chat** is kept, a **task** ("Switch to dark mode": one question, an
//! action, done) is kept for a day and then removed on its own. The
//! [`ConversationStore`] is the one way in, for every frontend; history can
//! be turned off ([`history_enabled`]), and then nothing new is written.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// What the person asked.
    You,
    /// The assistant's answer.
    Assistant,
    /// An action and its outcome ("✓ Install Firefox").
    Note,
    Error,
}

/// One line of the transcript, as shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub kind: LineKind,
    pub text: String,
}

impl Line {
    pub fn new(kind: LineKind, text: impl Into<String>) -> Self {
        Line { kind, text: text.into() }
    }
}

/// Where a conversation was started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum Source {
    /// The Homer app (and conversations saved before sources existed).
    #[default]
    App,
    /// The quick panel (Alt+Space).
    Panel,
    /// Search (Super+O, or a search that fell through to Homer).
    Search,
    /// Homer mode in the Terminal, working in this folder.
    Terminal { cwd: String },
}

impl Source {
    /// The folder of a Terminal conversation.
    pub fn cwd(&self) -> Option<&str> {
        match self {
            Source::Terminal { cwd } => Some(cwd),
            _ => None,
        }
    }
}

/// What a conversation is: kept (a chat), or a quick task kept for a day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Chat,
    Task,
}

/// How long a task is kept (seconds).
pub const TASK_TTL: u64 = 24 * 60 * 60;

/// The longest final answer a task has: a task ends in "Done, dark mode is
/// on.", not an explanation.
const TASK_ANSWER: usize = 280;

/// Chat or task, from the transcript alone (no model call): one question
/// that ended in an action done, a short answer, no error and no follow-up
/// is a task. Anything else, or anything continued, is a chat.
pub fn classify(c: &Conversation) -> Kind {
    let asked = c.lines.iter().filter(|l| l.kind == LineKind::You).count();
    let acted = c.lines.iter().any(|l| l.kind == LineKind::Note && l.text.starts_with('✓'));
    let trouble = c.lines.iter().any(|l| {
        l.kind == LineKind::Error || (l.kind == LineKind::Note && !l.text.starts_with('✓') && !l.text.ends_with('…'))
    });
    let answer = c.lines.iter().rev().find(|l| l.kind == LineKind::Assistant).map_or(0, |l| l.text.chars().count());
    if asked == 1 && acted && !trouble && answer <= TASK_ANSWER { Kind::Task } else { Kind::Chat }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub updated: u64,
    /// The model it was last continued with.
    pub model: String,
    /// What it has cost (US dollars, when known).
    pub spent: f64,
    pub lines: Vec<Line>,
    /// The model's view of the conversation (provider format).
    pub messages: Vec<Value>,
    /// What happened outside the conversation since the last answer (an
    /// action undone from Activity): told to the model with the next question.
    #[serde(default)]
    pub pending: Vec<String>,
    #[serde(default)]
    pub source: Source,
    /// Set on every save ([`classify`]).
    #[serde(default)]
    pub kind: Kind,
}

/// A conversation in the list (without its contents).
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub source: Source,
    pub kind: Kind,
    /// Its words, lowercased (for [`ConversationStore::search`]).
    text: String,
}

impl Summary {
    /// Every word of the query is in its title or transcript.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        q.split_whitespace().all(|w| self.text.contains(w))
    }
}

/// The list as the Homer app shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sections {
    /// Kept conversations from the app, the panel and search, newest first.
    pub chats: Vec<Summary>,
    /// Terminal conversations by folder: the folder with the newest one first.
    pub terminal: Vec<(String, Vec<Summary>)>,
    /// Recent quick tasks (from anywhere), newest first.
    pub tasks: Vec<Summary>,
}

/// Splits a list (newest first) into its sections.
pub fn sections(list: Vec<Summary>) -> Sections {
    let mut s = Sections::default();
    for c in list {
        match (&c.kind, c.source.cwd()) {
            (Kind::Task, _) => s.tasks.push(c),
            (Kind::Chat, Some(cwd)) => match s.terminal.iter_mut().find(|(d, _)| d == cwd) {
                Some((_, list)) => list.push(c),
                None => s.terminal.push((cwd.to_owned(), vec![c])),
            },
            (Kind::Chat, None) => s.chats.push(c),
        }
    }
    s
}

/// A folder as people read it: `~/code/iliada`.
pub fn short_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match path.strip_prefix(home.trim_end_matches('/')) {
        Some("") if !home.is_empty() => "~".into(),
        Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn dir() -> Option<PathBuf> {
    crate::data_path("conversations")
}

/// `~/.config/iliada/homer.toml`: Homer's own settings for this person.
fn prefs_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    Some(config.join("iliada").join("homer.toml"))
}

/// Whether conversations are kept (Settings → Homer → History; on unless
/// turned off).
pub fn history_enabled() -> bool {
    prefs_path().is_none_or(|p| keeps_history(&std::fs::read_to_string(p).unwrap_or_default()))
}

fn keeps_history(prefs: &str) -> bool {
    toml::from_str::<toml::Table>(prefs).ok().and_then(|t| t.get("keep_history")?.as_bool()).unwrap_or(true)
}

/// Turns history on or off (what is kept stays until it is cleared).
pub fn set_history_enabled(on: bool) -> Result<()> {
    let p = prefs_path().context("HOME is not set")?;
    let mut t: toml::Table = std::fs::read_to_string(&p).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
    t.insert("keep_history".into(), toml::Value::Boolean(on));
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, toml::to_string(&t)?)?;
    Ok(())
}

/// Saved conversations: where they are, and whether new ones are kept.
#[derive(Debug, Clone)]
pub struct ConversationStore {
    dir: Option<PathBuf>,
    keep: bool,
}

impl ConversationStore {
    /// This person's conversations, kept if history is on.
    pub fn open() -> Self {
        ConversationStore { dir: dir(), keep: history_enabled() }
    }

    /// Conversations in `dir` (tests, other frontends).
    pub fn at(dir: impl Into<PathBuf>, keep: bool) -> Self {
        ConversationStore { dir: Some(dir.into()), keep }
    }

    /// Whether new conversations are kept.
    pub fn keeps(&self) -> bool {
        self.keep
    }

    fn dir(&self) -> Result<&std::path::Path> {
        self.dir.as_deref().context("HOME is not set")
    }

    /// Saves it (titled and classified first); nothing when history is off.
    pub fn save(&self, c: &mut Conversation) -> Result<()> {
        if !self.keep {
            return Ok(());
        }
        c.save_in(self.dir()?)
    }

    pub fn load(&self, id: &str) -> Result<Conversation> {
        Conversation::load_from(self.dir()?, id)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        anyhow::ensure!(valid_id(id), "bad conversation id");
        match std::fs::remove_file(self.dir()?.join(format!("{id}.json"))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Deletes every conversation; how many there were.
    pub fn clear(&self) -> Result<usize> {
        let Ok(entries) = std::fs::read_dir(self.dir()?) else { return Ok(0) };
        let mut n = 0;
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                std::fs::remove_file(e.path())?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Saved conversations, newest first; tasks older than a day are
    /// removed on the way.
    pub fn list(&self) -> Vec<Summary> {
        let Ok(dir) = self.dir() else { return Vec::new() };
        let now = now();
        let (keep, old): (Vec<_>, Vec<_>) =
            list_in(dir).into_iter().partition(|s| s.kind == Kind::Chat || now.saturating_sub(s.updated) < TASK_TTL);
        for s in old {
            let _ = self.delete(&s.id);
        }
        keep
    }

    /// Conversations with every word of `query` in them, newest first.
    pub fn search(&self, query: &str) -> Vec<Summary> {
        self.list().into_iter().filter(|s| s.matches(query)).collect()
    }

    /// The newest conversation started in this Terminal folder, if it was
    /// continued in the last `within` seconds.
    pub fn latest_in(&self, cwd: &str, within: u64) -> Option<Conversation> {
        let now = now();
        let s = self.list().into_iter().find(|s| s.source.cwd() == Some(cwd))?;
        (now.saturating_sub(s.updated) < within).then(|| self.load(&s.id).ok()).flatten()
    }
}

/// Ids are file names: only these characters.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A title from the first question: its first line, at most ~50 characters,
/// cut at a word.
pub fn title_from(question: &str) -> String {
    let first = question.lines().next().unwrap_or("").trim();
    if first.chars().count() <= 50 {
        return first.to_owned();
    }
    let cut: String = first.chars().take(50).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).filter(|a| a.len() > 20).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches([',', '.', ';', ':']))
}

impl Conversation {
    /// A new conversation started here.
    pub fn from_source(model: &str, source: Source) -> Self {
        Conversation { source, ..Self::new(model) }
    }

    pub fn new(model: &str) -> Self {
        let t = now();
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        Conversation {
            id: format!("{t}-{:08x}", nanos ^ std::process::id()),
            created: t,
            updated: t,
            model: model.to_owned(),
            ..Default::default()
        }
    }

    /// Saves it with [`ConversationStore::open`] (nothing when history is off).
    pub fn save(&mut self) -> Result<()> {
        ConversationStore::open().save(self)
    }

    fn save_in(&mut self, dir: &std::path::Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        anyhow::ensure!(valid_id(&self.id), "bad conversation id");
        if self.title.is_empty()
            && let Some(q) = self.lines.iter().find(|l| l.kind == LineKind::You)
        {
            self.title = title_from(&q.text);
        }
        self.kind = classify(self);
        self.updated = now();
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let tmp = dir.join(format!(".{}.tmp", self.id));
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(&serde_json::to_vec(self)?)?;
        f.sync_all()?;
        std::fs::rename(&tmp, dir.join(format!("{}.json", self.id)))?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self> {
        ConversationStore::open().load(id)
    }

    fn load_from(dir: &std::path::Path, id: &str) -> Result<Self> {
        anyhow::ensure!(valid_id(id), "bad conversation id");
        let bytes = std::fs::read(dir.join(format!("{id}.json"))).context("that conversation is gone")?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn delete(id: &str) -> Result<()> {
        ConversationStore::open().delete(id)
    }

    /// Saved conversations, newest first.
    pub fn list() -> Vec<Summary> {
        ConversationStore::open().list()
    }
}

fn list_in(dir: &std::path::Path) -> Vec<Summary> {
    let mut out: Vec<Summary> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Conversation>(&b).ok())
        .filter(|c| !c.lines.is_empty())
        .map(|c| {
            let mut text = c.title.to_lowercase();
            for l in &c.lines {
                text.push('\n');
                text.push_str(&l.text.to_lowercase());
            }
            Summary {
                id: c.id,
                title: if c.title.is_empty() { "Conversation".into() } else { c.title },
                updated: c.updated,
                source: c.source,
                kind: c.kind,
                text,
            }
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out
}

/// The conversation as plain text turns, for a model from another provider
/// (thinking blocks and tool calls are provider-specific).
pub fn portable(messages: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        let role = m["role"].as_str().unwrap_or("");
        let text = match &m["content"] {
            Value::String(s) => s.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if text.trim().is_empty() || !matches!(role, "user" | "assistant") {
            continue;
        }
        match out.last_mut() {
            Some(last) if last["role"] == role => {
                let joined = format!("{}\n{}", last["content"].as_str().unwrap_or(""), text);
                last["content"] = Value::String(joined);
            }
            _ => out.push(json!({ "role": role, "content": text })),
        }
    }
    // A conversation sent to a model starts with the person.
    while out.first().is_some_and(|m| m["role"] != "user") {
        out.remove(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_short() {
        assert_eq!(title_from("Install Firefox"), "Install Firefox");
        let t = title_from("Please install Firefox and VLC and then turn on the SSH server and dark mode");
        assert!(t.ends_with('…') && t.chars().count() <= 51, "{t}");
        assert_eq!(title_from("first line\nsecond"), "first line");
    }

    #[test]
    fn conversations_are_saved_listed_and_loaded() {
        let d = std::env::temp_dir().join(format!("iliada-conv-{}", std::process::id()));
        let mut c = Conversation::new("deepseek/deepseek-v4-flash");
        c.lines.push(Line::new(LineKind::You, "Install Firefox"));
        c.lines.push(Line::new(LineKind::Assistant, "Done."));
        c.messages.push(json!({ "role": "user", "content": "Install Firefox" }));
        c.save_in(&d).unwrap();
        let mut empty = Conversation::new("x");
        empty.id = "0-empty".into();
        empty.save_in(&d).unwrap();
        let list = list_in(&d);
        assert_eq!(list.len(), 1, "empty conversations are not listed");
        assert_eq!(list[0].title, "Install Firefox");
        let back = Conversation::load_from(&d, &c.id).unwrap();
        assert_eq!(back.lines, c.lines);
        assert!(Conversation::load_from(&d, "../etc/passwd").is_err());
        let mode = std::fs::metadata(d.join(format!("{}.json", c.id))).unwrap();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777, 0o600);
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn said(lines: &[(LineKind, &str)]) -> Conversation {
        let mut c = Conversation::new("m");
        c.lines = lines.iter().map(|(k, t)| Line::new(*k, *t)).collect();
        c
    }

    #[test]
    fn one_shot_actions_are_tasks_and_conversations_are_chats() {
        use LineKind::*;
        let task = said(&[(You, "Change to dark mode"), (Note, "✓ Switch to dark appearance"), (Assistant, "Done.")]);
        assert_eq!(classify(&task), Kind::Task);
        // A follow-up makes it a chat.
        let mut more = task.clone();
        more.lines.push(Line::new(You, "and the volume to 40%"));
        assert_eq!(classify(&more), Kind::Chat);
        // A question without an action, a failure, a refusal: chats.
        assert_eq!(classify(&said(&[(You, "Who was Hector?"), (Assistant, "A Trojan prince.")])), Kind::Chat);
        let failed = said(&[(You, "Install Firefox"), (Note, "✗ Install Firefox: offline"), (Assistant, "No.")]);
        assert_eq!(classify(&failed), Kind::Chat);
        let declined = said(&[(You, "Remove VLC"), (Note, "You didn't allow: Remove VLC"), (Assistant, "Ok.")]);
        assert_eq!(classify(&declined), Kind::Chat);
        // A long explanation after a command is worth keeping.
        let long = "x".repeat(TASK_ANSWER + 1);
        let explained = said(&[(You, "what's here?"), (Note, "✓ Run the command: ls"), (Assistant, &long)]);
        assert_eq!(classify(&explained), Kind::Chat);
    }

    #[test]
    fn the_store_sorts_searches_prunes_and_clears() {
        let d = std::env::temp_dir().join(format!("iliada-store-{}", std::process::id()));
        let store = ConversationStore::at(&d, true);
        let mut chat = said(&[(LineKind::You, "Tell me of Achilles"), (LineKind::Assistant, "Sing, O Muse.")]);
        store.save(&mut chat).unwrap();
        let cwd = "/home/p/code".to_owned();
        let mut here = Conversation::from_source("m", Source::Terminal { cwd: cwd.clone() });
        here.id = "1-here".into();
        here.lines = chat.lines.clone();
        here.lines[0].text = "Why does make fail?".into();
        store.save(&mut here).unwrap();
        let mut task = said(&[
            (LineKind::You, "Change to dark mode"),
            (LineKind::Note, "✓ Switch to dark appearance"),
            (LineKind::Assistant, "Done."),
        ]);
        task.id = "2-task".into();
        store.save(&mut task).unwrap();
        assert_eq!(task.kind, Kind::Task);

        let s = sections(store.list());
        assert_eq!(s.chats.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Tell me of Achilles"]);
        assert_eq!(s.terminal.len(), 1);
        assert_eq!(s.terminal[0].0, cwd);
        assert_eq!(s.terminal[0].1[0].title, "Why does make fail?");
        assert_eq!(s.tasks.len(), 1);

        assert_eq!(store.search("MUSE achilles").len(), 1);
        assert_eq!(store.search("make fail").len(), 1);
        assert!(store.search("odysseus").is_empty());

        assert_eq!(store.latest_in(&cwd, 3600).map(|c| c.id), Some("1-here".to_owned()));
        assert!(store.latest_in("/elsewhere", 3600).is_none());

        // A task from yesterday is gone; an old chat stays.
        let mut old = Conversation::load_from(&d, "2-task").unwrap();
        old.updated = now() - TASK_TTL - 1;
        std::fs::write(d.join("2-task.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(store.list().len(), 2);
        assert!(!d.join("2-task.json").exists());

        // History off: nothing new is written.
        let off = ConversationStore::at(&d, false);
        let mut quiet = said(&[(LineKind::You, "secret"), (LineKind::Assistant, "ok")]);
        off.save(&mut quiet).unwrap();
        assert_eq!(store.list().len(), 2);

        assert_eq!(store.clear().unwrap(), 2);
        assert!(store.list().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn old_conversations_are_app_chats() {
        let old = r#"{"id":"1-a","title":"t","created":1,"updated":1,"model":"m","spent":0,"lines":[],"messages":[]}"#;
        let c: Conversation = serde_json::from_str(old).unwrap();
        assert_eq!((c.source, c.kind), (Source::App, Kind::Chat));
        let t = Conversation::from_source("m", Source::Terminal { cwd: "/x".into() });
        let back: Conversation = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back.source.cwd(), Some("/x"));
    }

    #[test]
    fn history_is_on_unless_turned_off() {
        assert!(keeps_history(""));
        assert!(keeps_history("keep_history = true"));
        assert!(!keeps_history("keep_history = false\n"));
        assert!(keeps_history("not toml ["));
    }

    #[test]
    fn other_providers_get_plain_turns() {
        let messages = vec![
            json!({ "role": "user", "content": "Install VLC" }),
            json!({ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "…", "signature": "x" },
                { "type": "text", "text": "Installing." },
                { "type": "tool_use", "id": "t1", "name": "install_app", "input": { "app": "vlc" } } ] }),
            json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "VLC is installed." }] }),
        ];
        let p = portable(&messages);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], json!({ "role": "user", "content": "Install VLC" }));
        assert_eq!(p[1]["content"], "Installing.\nVLC is installed.");
    }
}
