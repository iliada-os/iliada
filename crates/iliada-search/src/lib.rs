//! Iliada's search: Spotlight for Iliada.
//!
//! `iliada-searchd` keeps everything it can find in memory and answers each
//! keystroke from there, without touching the disk:
//!
//! * **Apps**: desktop entries (COSMIC's apps, Iliada's, and every `.app`
//!   bundle's launcher) and the bundles in `/Applications` ([`apps`]).
//! * **Files and folders** in the home folder: every name in one block of
//!   memory, searched with a single substring scan, kept current through
//!   inotify ([`files`]).
//! * **Settings** pages and quick actions (dark mode, lock, Wi-Fi…)
//!   ([`settings`]).
//! * **agora** apps not installed yet ([`agora`]).
//! * **Answers**: arithmetic, units and dates ([`calc`]).
//! * **Recent** files and what you open most ([`usage`], [`recent`]).
//! * **Open windows**: choosing one switches to it ([`windows`]).
//!
//! * **Clipboard history** (Super+V): the last 200 copies, encrypted, skipping
//!   secrets and excluded apps ([`clipboard`]).
//!
//! Prefixes: `>` runs a command in Terminal, `?` searches the web, `w `
//! lists open windows, `/` and `~/` browse paths. When nothing good is found and Homer is on, the last
//! row asks Homer.
//!
//! The overlay (`iliada-search-overlay`, a libcosmic layer-shell surface)
//! sends a [`Request`] per keystroke over a Unix socket and draws the
//! [`Hit`]s ([`proto`]).

pub mod actions;
pub mod agora;
pub mod apps;
pub mod calc;
pub mod clipboard;
pub mod clipboard_watch;
pub mod engine;
pub mod files;
pub mod preview;
pub mod proto;
pub mod recent;
pub mod settings;
pub mod usage;
pub mod windows;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use engine::Engine;
pub use proto::{Request, Response};

/// What kind of thing a result is (its group, and the word shown with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    App,
    Folder,
    File,
    Setting,
    Action,
    Agora,
    Answer,
    Homer,
    Web,
    Command,
    Clipboard,
    Window,
}

impl Kind {
    /// The word shown on the right of a row.
    pub fn label(self) -> &'static str {
        match self {
            Kind::App => "Application",
            Kind::Folder => "Folder",
            Kind::File => "File",
            Kind::Setting => "Settings",
            Kind::Action => "Action",
            Kind::Agora => "agora",
            Kind::Answer => "Answer",
            Kind::Homer => "Homer",
            Kind::Web => "Web",
            Kind::Command => "Terminal",
            Kind::Clipboard => "Clipboard",
            Kind::Window => "Window",
        }
    }
}

/// A result's picture: a named icon from the theme, or an image file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Icon {
    Name(String),
    Path(PathBuf),
}

/// What choosing a result does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Run a program (an app's command line, already split).
    Run(Vec<String>),
    /// Open a file or folder with its default app (`.app` bundles start
    /// sandboxed).
    Open(PathBuf),
    /// Open a web address in the default browser.
    Url(String),
    /// Put this text on the clipboard.
    Copy(String),
    /// Hand this to Homer.
    AskHomer(String),
    /// Switch the desktop to dark (true) or light.
    Appearance(bool),
    /// Put this clipboard-history item on the clipboard; paste if `paste`.
    RestoreClipboard { id: u64, paste: bool },
    /// Pin or unpin a clipboard-history item.
    ClipboardPin { id: u64, pinned: bool },
    /// Delete one clipboard-history item.
    ClipboardDelete(u64),
    /// Delete every clipboard-history item.
    ClipboardClear,
    /// Move this file: the overlay asks for a folder.
    MoveTo(PathBuf),
    /// Open each of these files with its default app.
    OpenAll(Vec<PathBuf>),
    /// Switch to this open window (its ext-foreign-toplevel identifier):
    /// its workspace, restored if minimized.
    FocusWindow { identifier: String },
}

/// One row of results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub kind: Kind,
    pub title: String,
    /// The second line: a folder's place, an app's description…
    pub subtitle: String,
    pub icon: Option<Icon>,
    pub action: Action,
    /// What remembers this result was chosen (frecency): an app's id, a
    /// path, a Settings page. Empty: never remembered.
    #[serde(default)]
    pub key: String,
    /// Higher is better; only for ordering.
    #[serde(default)]
    pub score: i64,
    /// Clipboard history item id (0: not one).
    #[serde(default)]
    pub clip_id: u64,
    /// A pinned clipboard-history item stays when the list is full.
    #[serde(default)]
    pub pinned: bool,
    /// When a clipboard item was copied, as seconds since the epoch (0: none).
    #[serde(default)]
    pub at: u64,
    /// The app that copied a clipboard item, if known.
    #[serde(default)]
    pub source: String,
}

impl Hit {
    pub fn new(kind: Kind, title: impl Into<String>, action: Action) -> Self {
        Hit {
            kind,
            title: title.into(),
            subtitle: String::new(),
            icon: None,
            action,
            key: String::new(),
            score: 0,
            clip_id: 0,
            pinned: false,
            at: 0,
            source: String::new(),
        }
    }
    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = s.into();
        self
    }
    pub fn icon_name(mut self, name: impl Into<String>) -> Self {
        self.icon = Some(Icon::Name(name.into()));
        self
    }
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
    pub fn score(mut self, score: i64) -> Self {
        self.score = score;
        self
    }
    /// Fill in the clipboard-history fields for Super+V.
    pub fn clip(mut self, id: u64, pinned: bool, at: u64, source: impl Into<String>) -> Self {
        self.clip_id = id;
        self.pinned = pinned;
        self.at = at;
        self.source = source.into();
        self
    }
    /// The path this result is, when it is a file or folder.
    pub fn path(&self) -> Option<&std::path::Path> {
        match &self.action {
            Action::Open(p) => Some(p),
            _ => None,
        }
    }
}

/// `$XDG_RUNTIME_DIR`, where the sockets live.
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}

/// The home folder.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `~/…` for paths in the home folder (shorter, and what people read).
pub fn tilde(path: &std::path::Path) -> String {
    if let Some(home) = home()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
    }
    path.display().to_string()
}

/// `$XDG_STATE_HOME/iliada` (what the search learns about you).
pub fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/state")))
        .map(|d| d.join("iliada"))
}

/// `$XDG_CONFIG_HOME/iliada`.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")))
        .map(|d| d.join("iliada"))
}

/// `$XDG_CACHE_HOME/iliada`.
pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".cache")))
        .map(|d| d.join("iliada"))
}
