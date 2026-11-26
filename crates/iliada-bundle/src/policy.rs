//! What every app may use, in one file people and agents can read (ADR 0007).
//!
//! Two files, one format:
//!
//! * `~/.config/iliada/permissions.toml`: **this person's** answers, one
//!   table per app, with every capability spelled out and a generated risk
//!   summary. Iliada writes it (first-launch prompts, Settings → Privacy,
//!   `iliada permissions set`, Homer after you confirm) and reads it before
//!   every launch. You may edit it by hand; edits apply the next time the app
//!   opens.
//! * `/etc/iliada/permissions.toml`: the **computer's** limits, set by an
//!   administrator through the system service (git-backed, backed up,
//!   undoable). It can only take access away (`deny`, or `read-only` for a
//!   folder), for every person and for one app or all of them (`[all]`).
//!
//! What an app gets is the narrowest of: what it declares (`Permissions.toml`),
//! the computer's limits, and the person's answer. A file Iliada cannot
//! read never widens anything: a syntax error makes every decided
//! capability `deny` until it is fixed, and a bad value denies that one
//! capability. The file only says what is enforced: what Iliada cannot
//! restrict yet is written `unrestricted`, and what sandboxes never get is
//! written `deny` and cannot be changed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use iliada_types::{Capability, ClipboardAccess, FileScope, KnownDir};
use serde::{Serialize, Serializer};

use crate::{Bundle, IsolationTier};

/// The machine-readable schema `iliada permissions --json` follows
/// (docs/permissions-schema.md). Bumped only on incompatible changes.
pub const SCHEMA: &str = "iliada.permissions/1";
/// `version = 1` at the top of both files.
pub const FORMAT_VERSION: i64 = 1;
/// The computer's limits.
pub const SYSTEM_PATH: &str = "/etc/iliada/permissions.toml";

// ---------------------------------------------------------------------------
// Capabilities

/// One thing an app could use or do. Names are stable (they are the file's
/// keys and the JSON schema's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Network,
    Folder(KnownDir),
    Camera,
    Microphone,
    Sound,
    ScreenCapture,
    ClipboardRead,
    ClipboardWrite,
    Background,
    Notifications,
    Location,
    Autostart,
    Usb,
    Bluetooth,
    SystemChanges,
    OtherApps,
    /// Homer may read this app's windows and act in them.
    ControlledByHomer,
}

/// How Iliada handles a capability today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Apps that declare it get it only as the person decides; enforced by
    /// the sandbox. `default` applies until they answer.
    Decided { default: Value },
    /// Sandboxed apps are never given a way to do this (enforced).
    Never { why: &'static str },
    /// Iliada cannot restrict this yet: every sandboxed app can do it.
    NotRestrictedYet { why: &'static str },
}

impl Key {
    pub const ALL: [Key; 21] = [
        Key::Network,
        Key::Folder(KnownDir::Documents),
        Key::Folder(KnownDir::Downloads),
        Key::Folder(KnownDir::Music),
        Key::Folder(KnownDir::Pictures),
        Key::Folder(KnownDir::Videos),
        Key::Camera,
        Key::Microphone,
        Key::Sound,
        Key::ScreenCapture,
        Key::ClipboardRead,
        Key::ClipboardWrite,
        Key::Background,
        Key::Notifications,
        Key::Location,
        Key::Autostart,
        Key::Usb,
        Key::Bluetooth,
        Key::SystemChanges,
        Key::OtherApps,
        Key::ControlledByHomer,
    ];

    /// The key in the file and in JSON.
    pub fn name(self) -> &'static str {
        match self {
            Key::Network => "network",
            Key::Folder(KnownDir::Documents) => "files.documents",
            Key::Folder(KnownDir::Downloads) => "files.downloads",
            Key::Folder(KnownDir::Music) => "files.music",
            Key::Folder(KnownDir::Pictures) => "files.pictures",
            Key::Folder(KnownDir::Videos) => "files.videos",
            Key::Camera => "camera",
            Key::Microphone => "microphone",
            Key::Sound => "sound",
            Key::ScreenCapture => "screen_capture",
            Key::ClipboardRead => "clipboard.read",
            Key::ClipboardWrite => "clipboard.write",
            Key::Background => "background",
            Key::Notifications => "notifications",
            Key::Location => "location",
            Key::Autostart => "autostart",
            Key::Usb => "devices.usb",
            Key::Bluetooth => "devices.bluetooth",
            Key::SystemChanges => "system_changes",
            Key::OtherApps => "other_apps",
            Key::ControlledByHomer => "controlled_by_homer",
        }
    }

    pub fn parse(s: &str) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.name() == s)
    }

    /// How people read it ("Pictures folder", "Internet").
    /// The label inside a sentence ("the camera", "Bluetooth", "USB devices").
    pub fn phrase(self) -> String {
        let l = self.label();
        if matches!(self, Key::Usb | Key::Bluetooth | Key::Folder(_) | Key::ControlledByHomer) {
            l
        } else {
            l.to_lowercase()
        }
    }

    pub fn label(self) -> String {
        match self {
            Key::Network => "Internet".into(),
            Key::Folder(d) => format!("{} folder", d.name()),
            Key::Camera => "Camera".into(),
            Key::Microphone => "Microphone".into(),
            Key::Sound => "Sound".into(),
            Key::ScreenCapture => "Screen recording".into(),
            Key::ClipboardRead => "Reading the clipboard".into(),
            Key::ClipboardWrite => "Changing the clipboard".into(),
            Key::Background => "Running in the background".into(),
            Key::Notifications => "Notifications".into(),
            Key::Location => "Location".into(),
            Key::Autostart => "Starting at login".into(),
            Key::Usb => "USB devices".into(),
            Key::Bluetooth => "Bluetooth".into(),
            Key::SystemChanges => "Changing the system".into(),
            Key::OtherApps => "Talking to other apps".into(),
            Key::ControlledByHomer => "Homer may control".into(),
        }
    }

    /// Decided for every app, even if the app does not declare it: this is
    /// about what Homer may do, not what the app asks for.
    pub fn always_decided(self) -> bool {
        matches!(self, Key::ControlledByHomer)
    }

    /// The capability an app declares for this key, when there is one.
    pub fn from_capability(c: &Capability) -> Option<Key> {
        Some(match c {
            Capability::Camera => Key::Camera,
            Capability::Microphone => Key::Microphone,
            Capability::Sound => Key::Sound,
            Capability::Files(FileScope::KnownDir(d)) => Key::Folder(*d),
            Capability::Files(FileScope::UserPicked) => return None,
            Capability::ScreenCapture => Key::ScreenCapture,
            Capability::Clipboard(ClipboardAccess::Read) => Key::ClipboardRead,
            Capability::Clipboard(ClipboardAccess::Write) => Key::ClipboardWrite,
            Capability::Notifications => Key::Notifications,
            Capability::Location(_) => Key::Location,
        })
    }

    pub fn folder(self) -> Option<KnownDir> {
        match self {
            Key::Folder(d) => Some(d),
            _ => None,
        }
    }

    /// What Iliada can enforce for this key today. Keep this table honest:
    /// it is what the file, Settings, agora and Homer all say.
    pub fn rule(self) -> Rule {
        match self {
            Key::Network | Key::Sound => Rule::Decided { default: Value::Allow },
            Key::Folder(_) | Key::Camera | Key::Microphone => Rule::Decided { default: Value::Ask },
            Key::ScreenCapture => Rule::NotRestrictedYet {
                why: "the desktop does not yet tell sandboxed windows apart, so any app with a window could capture the screen",
            },
            Key::ClipboardRead => Rule::NotRestrictedYet {
                why: "the desktop gives every window the clipboard, so any app with a window could read what you copy",
            },
            Key::ClipboardWrite => Rule::NotRestrictedYet {
                why: "the desktop gives every window the clipboard, so any app with a window could replace what you copied",
            },
            Key::Background => Rule::NotRestrictedYet {
                why: "an app keeps running, even with no window open, until you quit it or log out",
            },
            Key::Notifications => Rule::Never { why: "there is no notification service inside the sandbox (no D-Bus)" },
            Key::Location => Rule::Never { why: "there is no location service inside the sandbox" },
            Key::Autostart => Rule::Never { why: "apps cannot see your startup folder or the system's services" },
            Key::Usb => Rule::Never { why: "no USB devices are put in the sandbox" },
            Key::Bluetooth => Rule::Never { why: "there is no Bluetooth service inside the sandbox" },
            Key::SystemChanges => Rule::Never { why: "the system service's socket is not in the sandbox" },
            Key::OtherApps => {
                Rule::Never { why: "each app has its own processes and memory, and no D-Bus session bus" }
            }
            Key::ControlledByHomer => Rule::Decided { default: Value::Ask },
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// A value in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    /// Allowed.
    Allow,
    /// Not allowed.
    Deny,
    /// Not decided: asked the next time the app opens (no access until then).
    Ask,
    /// A folder the app may read but not change.
    ReadOnly,
    /// A folder the app may read, change and delete files in.
    ReadWrite,
    /// The app does not declare it, so it never gets it.
    NotRequested,
    /// Iliada cannot restrict it yet.
    Unrestricted,
}

impl Value {
    pub fn name(self) -> &'static str {
        match self {
            Value::Allow => "allow",
            Value::Deny => "deny",
            Value::Ask => "ask",
            Value::ReadOnly => "read-only",
            Value::ReadWrite => "read-write",
            Value::NotRequested => "not-requested",
            Value::Unrestricted => "unrestricted",
        }
    }

    pub fn parse(s: &str) -> Option<Value> {
        [
            Value::Allow,
            Value::Deny,
            Value::Ask,
            Value::ReadOnly,
            Value::ReadWrite,
            Value::NotRequested,
            Value::Unrestricted,
        ]
        .into_iter()
        .find(|v| v.name() == s)
    }

    /// The app can use it now.
    pub fn grants(self) -> bool {
        matches!(self, Value::Allow | Value::ReadOnly | Value::ReadWrite | Value::Unrestricted)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// The values a person may write for `key`.
pub fn choices(key: Key) -> &'static [Value] {
    match (key.rule(), key.folder()) {
        (Rule::Decided { .. }, Some(_)) => &[Value::ReadWrite, Value::ReadOnly, Value::Deny, Value::Ask],
        (Rule::Decided { .. }, None) => &[Value::Allow, Value::Deny, Value::Ask],
        (Rule::Never { .. }, _) => &[Value::Deny],
        (Rule::NotRestrictedYet { .. }, _) => &[Value::Unrestricted],
    }
}

/// Words people type for a value ("on", "yes", "rw").
pub fn parse_choice(key: Key, s: &str) -> Option<Value> {
    let s = s.trim().to_lowercase();
    let v = match s.as_str() {
        "allow" | "on" | "yes" | "true" | "allowed" => {
            if key.folder().is_some() {
                Value::ReadWrite
            } else {
                Value::Allow
            }
        }
        "deny" | "off" | "no" | "false" | "denied" | "none" => Value::Deny,
        "rw" | "readwrite" | "read_write" => Value::ReadWrite,
        "ro" | "readonly" | "read_only" | "read" => Value::ReadOnly,
        other => Value::parse(other)?,
    };
    Some(v)
}

// ---------------------------------------------------------------------------
// Apps

/// Where an app comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// A sandboxed `.app` from agora (or built from Debian's package through agora).
    Agora,
    /// Part of Iliada or the desktop; runs as you, outside any sandbox.
    BuiltIn,
    /// A command-line tool from Settings → Tools; runs as you, outside any sandbox.
    Package,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Agora => "agora",
            Source::BuiltIn => "built-in",
            Source::Package => "package",
        }
    }

    pub fn sandboxed(self) -> bool {
        self == Source::Agora
    }
}

/// What Iliada knows about an app, from its validated bundle (never from the app).
#[derive(Debug, Clone, PartialEq)]
pub struct AppFacts {
    pub id: String,
    pub name: String,
    pub source: Source,
    /// How it is isolated (sandboxed apps only).
    pub isolation: Option<IsolationTier>,
    /// What it declares, with the purpose it gives.
    pub declared: BTreeMap<Key, String>,
    /// It adds results to Search under a keyword (ADR 0009).
    pub search: Option<crate::search::Extension>,
}

impl AppFacts {
    /// An installed bundle, isolated as `isolation` (its own setting, or the
    /// person's Extra isolation, or High security mode).
    pub fn from_bundle(b: &Bundle, isolation: IsolationTier) -> Self {
        let mut declared = BTreeMap::new();
        if b.info.network {
            declared.insert(Key::Network, "Uses the internet (shown in agora before installing).".to_owned());
        }
        for r in &b.permissions.requests {
            if let Some(k) = Key::from_capability(&r.capability) {
                declared.entry(k).or_insert_with(|| r.shown_purpose().to_owned());
            }
        }
        Self {
            id: b.info.id.as_str().to_owned(),
            name: b.info.name.clone(),
            source: Source::Agora,
            isolation: Some(isolation),
            declared,
            search: None,
        }
    }

    /// An app as agora describes it before installing.
    pub fn from_declared(id: &str, name: &str, network: bool, caps: &[Capability]) -> Self {
        let mut declared = BTreeMap::new();
        if network {
            declared.insert(Key::Network, String::new());
        }
        for c in caps {
            if let Some(k) = Key::from_capability(c) {
                declared.insert(k, String::new());
            }
        }
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            source: Source::Agora,
            isolation: Some(IsolationTier::Namespace),
            declared,
            search: None,
        }
    }

    /// An app as agora's catalog lists it: `network`, and what it may ask
    /// for by the permissions file's names (unknown names grant nothing).
    pub fn from_catalog<'a>(id: &str, name: &str, network: bool, keys: impl IntoIterator<Item = &'a str>) -> Self {
        let mut f = Self::from_declared(id, name, network, &[]);
        for k in keys.into_iter().filter_map(Key::parse) {
            f.declared.insert(k, String::new());
        }
        f
    }

    pub fn unsandboxed(id: &str, name: &str, source: Source) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            source,
            isolation: None,
            declared: BTreeMap::new(),
            search: None,
        }
    }

    pub fn declares(&self, key: Key) -> bool {
        self.declared.contains_key(&key)
    }
}

// ---------------------------------------------------------------------------
// Reading the files

/// Something wrong in a permissions file, said plainly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        match (&self.app, &self.key) {
            (Some(a), Some(k)) => write!(f, ", {a}, {k}")?,
            (Some(a), None) => write!(f, ", {a}")?,
            (None, Some(k)) => write!(f, ", {k}")?,
            (None, None) => {}
        }
        write!(f, ": {}", self.message)
    }
}

/// The `[homer]` table: the global switch, the whole-screen grant, and
/// reading notifications.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomerSection {
    /// `None`: not written. Defaults to on for a person, and not off for
    /// the computer.
    pub control: Option<bool>,
    /// Screenshots and the virtual mouse and keyboard. `None`: ask.
    pub screen: Option<Value>,
    /// Reading the person's notifications (the Notification Center's
    /// history). `None`: ask.
    pub notifications: Option<Value>,
}

/// One file, read. `broken`: Iliada could not read it at all, so every
/// decided capability is denied until it is fixed.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub exists: bool,
    pub broken: bool,
    pub problems: Vec<Problem>,
    /// app id (`*` for `[all]` in the system file) → key → value.
    pub apps: BTreeMap<String, BTreeMap<Key, Value>>,
    /// Values that were not understood (and so are `deny`).
    pub invalid: BTreeSet<(String, Key)>,
    pub homer: HomerSection,
}

/// Keys Iliada writes for people to read; never decisions.
const INFORMATIONAL: [&str; 9] =
    ["name", "source", "isolation", "risk", "worst_case", "summary", "installed", "all", "search_extension"];

/// Which file a [`parse`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// A person's answers.
    User,
    /// The computer's limits: only `deny`, or `read-only` for a folder.
    System,
}

fn shown_choices(key: Key) -> String {
    choices(key).iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ")
}

/// Read a permissions file's text. Never fails: problems are listed, and
/// anything not understood grants nothing.
pub fn parse(text: &str, file: &str, layer: Layer) -> Loaded {
    let mut out = Loaded { exists: true, ..Loaded::default() };
    let problem = |app: Option<&str>, key: Option<&str>, message: String| Problem {
        file: file.to_owned(),
        app: app.map(str::to_owned),
        key: key.map(str::to_owned),
        message,
    };
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            out.broken = true;
            let msg = e.to_string().lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
            out.problems.push(problem(
                None,
                None,
                format!("cannot be read ({msg}); every app is denied until it is fixed"),
            ));
            return out;
        }
    };
    match table.get("version") {
        None => {}
        Some(toml::Value::Integer(v)) if *v <= FORMAT_VERSION => {}
        Some(v) => {
            out.broken = true;
            out.problems.push(problem(
                None,
                Some("version"),
                format!("{v} is not a version this Iliada knows (it knows {FORMAT_VERSION}); every app is denied"),
            ));
            return out;
        }
    }
    for (k, v) in &table {
        match (k.as_str(), v) {
            ("version", _) => {}
            ("apps", toml::Value::Table(apps)) => {
                for (id, entry) in apps {
                    let toml::Value::Table(entry) = entry else {
                        out.broken = true;
                        out.problems.push(problem(
                            Some(id),
                            None,
                            "must be a table ([apps.\"id\"]); every app is denied until it is fixed".into(),
                        ));
                        continue;
                    };
                    let mut found = Vec::new();
                    let parsed = parse_app(entry, layer, &mut |key, m| found.push((key.map(str::to_owned), m)));
                    for (key, m) in found {
                        if let Some(k) = key.as_deref().and_then(Key::parse)
                            && m.ends_with("denied until it is fixed")
                        {
                            out.invalid.insert((id.clone(), k));
                        }
                        out.problems.push(problem(Some(id), key.as_deref(), m));
                    }
                    out.apps.insert(id.clone(), parsed);
                }
            }
            ("all", toml::Value::Table(entry)) if layer == Layer::System => {
                let mut found = Vec::new();
                let parsed = parse_app(entry, layer, &mut |key, m| found.push((key.map(str::to_owned), m)));
                out.problems.extend(found.into_iter().map(|(key, m)| problem(Some("all"), key.as_deref(), m)));
                out.apps.insert("*".into(), parsed);
            }
            ("homer", toml::Value::Table(t)) => {
                out.homer = parse_homer(t, layer, &mut |key, m| out.problems.push(problem(None, key, m)));
            }
            ("apps", _) | ("all", _) | ("homer", _) => {
                out.broken = true;
                out.problems.push(problem(
                    None,
                    Some(k),
                    "must be a table; every app is denied until it is fixed".into(),
                ));
            }
            _ => out.problems.push(problem(None, Some(k), "is not something this file can hold; ignored".into())),
        }
    }
    out
}

fn parse_homer(t: &toml::Table, layer: Layer, problem: &mut dyn FnMut(Option<&str>, String)) -> HomerSection {
    let mut s = HomerSection::default();
    for (k, v) in t {
        match k.as_str() {
            "control" => match (layer, v.as_bool()) {
                (_, None) => {
                    problem(Some("control"), format!("{v} must be true or false; treated as off"));
                    s.control = Some(false);
                }
                (Layer::System, Some(true)) => {
                    problem(Some("control"), "the computer's file can only switch this off; ignored".into())
                }
                (Layer::System, Some(false)) => s.control = Some(false),
                (Layer::User, Some(b)) => s.control = Some(b),
            },
            key @ ("screen" | "notifications") => {
                let Some(value) = v.as_str().and_then(Value::parse) else {
                    problem(Some(key), format!("{v} is not a value Iliada knows (use allow, deny or ask); ignored"));
                    continue;
                };
                let slot = if key == "screen" { &mut s.screen } else { &mut s.notifications };
                match layer {
                    Layer::System if value == Value::Deny => *slot = Some(Value::Deny),
                    Layer::System => problem(Some(key), format!("\"{value}\" cannot be a limit (use deny); ignored")),
                    Layer::User if matches!(value, Value::Allow | Value::Deny | Value::Ask) => *slot = Some(value),
                    Layer::User => {
                        problem(Some(key), format!("\"{value}\" does not apply here (use allow, deny or ask); ignored"))
                    }
                }
            }
            _ => problem(Some(k), "is not something [homer] can hold; ignored".into()),
        }
    }
    s
}

fn parse_app(entry: &toml::Table, layer: Layer, problem: &mut dyn FnMut(Option<&str>, String)) -> BTreeMap<Key, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in entry {
        if INFORMATIONAL.contains(&k.as_str()) {
            continue;
        }
        let Some(key) = Key::parse(k) else {
            problem(Some(k), "is not a capability Iliada knows; ignored (it grants nothing)".into());
            continue;
        };
        let Some(value) = v.as_str().and_then(Value::parse) else {
            problem(
                Some(k),
                format!("{v} is not a value Iliada knows (use {}); denied until it is fixed", shown_choices(key)),
            );
            out.insert(key, Value::Deny);
            continue;
        };
        match layer {
            Layer::System => {
                let ok = value == Value::Deny || (value == Value::ReadOnly && key.folder().is_some());
                if !ok {
                    let alt = if key.folder().is_some() { " or \"read-only\"" } else { "" };
                    problem(Some(k), format!("\"{value}\" cannot be a limit (use \"deny\"{alt}); ignored"));
                    continue;
                }
                out.insert(key, value);
            }
            Layer::User => match key.rule() {
                Rule::Decided { .. } => {
                    // `not-requested` is what Iliada writes for undeclared capabilities.
                    if value == Value::NotRequested {
                        continue;
                    }
                    if !choices(key).contains(&value) {
                        problem(
                            Some(k),
                            format!(
                                "\"{value}\" does not apply here (use {}); denied until it is fixed",
                                shown_choices(key)
                            ),
                        );
                        out.insert(key, Value::Deny);
                        continue;
                    }
                    out.insert(key, value);
                }
                Rule::Never { why } => {
                    if value != Value::Deny {
                        problem(Some(k), format!("stays \"deny\": Iliada has no way to give apps this ({why})"));
                    }
                }
                Rule::NotRestrictedYet { why } => {
                    if value != Value::Unrestricted {
                        problem(Some(k), format!("stays \"unrestricted\": Iliada cannot restrict this yet ({why})"));
                    }
                }
            },
        }
    }
    out
}

fn read(path: &Path, layer: Layer) -> Loaded {
    match std::fs::read_to_string(path) {
        Ok(t) => parse(&t, &path.display().to_string(), layer),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::default(),
        Err(e) => Loaded {
            exists: true,
            broken: true,
            problems: vec![Problem {
                file: path.display().to_string(),
                app: None,
                key: None,
                message: format!("cannot be read ({e}); every app is denied until it can"),
            }],
            ..Loaded::default()
        },
    }
}

// ---------------------------------------------------------------------------
// What an app gets

/// Who decided a capability's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecidedBy {
    /// The person (their file).
    You,
    /// Nobody yet: Iliada's default.
    Default,
    /// The computer's limits (`/etc/iliada/permissions.toml`).
    System,
    /// The app doesn't declare it.
    Declaration,
    /// Fixed by how Iliada works today (see `note`).
    Iliada,
    /// A permissions file could not be read, so it is denied.
    BrokenFile,
}

/// One capability of one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub key: Key,
    /// What the app gets now. `ask`: nothing until the person answers at
    /// the next launch.
    pub value: Value,
    /// What the person's file says (or Iliada's default), before limits.
    pub setting: Value,
    pub decided_by: DecidedBy,
    /// Whether Iliada enforces `value`. `false` means the app may be able
    /// to do it whatever the file says (see `note`).
    pub enforced: bool,
    /// Whether the person can change it.
    pub changeable: bool,
    /// Exactly what it reaches: a folder's path and mode, the network's reach.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The app's own reason, when it declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Everything about one app's access: the capability card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppReport {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub sandboxed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation: Option<IsolationTier>,
    /// What it can touch now, in a few words each.
    pub summary: Vec<String>,
    /// What it could do at worst, in one sentence.
    pub worst_case: String,
    /// What it can do, in plain sentences (generated from the capabilities).
    pub risk: Vec<String>,
    /// What every sandboxed app always has.
    pub always: Vec<String>,
    /// Capabilities still to be asked about at the next launch.
    pub asks: Vec<Key>,
    /// Every capability, by its key.
    pub capabilities: BTreeMap<&'static str, Entry>,
    /// A Search extension: its keyword, and whether it is on and online.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchReport>,
}

/// A Search extension's part of the card (ADR 0009). It only ever learns
/// what the person types after its keyword.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchReport {
    pub keyword: String,
    pub title: String,
    /// Turned on in Settings (and not waiting for a new keyword).
    pub enabled: bool,
    /// Search sends it queries now.
    pub active: bool,
    /// It can use the internet (declared, and allowed by the files).
    pub internet: bool,
    /// The extension that has this keyword: this one needs a new one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clashes_with: Option<String>,
}

impl SearchReport {
    /// One line for the file and the command line.
    pub fn sentence(&self) -> String {
        let mut s = format!("a Search extension: it sees only what you type after \"{} \"", self.keyword);
        s.push_str(if self.internet { ", and can use the internet" } else { ", with no internet" });
        if let Some(by) = &self.clashes_with {
            s.push_str(&format!("; waiting for a new keyword (\"{}\" is used by {by})", self.keyword));
        } else if !self.enabled {
            s.push_str("; turned off in Settings");
        }
        s
    }
}

impl AppReport {
    pub fn get(&self, key: Key) -> &Entry {
        &self.capabilities[key.name()]
    }
}

/// Where a person's folder is (`~/.config/user-dirs.dirs`, default
/// `~/<Name>`). A folder outside the home (or the home itself) is never
/// handed out.
pub fn user_dir(home: &Path, config_home: &Path, d: KnownDir) -> Option<PathBuf> {
    let key = format!("XDG_{}_DIR=", d.name().to_uppercase());
    let configured = std::fs::read_to_string(config_home.join("user-dirs.dirs"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix(&key).map(|v| v.trim_matches('"').to_owned())));
    let path = match configured {
        Some(v) => PathBuf::from(v.replace("$HOME", &home.display().to_string())),
        None => home.join(d.name()),
    };
    let escapes = path.components().any(|c| c == std::path::Component::ParentDir);
    (path.starts_with(home) && path != home && !escapes).then_some(path)
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The person's answers and the computer's limits.
#[derive(Debug, Clone)]
pub struct Policy {
    pub user_path: Option<PathBuf>,
    pub system_path: PathBuf,
    pub user: Loaded,
    pub system: Loaded,
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    /// Answers brought over from the old file, not written yet.
    pub migrated: bool,
}

impl Policy {
    /// `$XDG_CONFIG_HOME/iliada/permissions.toml`.
    pub fn default_user_path() -> Option<PathBuf> {
        Some(Self::config_home()?.join("iliada/permissions.toml"))
    }

    pub(crate) fn config_home() -> Option<PathBuf> {
        env_path("XDG_CONFIG_HOME").or_else(|| Some(env_path("HOME")?.join(".config")))
    }

    /// Where answers were kept before this file (`~/.local/share/iliada/permissions.toml`).
    pub fn legacy_path() -> Option<PathBuf> {
        let data = env_path("XDG_DATA_HOME").or_else(|| Some(env_path("HOME")?.join(".local/share")))?;
        Some(data.join("iliada/permissions.toml"))
    }

    /// This person's policy, from the usual places.
    pub fn load() -> Self {
        let system = env_path("ILIADA_SYSTEM_PERMISSIONS").unwrap_or_else(|| SYSTEM_PATH.into());
        Self::load_from(Self::default_user_path(), system, Self::legacy_path(), env_path("HOME"), Self::config_home())
    }

    pub fn load_from(
        user_path: Option<PathBuf>,
        system_path: PathBuf,
        legacy: Option<PathBuf>,
        home: Option<PathBuf>,
        config_home: Option<PathBuf>,
    ) -> Self {
        let mut user = user_path.as_deref().map(|p| read(p, Layer::User)).unwrap_or_default();
        let mut migrated = false;
        if !user.exists
            && let Some(old) = legacy
            && let Ok(text) = std::fs::read_to_string(&old)
            && let Ok(t) = text.parse::<toml::Table>()
        {
            // The old file: `["app"]` then `"files.pictures" = "allow"`.
            for (app, entry) in t {
                let toml::Value::Table(entry) = entry else { continue };
                let mut answers = BTreeMap::new();
                for (k, v) in entry {
                    let (Some(key), Some(v)) = (Key::parse(&k), v.as_str()) else { continue };
                    if !matches!(key.rule(), Rule::Decided { .. }) {
                        continue;
                    }
                    let value = match v {
                        "allow" if key.folder().is_some() => Value::ReadWrite,
                        "allow" => Value::Allow,
                        _ => Value::Deny,
                    };
                    answers.insert(key, value);
                }
                if !answers.is_empty() {
                    user.apps.insert(app, answers);
                    migrated = true;
                }
            }
        }
        let system = read(&system_path, Layer::System);
        Self { user_path, system_path, user, system, home, config_home, migrated }
    }

    /// Both files could be read (problems with single values may remain).
    pub fn readable(&self) -> bool {
        !self.user.broken && !self.system.broken
    }

    /// Homer may use other apps at all. Off if the person switched it off,
    /// an administrator did, or a file cannot be read.
    pub fn homer_enabled(&self) -> bool {
        self.readable() && self.user.homer.control.unwrap_or(true) && self.system.homer.control != Some(false)
    }

    /// An administrator has switched Homer's app control off for everyone.
    pub fn homer_forced_off(&self) -> bool {
        self.system.homer.control == Some(false)
    }

    pub fn set_homer_enabled(&mut self, on: bool) {
        self.user.homer.control = Some(on);
    }

    /// What Homer may do with the whole screen (screenshots, the virtual
    /// mouse and keyboard). The computer can only deny it.
    pub fn screen_control(&self) -> Value {
        if !self.readable() || self.system.homer.screen == Some(Value::Deny) || self.system.homer.control == Some(false)
        {
            return Value::Deny;
        }
        self.user.homer.screen.unwrap_or(Value::Ask)
    }

    /// Whether Homer may read the person's notifications (allow, deny, or
    /// ask each time). The computer can only deny it. Unlike using other
    /// apps, this does not depend on `control`: it is Homer reading, not
    /// acting.
    pub fn homer_notifications(&self) -> Value {
        if !self.readable() || self.system.homer.notifications == Some(Value::Deny) {
            return Value::Deny;
        }
        self.user.homer.notifications.unwrap_or(Value::Ask)
    }

    /// Set it (allow, deny or ask; anything else is ignored).
    pub fn set_homer_notifications(&mut self, v: Value) {
        if matches!(v, Value::Allow | Value::Deny | Value::Ask) {
            self.user.homer.notifications = Some(v);
        }
    }

    pub fn problems(&self) -> Vec<Problem> {
        self.system.problems.iter().chain(&self.user.problems).cloned().collect()
    }

    /// The person's own answer.
    pub fn answer(&self, app: &str, key: Key) -> Option<Value> {
        self.user.apps.get(app)?.get(&key).copied()
    }

    /// The computer's limit for `app`'s `key`, if any (the narrower of the
    /// app's own and `[all]`).
    pub fn limit(&self, app: &str, key: Key) -> Option<Value> {
        let get = |a: &str| self.system.apps.get(a).and_then(|m| m.get(&key)).copied();
        match (get(app), get("*")) {
            (Some(Value::Deny), _) | (_, Some(Value::Deny)) => Some(Value::Deny),
            (Some(v), _) | (_, Some(v)) => Some(v),
            _ => None,
        }
    }

    /// Record an answer (in memory; [`Policy::save`] or [`Policy::edit`]
    /// writes it). Refuses what the app doesn't declare, what Iliada can't
    /// enforce, and any change while the file can't be read.
    pub fn set(&mut self, app: &AppFacts, key: Key, value: Value) -> Result<(), String> {
        if self.user.broken {
            return Err("the permissions file has a mistake: fix it, or reset it, first".into());
        }
        if !app.source.sandboxed() && !key.always_decided() {
            return Err(format!("{} runs as you, outside any sandbox: Iliada cannot limit it", app.name));
        }
        match key.rule() {
            Rule::Never { why } => return Err(format!("{}: Iliada never gives apps this ({why})", key.label())),
            Rule::NotRestrictedYet { why } => {
                return Err(format!("{}: Iliada cannot restrict this yet ({why})", key.label()));
            }
            Rule::Decided { .. } => {}
        }
        if !app.declares(key) && !key.always_decided() {
            return Err(format!("{} does not ask for {}, so it never gets it", app.name, key.phrase()));
        }
        if !choices(key).contains(&value) {
            let shown = choices(key).iter().map(|v| v.name()).collect::<Vec<_>>().join(", ");
            return Err(format!("{} can be {shown}", key.name()));
        }
        self.user.apps.entry(app.id.clone()).or_default().insert(key, value);
        Ok(())
    }

    /// Forget answers (asked again next time): one app's, or everyone's.
    pub fn reset(&mut self, app: Option<&str>) {
        match app {
            Some(a) => {
                self.user.apps.remove(a);
            }
            None => self.user.apps.clear(),
        }
        if app.is_none() {
            // A full reset starts a clean file, even from a broken one.
            self.user.broken = false;
            self.user.problems.clear();
            self.user.homer = HomerSection::default();
        }
    }

    /// What `app` gets, exactly.
    pub fn report(&self, app: &AppFacts) -> AppReport {
        let mut caps: BTreeMap<&'static str, Entry> = BTreeMap::new();
        let tier = app.isolation.unwrap_or_default();
        for key in Key::ALL {
            let mut e = Entry {
                key,
                value: Value::Deny,
                setting: Value::Deny,
                decided_by: DecidedBy::Iliada,
                enforced: true,
                changeable: false,
                scope: None,
                note: None,
                purpose: app.declared.get(&key).filter(|p| !p.is_empty()).cloned(),
            };
            if !app.source.sandboxed() && !key.always_decided() {
                e.value = Value::Unrestricted;
                e.setting = Value::Unrestricted;
                e.enforced = false;
                e.note = Some("runs as you, outside any sandbox: it can do whatever you can".into());
                caps.insert(key.name(), e);
                continue;
            }
            match key.rule() {
                Rule::Never { why } => e.note = Some(why.into()),
                Rule::NotRestrictedYet { why } => {
                    e.value = Value::Unrestricted;
                    e.setting = Value::Unrestricted;
                    e.enforced = false;
                    e.note = Some(format!("not restricted yet: {why}"));
                }
                Rule::Decided { default } => {
                    if !app.declares(key) && !key.always_decided() {
                        e.value = Value::NotRequested;
                        e.setting = Value::NotRequested;
                        e.decided_by = DecidedBy::Declaration;
                    } else if !self.readable() {
                        e.setting = self.answer(&app.id, key).unwrap_or(default);
                        e.decided_by = DecidedBy::BrokenFile;
                        e.note = Some("denied until the permissions file is fixed".into());
                        e.changeable = true;
                    } else {
                        e.changeable = true;
                        let (setting, by) = match self.answer(&app.id, key) {
                            Some(v) => (v, DecidedBy::You),
                            None => (default, DecidedBy::Default),
                        };
                        e.setting = setting;
                        e.value = setting;
                        e.decided_by = by;
                        if self.user.invalid.contains(&(app.id.clone(), key)) {
                            e.decided_by = DecidedBy::BrokenFile;
                            e.note = Some("the file's value was not understood, so it is denied".into());
                        }
                        match (self.limit(&app.id, key), setting) {
                            (Some(Value::Deny), s) if s != Value::Deny => {
                                e.value = Value::Deny;
                                e.decided_by = DecidedBy::System;
                                e.note = Some("an administrator turned this off on this computer".into());
                            }
                            (Some(Value::ReadOnly), Value::ReadWrite | Value::Ask) => {
                                if setting == Value::ReadWrite {
                                    e.value = Value::ReadOnly;
                                    e.decided_by = DecidedBy::System;
                                }
                                e.note =
                                    Some("an administrator limited this folder to reading on this computer".into());
                            }
                            _ => {}
                        }
                    }
                }
            }
            caps.insert(key.name(), e);
        }

        if app.source.sandboxed() {
            adjust_for_how_sandboxes_work(&mut caps, tier);
            for d in KnownDir::ALL {
                let path = match (&self.home, &self.config_home) {
                    (Some(h), Some(c)) => user_dir(h, c, d),
                    _ => None,
                };
                let shown = path.map(|p| p.display().to_string()).unwrap_or_else(|| format!("~/{}", d.name()));
                let e = caps.get_mut(Key::Folder(d).name()).expect("every key is present");
                e.scope = match e.value {
                    Value::ReadWrite => Some(format!("{shown} and everything in it: read, change, delete")),
                    Value::ReadOnly => Some(format!("{shown} and everything in it: read only")),
                    _ => None,
                };
            }
        }
        let always = if app.source.sandboxed() {
            let data = self
                .home
                .as_ref()
                .map(|h| h.join(".local/share/iliada/apps").join(&app.id).display().to_string())
                .unwrap_or_else(|| format!("~/.local/share/iliada/apps/{}", app.id));
            vec![
                format!("its own private folder, {data} (read, change, delete)"),
                "files you open with it, one by one (read and change)".into(),
                "a window on your screen".into(),
            ]
        } else {
            vec!["everything you can do: all your files, the internet, every device you can use".into()]
        };
        let asks = caps.values().filter(|e| e.value == Value::Ask && !e.key.always_decided()).map(|e| e.key).collect();
        let mut r = AppReport {
            id: app.id.clone(),
            name: app.name.clone(),
            source: app.source,
            sandboxed: app.source.sandboxed(),
            isolation: app.isolation,
            summary: Vec::new(),
            worst_case: String::new(),
            risk: Vec::new(),
            always,
            asks,
            capabilities: caps,
            search: None,
        };
        describe(&mut r);
        if let Some(ext) = &app.search {
            let internet = app.source.sandboxed() && r.get(Key::Network).value.grants();
            r.summary.push(format!("what you type after \"{}\" in Search", ext.keyword));
            r.search = Some(SearchReport {
                keyword: ext.keyword.clone(),
                title: ext.title.clone(),
                enabled: ext.enabled,
                active: ext.active(),
                internet,
                clashes_with: ext.clashes_with.clone(),
            });
        }
        r
    }

    /// The card agora shows before installing: what the app could do if
    /// everything it asks for were allowed.
    pub fn preview(app: &AppFacts) -> AppReport {
        let mut p = Policy {
            user_path: None,
            system_path: PathBuf::new(),
            user: Loaded::default(),
            system: Loaded::default(),
            home: None,
            config_home: None,
            migrated: false,
        };
        let answers = app
            .declared
            .keys()
            .filter(|k| matches!(k.rule(), Rule::Decided { .. }))
            .map(|k| (*k, if k.folder().is_some() { Value::ReadWrite } else { Value::Allow }))
            .collect();
        p.user.apps.insert(app.id.clone(), answers);
        p.report(app)
    }

    /// The person's file, as Iliada writes it: every app in `apps`, plus
    /// answers kept for apps that are not installed now.
    pub fn render(&self, apps: &[AppFacts]) -> String {
        let mut s = String::from(HEADER);
        s.push_str(&format!("version = {FORMAT_VERSION}\n"));
        s.push_str(&render_homer(&self.user.homer, self.system.homer.control == Some(false)));
        let mut seen = BTreeSet::new();
        let mut sorted: Vec<&AppFacts> = apps.iter().collect();
        sorted.sort_by_key(|a| (!a.source.sandboxed(), a.name.to_lowercase()));
        let width = Key::ALL.iter().map(|k| key_name(*k).len()).max().unwrap_or(0);
        for app in sorted {
            if !seen.insert(app.id.clone()) {
                continue;
            }
            s.push('\n');
            s.push_str(&format!("[apps.{}]\n", quote(&app.id)));
            s.push_str(&format!("name = {}\n", quote(&app.name)));
            s.push_str(&format!("source = {}\n", quote(app.source.name())));
            if !app.source.sandboxed() {
                s.push_str("all = \"unrestricted\"  # runs as you, outside any sandbox: it can do whatever you can\n");
                let r = self.report(app);
                let e = r.get(Key::ControlledByHomer);
                s.push_str(&format!("controlled_by_homer = {:<8}  # {}\n", quote(e.setting.name()), file_comment(e)));
                continue;
            }
            let r = self.report(app);
            if let Some(t) = app.isolation {
                let t = match t {
                    IsolationTier::Namespace => "sandbox",
                    IsolationTier::MicroVm => "micro-vm",
                };
                s.push_str(&format!("isolation = {}\n", quote(t)));
            }
            if let Some(ext) = &r.search {
                s.push_str(&format!("search_extension = {}  # {}\n", quote(&ext.keyword), ext.sentence()));
            }
            s.push_str(&format!("worst_case = {}\n", quote(&r.worst_case)));
            s.push_str("risk = [\n");
            for line in &r.risk {
                s.push_str(&format!("  {},\n", quote(line)));
            }
            s.push_str("]\n");
            for key in Key::ALL {
                let e = r.get(key);
                let k = key_name(key);
                let line = format!("{k}{} = {}", " ".repeat(width - k.len()), quote(e.setting.name()));
                let comment = file_comment(e);
                if comment.is_empty() {
                    s.push_str(&format!("{line}\n"));
                } else {
                    s.push_str(&format!("{line:<pad$}  # {comment}\n", pad = width + 18));
                }
            }
        }
        for (id, answers) in &self.user.apps {
            if seen.contains(id) || answers.is_empty() {
                continue;
            }
            s.push('\n');
            s.push_str(&format!("[apps.{}]  # not installed now; kept for when it is\n", quote(id)));
            for (k, v) in answers {
                s.push_str(&format!("{} = {}\n", key_name(*k), quote(v.name())));
            }
        }
        s
    }

    /// Write the person's file (atomically). Refuses while it can't be read,
    /// so a mistake is never overwritten with defaults.
    pub fn save(&mut self, apps: &[AppFacts]) -> std::io::Result<()> {
        if self.user.broken {
            return Err(std::io::Error::other("the permissions file has a mistake; it was left as it is"));
        }
        let Some(path) = &self.user_path else { return Err(std::io::Error::other("HOME is not set")) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = self.render(apps);
        if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
            self.migrated = false;
            return Ok(());
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        self.user.exists = true;
        self.migrated = false;
        Ok(())
    }

    /// Change answers safely while other programs may be writing too:
    /// lock, re-read the file, change, write.
    pub fn edit(
        &mut self,
        apps: &[AppFacts],
        change: impl FnOnce(&mut Policy) -> Result<(), String>,
    ) -> Result<(), String> {
        let path = self.user_path.clone().ok_or("HOME is not set")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("toml.lock"))
            .map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        let mut fresh =
            Policy::load_from(Some(path), self.system_path.clone(), None, self.home.clone(), self.config_home.clone());
        if self.migrated && !fresh.user.exists {
            // Answers brought over from the old file are only in memory so far.
            fresh.user.apps = self.user.apps.clone();
        }
        change(&mut fresh)?;
        fresh.save(apps).map_err(|e| e.to_string())?;
        *self = fresh;
        Ok(())
    }
}

/// What sandboxes really do, beyond the person's answers.
fn adjust_for_how_sandboxes_work(caps: &mut BTreeMap<&'static str, Entry>, tier: IsolationTier) {
    fn get<'a>(caps: &'a mut BTreeMap<&'static str, Entry>, k: Key) -> &'a mut Entry {
        caps.get_mut(k.name()).expect("every key is present")
    }
    let value = |caps: &BTreeMap<&'static str, Entry>, k: Key| caps[k.name()].value;
    // A micro-VM has no camera yet.
    if tier == IsolationTier::MicroVm && get(caps, Key::Camera).value == Value::Allow {
        let e = get(caps, Key::Camera);
        e.value = Value::Deny;
        e.decided_by = DecidedBy::Iliada;
        e.note = Some("no camera in a micro-VM yet".into());
    }
    // One sound-server connection carries both playing and recording.
    let sound = value(caps, Key::Sound) == Value::Allow;
    let mic = value(caps, Key::Microphone) == Value::Allow;
    if sound && !mic {
        let e = get(caps, Key::Microphone);
        e.value = Value::Unrestricted;
        e.enforced = false;
        e.note = Some(
            "not restricted while sound is allowed: one connection to the sound server carries both playing and recording, so deny sound to block the microphone"
                .into(),
        );
    }
    if mic && !sound {
        let e = get(caps, Key::Sound);
        e.value = Value::Unrestricted;
        e.enforced = false;
        e.note = Some("allowed with the microphone: one connection to the sound server carries both".into());
    }
    if get(caps, Key::Network).value == Value::Allow {
        get(caps, Key::Network).scope = Some(match tier {
            IsolationTier::Namespace => {
                "any address: the internet, your local network, and programs listening on this computer".into()
            }
            IsolationTier::MicroVm => {
                "any address on the internet and your local network (not this computer's own services)".into()
            }
        });
        if tier == IsolationTier::Namespace {
            let e = get(caps, Key::OtherApps);
            e.value = Value::Unrestricted;
            e.setting = Value::Deny;
            e.enforced = false;
            e.note = Some(
                "only over the network: it shares this computer's network, so it can reach programs listening on this computer"
                    .into(),
            );
        }
        get(caps, Key::Location).note = Some(
            "no location service, but with the internet it can guess your rough location from your address".into(),
        );
    }
}

/// The comment after a value in the file.
fn file_comment(e: &Entry) -> String {
    let mut c = Vec::new();
    match (e.key.rule(), e.decided_by) {
        (_, DecidedBy::Declaration) => c.push("the app does not ask for this".to_owned()),
        (Rule::Never { .. }, _) => c.push("fixed".to_owned()),
        (Rule::NotRestrictedYet { .. }, _) => c.push("NOT RESTRICTED YET".to_owned()),
        _ => c.push(choices(e.key).iter().map(|v| v.name()).collect::<Vec<_>>().join(" | ")),
    }
    if e.value != e.setting {
        c.push(format!("in effect: {}", e.value));
    }
    if let Some(n) = &e.note
        && !matches!(e.key.rule(), Rule::NotRestrictedYet { .. })
    {
        c.push(n.clone());
    } else if let Rule::NotRestrictedYet { why } = e.key.rule() {
        c.push(why.to_owned());
    }
    c.join("; ")
}

fn key_name(k: Key) -> String {
    let n = k.name();
    if n.contains('.') { quote(n) } else { n.to_owned() }
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

fn render_homer(user: &HomerSection, system_off: bool) -> String {
    let on = user.control.unwrap_or(true);
    let screen = user.screen.unwrap_or(Value::Ask);
    let notifications = user.notifications.unwrap_or(Value::Ask);
    let mut s = String::from(
        "
[homer]
         # Homer may use other apps at all (Settings > Homer > Use other apps).
",
    );
    s.push_str(&format!(
        "control = {on}
"
    ));
    if system_off {
        s.push_str(
            "# in effect: false (an administrator turned this off on this computer)
",
        );
    }
    s.push_str(
        "# Screenshots and the virtual mouse and keyboard, for apps with no accessibility.
         # allow | deny | ask
",
    );
    s.push_str(&format!(
        "screen = {}
",
        quote(screen.name())
    ));
    s.push_str(
        "# Reading your notifications (Settings > Notifications): what they say
         # can be private. allow | deny | ask (asked each time)
",
    );
    s.push_str(&format!(
        "notifications = {}
",
        quote(notifications.name())
    ));
    s
}

const HEADER: &str = "\
# What every app may use on this computer, for you. One table per app.
#
# Iliada reads this file every time an app opens, and writes it when you
# answer a permission question, change Settings > Privacy, run
# `iliada permissions set`, or confirm a change Homer proposes. You can edit
# the values by hand: changes apply the next time the app opens. Iliada
# rewrites everything else (names, risk, worst_case, comments) from the
# values, so the summaries always match what is enforced.
#
# Values:
#   allow, deny       the app may, or may not, use it
#   read-write        a folder: read, change and delete everything in it
#   read-only         a folder: read it, change nothing
#   ask               not decided: asked the next time the app opens
#   not-requested     the app does not ask for it, so it never gets it
#   unrestricted      Iliada cannot restrict this yet (said, not hidden)
# A value marked `fixed` is how Iliada works and cannot be changed here.
# `in effect:` shows where the computer's limits or the sandbox give less
# (or, marked NOT RESTRICTED, more) than the value says.
#
# Every sandboxed app also always has its own private folder
# (~/.local/share/iliada/apps/<id>), the files you open with it, and a window.
# Built-in programs and command-line tools are not sandboxed: they can do
# whatever you can.
#
# Safety: if this file cannot be read, every app is denied everything you
# decide here until it is fixed (`iliada permissions check` says where).
# A value Iliada does not understand denies that one thing.
#
# The computer's administrator can narrow any of this for everyone in
# /etc/iliada/permissions.toml; that always wins.
# Machine-readable: `iliada permissions --json` (docs/permissions-schema.md).
#
# [homer] is the global switch for Homer using other apps (Settings >
# Homer > Use other apps), `screen` for screenshots and the virtual
# mouse and keyboard, and `notifications` for reading your notifications.
# `controlled_by_homer` on each app is whether Homer
# may use that app (allow, deny or ask). It is decided for every app,
# including built-in ones.

";

// ---------------------------------------------------------------------------
// Plain words

/// Fill in a report's summary, risk and worst case from its capabilities.
fn describe(r: &mut AppReport) {
    if !r.sandboxed {
        r.summary = vec!["Everything you can reach (not sandboxed)".into()];
        r.worst_case = "It runs as you, outside any sandbox: it could read, change or delete any of your files and send them anywhere.".into();
        r.risk = vec![
            "Runs as you, outside any sandbox: Iliada cannot limit what it does.".into(),
            "Can read, change and delete all of your files, use the internet, and use every device you can.".into(),
        ];
        return;
    }
    let v = |k: Key| r.get(k).value;
    let mut summary = Vec::new();
    let mut risk = Vec::new();
    let (mut rw, mut ro, mut asks) = (Vec::new(), Vec::new(), Vec::new());
    for d in KnownDir::ALL {
        match v(Key::Folder(d)) {
            Value::ReadWrite => rw.push(d.name()),
            Value::ReadOnly => ro.push(d.name()),
            Value::Ask => asks.push(format!("your {} folder", d.name())),
            _ => {}
        }
    }
    for d in &rw {
        summary.push(format!("{d} (read-write)"));
        risk.push(format!("Can read, change and delete everything in your {d} folder."));
    }
    for d in &ro {
        summary.push(format!("{d} (read-only)"));
        risk.push(format!("Can read everything in your {d} folder, but not change it."));
    }
    let net = v(Key::Network) == Value::Allow;
    if net {
        summary.push("Internet".into());
        risk.push("Can connect to any address on the internet and send out anything it can read.".into());
        if r.isolation == Some(IsolationTier::Namespace) {
            risk.push("Can reach programs listening on this computer and your local network.".into());
        }
    }
    if v(Key::Camera) == Value::Allow {
        summary.push("Camera".into());
        risk.push("Can see through your camera while it is open.".into());
    }
    match v(Key::Microphone) {
        Value::Allow => {
            summary.push("Microphone".into());
            risk.push("Can record from your microphone while it is open.".into());
        }
        Value::Unrestricted => {
            summary.push("Microphone (not restricted yet)".into());
            risk.push(
                "Could record from your microphone: it may play sound, and Iliada can't separate playing from recording yet."
                    .into(),
            );
        }
        _ => {}
    }
    if v(Key::Sound).grants() {
        summary.push("Sound".into());
    }
    for k in [Key::Camera, Key::Microphone, Key::Network] {
        if v(k) == Value::Ask {
            asks.push(format!("the {}", k.phrase()));
        }
    }
    if !asks.is_empty() {
        risk.push(format!("Will ask before it can use {}.", join(&asks)));
    }
    match v(Key::ControlledByHomer) {
        Value::Allow => {
            summary.push("Homer may use it".into());
            risk.push("Homer can read this app's windows, press its buttons and type into it.".into());
        }
        Value::Ask => risk.push("Homer will ask before using this app.".into()),
        _ => {}
    }
    let loose: Vec<&str> = [(Key::ClipboardRead, "read what you copy"), (Key::ScreenCapture, "capture your screen")]
        .into_iter()
        .filter(|(k, _)| v(*k) == Value::Unrestricted)
        .map(|(_, what)| what)
        .collect();
    if !loose.is_empty() {
        summary.push("Clipboard and screen (not restricted yet)".into());
        risk.push(format!("Not restricted yet: could {} (Iliada will restrict this).", join(&loose)));
    }
    if v(Key::Background) == Value::Unrestricted {
        risk.push("Keeps running until you quit it, even with no window open.".into());
    }
    let never: Vec<String> = Key::ALL
        .into_iter()
        .filter(|k| matches!(k.rule(), Rule::Never { .. }) && v(*k) == Value::Deny)
        .map(|k| k.phrase())
        .collect();
    if !never.is_empty() {
        risk.push(format!("Cannot use: {}.", join(&never)));
    }
    if summary.is_empty() {
        summary.push("Only its own files".into());
    }

    // Worst case: what it could damage, and what it could leak.
    let mut clauses = Vec::new();
    if !rw.is_empty() {
        clauses.push(format!("delete or change everything in your {} folder{}", join(&rw), plural(rw.len())));
    }
    if net {
        let mut sources: Vec<String> = Vec::new();
        let readable: Vec<&str> = rw.iter().chain(&ro).copied().collect();
        if !readable.is_empty() {
            sources.push(format!("your {} files", join(&readable)));
        }
        if v(Key::Camera) == Value::Allow {
            sources.push("what your camera sees".into());
        }
        if v(Key::Microphone).grants() {
            sources.push("what your microphone hears".into());
        }
        if v(Key::ClipboardRead) == Value::Unrestricted {
            sources.push("what you copy".into());
        }
        if v(Key::ScreenCapture) == Value::Unrestricted {
            sources.push("your screen".into());
        }
        if sources.is_empty() {
            sources.push("what you type into it".into());
        }
        clauses.push(format!("send {} to anyone on the internet", join(&sources)));
    }
    let mut worst = if clauses.is_empty() {
        if ro.is_empty() {
            "It can only affect its own files: it has no internet and none of your folders.".into()
        } else {
            format!("It could read your {} files, but cannot change them or send them anywhere.", join(&ro))
        }
    } else {
        let mut s = format!("It could {}", join(&clauses));
        if !net {
            s.push_str(", but it cannot send anything anywhere (no internet)");
        }
        s.push('.');
        s
    };
    if !net && !loose.is_empty() {
        worst.push_str(&format!(
            " Not restricted yet: it could {}, though it has no internet to send anything out.",
            join(&loose)
        ));
    }
    r.worst_case = worst;
    r.summary = summary;
    r.risk = risk;
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn join<S: AsRef<str>>(items: &[S]) -> String {
    let items: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
    match items.len() {
        0 => String::new(),
        1 => items[0].to_owned(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

// ---------------------------------------------------------------------------
// Installed programs

/// Where agora installs apps.
pub const APPLICATIONS: &str = "/Applications";

/// Apps the person put in their own micro-VM (Settings → Privacy → Extra
/// isolation): `~/.config/iliada/extra-isolation`, one id per line.
pub fn extra_isolation() -> Vec<String> {
    Policy::config_home()
        .and_then(|c| std::fs::read_to_string(c.join("iliada/extra-isolation")).ok())
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// High security mode: every app in a micro-VM (`/etc/iliada/isolation.json`).
fn high_security() -> bool {
    std::fs::read_to_string("/etc/iliada/isolation.json")
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("high_security").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// How an installed bundle is isolated when it opens.
pub fn isolation_of(b: &Bundle, extra: &[String], high_security: bool) -> IsolationTier {
    if high_security || extra.iter().any(|a| a == b.info.id.as_str()) {
        IsolationTier::MicroVm
    } else {
        b.info.isolation
    }
}

/// Installed apps, as bundles with their facts.
pub fn installed_bundles() -> Vec<(Bundle, AppFacts)> {
    let (extra, high) = (extra_isolation(), high_security());
    let dir = env_path("ILIADA_APPLICATIONS").unwrap_or_else(|| APPLICATIONS.into());
    let mut out: Vec<(Bundle, AppFacts)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Bundle::open(e.path()).ok())
        .map(|b| {
            let f = AppFacts::from_bundle(&b, isolation_of(&b, &extra, high));
            (b, f)
        })
        .collect();
    out.sort_by_key(|(_, f)| f.name.to_lowercase());
    // Search extensions: keywords shared out among all of them.
    let candidates = out.iter().filter_map(|(b, _)| crate::search::candidate(b)).collect();
    let choices = crate::search::Choices::default_path().map(|p| crate::search::Choices::load(&p)).unwrap_or_default();
    for ext in crate::search::resolve(candidates, &choices) {
        if let Some((_, f)) = out.iter_mut().find(|(_, f)| f.id == ext.id) {
            f.search = Some(ext);
        }
    }
    out
}

/// Every program on this computer the permissions file describes:
/// sandboxed apps, then built-in programs and command-line tools.
pub fn installed_programs() -> Vec<AppFacts> {
    let mut apps: Vec<AppFacts> = installed_bundles().into_iter().map(|(_, f)| f).collect();
    apps.extend(unsandboxed_programs());
    apps
}

/// Find a program by id or name (any case).
pub fn find<'a>(apps: &'a [AppFacts], name: &str) -> Option<&'a AppFacts> {
    let n = name.trim().to_lowercase();
    let short = |s: &str| s.to_lowercase().replace(' ', "-");
    apps.iter()
        .find(|a| a.id == name)
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase() == n || a.id.to_lowercase() == n))
        .or_else(|| apps.iter().find(|a| short(&a.name) == short(&n)))
        .or_else(|| apps.iter().find(|a| a.id.rsplit('.').next().is_some_and(|last| last.to_lowercase() == n)))
}

// ---------------------------------------------------------------------------
// Programs that are not sandboxed

/// Iliada's and the desktop's own programs, and command-line tools: they
/// run as the person, outside any sandbox. Listed so nothing is hidden.
pub fn unsandboxed_programs() -> Vec<AppFacts> {
    let mut out = Vec::new();
    for e in std::fs::read_dir("/run/current-system/sw/share/applications").into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "desktop") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let field = |k: &str| {
            text.lines()
                .skip_while(|l| l.trim() != "[Desktop Entry]")
                .take_while(|l| !l.starts_with("[Desktop Action"))
                .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')).map(str::trim))
        };
        if field("NoDisplay") == Some("true") || field("Exec").is_some_and(|x| x.contains("iliada-run")) {
            continue;
        }
        let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = field("Name").unwrap_or(&id).to_owned();
        out.push(AppFacts::unsandboxed(&id, &name, Source::BuiltIn));
    }
    if let Ok(text) = std::fs::read_to_string("/etc/iliada/settings.toml")
        && let Ok(t) = text.parse::<toml::Table>()
        && let Some(pkgs) = t.get("tools").and_then(|t| t.get("packages")).and_then(|p| p.as_array())
    {
        for p in pkgs.iter().filter_map(|p| p.as_str()) {
            out.push(AppFacts::unsandboxed(&format!("package:{p}"), p, Source::Package));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gimp() -> AppFacts {
        AppFacts::from_declared(
            "org.gimp.GIMP",
            "GIMP",
            true,
            &[Capability::Files(FileScope::KnownDir(KnownDir::Pictures)), Capability::Camera, Capability::Sound],
        )
    }

    fn policy(dir: &Path, user: Option<&str>, system: Option<&str>) -> Policy {
        let u = dir.join("user.toml");
        let s = dir.join("system.toml");
        if let Some(t) = user {
            std::fs::write(&u, t).unwrap();
        }
        if let Some(t) = system {
            std::fs::write(&s, t).unwrap();
        }
        Policy::load_from(Some(u), s, None, Some(dir.join("home")), Some(dir.join("home/.config")))
    }

    #[test]
    fn every_key_and_value_round_trips() {
        for k in Key::ALL {
            assert_eq!(Key::parse(k.name()), Some(k));
        }
        for v in [Value::Allow, Value::ReadOnly, Value::NotRequested, Value::Unrestricted] {
            assert_eq!(Value::parse(v.name()), Some(v));
        }
        assert_eq!(parse_choice(Key::Folder(KnownDir::Music), "on"), Some(Value::ReadWrite));
        assert_eq!(parse_choice(Key::Camera, "on"), Some(Value::Allow));
        assert_eq!(parse_choice(Key::Folder(KnownDir::Music), "ro"), Some(Value::ReadOnly));
    }

    #[test]
    fn defaults_follow_the_declaration() {
        let t = tempfile::tempdir().unwrap();
        let r = policy(t.path(), None, None).report(&gimp());
        assert_eq!(r.get(Key::Network).value, Value::Allow);
        assert_eq!(r.get(Key::Folder(KnownDir::Pictures)).value, Value::Ask);
        assert_eq!(r.get(Key::Folder(KnownDir::Documents)).value, Value::NotRequested);
        assert_eq!(r.get(Key::Camera).value, Value::Ask);
        assert_eq!(r.get(Key::ControlledByHomer).value, Value::Ask);
        assert!(r.get(Key::ControlledByHomer).changeable);
        assert_eq!(r.get(Key::Notifications).value, Value::Deny);
        assert_eq!(r.get(Key::ClipboardRead).value, Value::Unrestricted);
        assert!(!r.get(Key::ClipboardRead).enforced);
        // Sound allowed: the microphone is honestly not restricted.
        assert_eq!(r.get(Key::Microphone).value, Value::Unrestricted);
        assert!(r.asks.contains(&Key::Camera));
    }

    #[test]
    fn a_broken_file_denies_everything_decided_and_is_not_overwritten() {
        let t = tempfile::tempdir().unwrap();
        let broken = "[apps.\"org.gimp.GIMP\"\nnetwork = \"allow\"\n";
        let mut p = policy(t.path(), Some(broken), None);
        assert!(!p.readable());
        assert!(p.problems()[0].message.contains("every app is denied"), "{:?}", p.problems());
        let r = p.report(&gimp());
        for k in [Key::Network, Key::Camera, Key::Folder(KnownDir::Pictures), Key::Sound] {
            assert_eq!(r.get(k).value, Value::Deny, "{k}");
            assert_eq!(r.get(k).decided_by, DecidedBy::BrokenFile);
        }
        assert_eq!(r.get(Key::Microphone).value, Value::NotRequested);
        assert!(r.asks.is_empty(), "no prompts that would write over the file");
        assert!(p.save(&[gimp()]).is_err());
        assert!(p.set(&gimp(), Key::Camera, Value::Allow).is_err());
        assert!(p.edit(&[gimp()], |p| p.set(&gimp(), Key::Camera, Value::Allow)).is_err());
        assert_eq!(std::fs::read_to_string(t.path().join("user.toml")).unwrap(), broken);
        // Only a full reset starts over.
        p.edit(&[gimp()], |p| {
            p.reset(None);
            Ok(())
        })
        .unwrap();
        assert!(p.readable());
    }

    #[test]
    fn a_broken_system_file_denies_too() {
        let t = tempfile::tempdir().unwrap();
        let p = policy(t.path(), Some("[apps.\"org.gimp.GIMP\"]\nnetwork = \"allow\"\n"), Some("version = ["));
        assert_eq!(p.report(&gimp()).get(Key::Network).value, Value::Deny);
    }

    #[test]
    fn unknown_versions_wrong_types_and_bad_values_never_widen() {
        let t = tempfile::tempdir().unwrap();
        assert!(!policy(t.path(), Some("version = 99\n"), None).readable());
        assert!(!policy(t.path(), Some("apps = 3\n"), None).readable());
        assert!(!policy(t.path(), Some("[apps]\n\"org.gimp.GIMP\" = \"allow\"\n"), None).readable());
        let p = policy(
            t.path(),
            Some(
                "[apps.\"org.gimp.GIMP\"]\nnetwork = \"yes please\"\ncamera = \"read-write\"\nsound = 1\n\
                 \"clipboard.read\" = \"deny\"\nnotifications = \"allow\"\nteleport = \"allow\"\n",
            ),
            None,
        );
        assert!(p.readable());
        let r = p.report(&gimp());
        assert_eq!(r.get(Key::Network).value, Value::Deny);
        assert_eq!(r.get(Key::Camera).value, Value::Deny);
        assert_eq!(r.get(Key::Sound).value, Value::Deny);
        // It can't pretend to restrict what Iliada can't, or give what it never does.
        assert_eq!(r.get(Key::ClipboardRead).value, Value::Unrestricted);
        assert_eq!(r.get(Key::Notifications).value, Value::Deny);
        let msgs: Vec<String> = p.problems().iter().map(ToString::to_string).collect();
        assert_eq!(msgs.len(), 6, "{msgs:#?}");
        assert!(msgs.iter().any(|m| m.contains("teleport") && m.contains("grants nothing")));
        assert!(msgs.iter().any(|m| m.contains("clipboard.read") && m.contains("cannot restrict this yet")));
    }

    #[test]
    fn undeclared_capabilities_are_never_granted_even_if_written() {
        let t = tempfile::tempdir().unwrap();
        let mut p = policy(t.path(), Some("[apps.\"org.gimp.GIMP\"]\n\"files.documents\" = \"read-write\"\n"), None);
        let r = p.report(&gimp());
        assert_eq!(r.get(Key::Folder(KnownDir::Documents)).value, Value::NotRequested);
        assert!(p.set(&gimp(), Key::Folder(KnownDir::Documents), Value::ReadWrite).is_err());
        assert!(p.set(&gimp(), Key::ClipboardRead, Value::Deny).is_err());
        assert!(p.set(&gimp(), Key::Camera, Value::ReadOnly).is_err());
    }

    #[test]
    fn the_system_file_only_narrows() {
        let t = tempfile::tempdir().unwrap();
        let p = policy(
            t.path(),
            Some(
                "[apps.\"org.gimp.GIMP\"]\nnetwork = \"allow\"\n\"files.pictures\" = \"read-write\"\ncamera = \"allow\"\n",
            ),
            Some(
                "version = 1\n[all]\nnetwork = \"deny\"\ncamera = \"allow\"\n[apps.\"org.gimp.GIMP\"]\n\"files.pictures\" = \"read-only\"\n",
            ),
        );
        let r = p.report(&gimp());
        assert_eq!(r.get(Key::Network).value, Value::Deny);
        assert_eq!(r.get(Key::Network).decided_by, DecidedBy::System);
        assert_eq!(r.get(Key::Folder(KnownDir::Pictures)).value, Value::ReadOnly);
        // "allow" is not a limit: ignored, with a problem.
        assert_eq!(r.get(Key::Camera).value, Value::Allow);
        assert!(p.problems().iter().any(|x| x.message.contains("cannot be a limit")));
    }

    #[test]
    fn micro_vms_and_sound_are_reported_as_they_are() {
        let t = tempfile::tempdir().unwrap();
        let mut app = gimp();
        app.isolation = Some(IsolationTier::MicroVm);
        let p = policy(t.path(), Some("[apps.\"org.gimp.GIMP\"]\ncamera = \"allow\"\nsound = \"deny\"\n"), None);
        let r = p.report(&app);
        assert_eq!(r.get(Key::Camera).value, Value::Deny);
        assert_eq!(r.get(Key::Microphone).value, Value::NotRequested);
        assert_eq!(r.get(Key::OtherApps).value, Value::Deny, "passt keeps the VM off this computer's services");
        let r = p.report(&gimp());
        assert_eq!(r.get(Key::OtherApps).value, Value::Unrestricted, "a shared network reaches localhost");
    }

    #[test]
    fn render_then_parse_keeps_answers_and_explains_itself() {
        let t = tempfile::tempdir().unwrap();
        let mut p = policy(t.path(), None, None);
        p.set(&gimp(), Key::Folder(KnownDir::Pictures), Value::ReadOnly).unwrap();
        p.set(&gimp(), Key::Network, Value::Deny).unwrap();
        p.user.apps.entry("org.gone.App".into()).or_default().insert(Key::Camera, Value::Allow);
        let text = p.render(&[gimp(), AppFacts::unsandboxed("cosmic-term", "Terminal", Source::BuiltIn)]);
        assert!(text.starts_with("# What every app may use"));
        assert!(text.contains("\"files.pictures\""), "{text}");
        assert!(text.contains("NOT RESTRICTED YET"), "{text}");
        assert!(text.contains("worst_case = \"It could read your Pictures files"), "{text}");
        assert!(text.contains("not installed now"), "{text}");
        assert!(text.contains("all = \"unrestricted\""), "{text}");
        let back = parse(&text, "x", Layer::User);
        assert!(back.problems.is_empty(), "{:?}", back.problems);
        assert_eq!(back.apps["org.gimp.GIMP"][&Key::Folder(KnownDir::Pictures)], Value::ReadOnly);
        assert_eq!(back.apps["org.gimp.GIMP"][&Key::Network], Value::Deny);
        assert_eq!(back.apps["org.gone.App"][&Key::Camera], Value::Allow);
        assert_eq!(back.apps["org.gimp.GIMP"].get(&Key::Camera), Some(&Value::Ask));
    }

    #[test]
    fn search_extensions_are_listed_with_their_keyword_and_internet() {
        let t = tempfile::tempdir().unwrap();
        let mut p = policy(t.path(), None, None);
        let mut gh = AppFacts::from_declared("org.example.gh", "GitHub Search", true, &[]);
        gh.search = Some(crate::search::Extension {
            id: gh.id.clone(),
            title: "GitHub".into(),
            placeholder: None,
            keyword: "gh".into(),
            declared_keyword: "gh".into(),
            enabled: true,
            network: true,
            clashes_with: None,
            bundle: "/Applications/GitHub Search.app".into(),
        });
        let r = p.report(&gh);
        let s = r.search.clone().unwrap();
        assert!(s.active && s.internet, "{s:?}");
        assert!(r.summary.iter().any(|l| l.contains("after \"gh\" in Search")), "{:?}", r.summary);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["search"]["keyword"], "gh");
        assert_eq!(json["search"]["internet"], true);
        // The person turns its internet off: the card and the file say so.
        p.set(&gh, Key::Network, Value::Deny).unwrap();
        assert!(!p.report(&gh).search.unwrap().internet);
        let text = p.render(std::slice::from_ref(&gh));
        assert!(text.contains("search_extension = \"gh\"  # a Search extension: it sees only what you type after \"gh \", with no internet"), "{text}");
        let back = parse(&text, "x", Layer::User);
        assert!(back.problems.is_empty(), "{:?}", back.problems);
        assert_eq!(back.apps["org.example.gh"][&Key::Network], Value::Deny);
        // Waiting for a keyword.
        gh.search.as_mut().unwrap().clashes_with = Some("Other".into());
        let r = p.report(&gh).search.unwrap();
        assert!(!r.active && r.sentence().contains("waiting for a new keyword"));
        // Apps that are not extensions have no search part.
        assert!(p.report(&gimp()).search.is_none());
    }

    #[test]
    fn worst_case_is_generated_from_what_is_granted() {
        let t = tempfile::tempdir().unwrap();
        let p = policy(t.path(), Some("[apps.\"org.gimp.GIMP\"]\n\"files.pictures\" = \"read-write\"\n"), None);
        let r = p.report(&gimp());
        assert!(r.worst_case.contains("delete or change everything in your Pictures folder"), "{}", r.worst_case);
        assert!(r.worst_case.contains("send your Pictures files"), "{}", r.worst_case);
        let p = policy(
            t.path(),
            Some(
                "[apps.\"org.gimp.GIMP\"]\n\"files.pictures\" = \"read-only\"\nnetwork = \"deny\"\nsound = \"deny\"\n",
            ),
            None,
        );
        let r = p.report(&gimp());
        assert_eq!(
            r.worst_case,
            "It could read your Pictures files, but cannot change them or send them anywhere. Not restricted yet: it could read what you copy and capture your screen, though it has no internet to send anything out."
        );
        let preview = Policy::preview(&gimp());
        assert_eq!(preview.get(Key::Camera).value, Value::Allow);
        assert!(preview.summary.contains(&"Pictures (read-write)".to_owned()));
    }

    #[test]
    fn edit_locks_rereads_and_writes() {
        let t = tempfile::tempdir().unwrap();
        let mut a = policy(t.path(), None, None);
        let mut b = policy(t.path(), None, None);
        a.edit(&[gimp()], |p| p.set(&gimp(), Key::Camera, Value::Allow)).unwrap();
        // b's view is stale, but its edit starts from the file.
        b.edit(&[gimp()], |p| p.set(&gimp(), Key::Network, Value::Deny)).unwrap();
        let c = policy(t.path(), None, None);
        assert_eq!(c.answer("org.gimp.GIMP", Key::Camera), Some(Value::Allow));
        assert_eq!(c.answer("org.gimp.GIMP", Key::Network), Some(Value::Deny));
    }

    #[test]
    fn old_answers_are_brought_over() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join("old.toml");
        std::fs::write(&old, "[\"org.gimp.GIMP\"]\n\"files.pictures\" = \"allow\"\ncamera = \"deny\"\n").unwrap();
        let mut p = Policy::load_from(Some(t.path().join("new.toml")), t.path().join("sys"), Some(old), None, None);
        assert!(p.migrated);
        assert_eq!(p.answer("org.gimp.GIMP", Key::Folder(KnownDir::Pictures)), Some(Value::ReadWrite));
        assert_eq!(p.answer("org.gimp.GIMP", Key::Camera), Some(Value::Deny));
        p.save(&[gimp()]).unwrap();
        assert!(!p.migrated);
        assert!(t.path().join("new.toml").exists());
    }

    #[test]
    fn homer_control_is_in_the_permissions_file() {
        let t = tempfile::tempdir().unwrap();
        let p = policy(t.path(), None, None);
        assert!(p.homer_enabled());
        assert_eq!(p.screen_control(), Value::Ask);
        let r = p.report(&gimp());
        assert_eq!(r.get(Key::ControlledByHomer).value, Value::Ask);
        assert!(r.risk.iter().any(|l| l.contains("Homer will ask")), "{:?}", r.risk);

        let mut p = policy(
            t.path(),
            Some(
                "[homer]\ncontrol = true\nscreen = \"allow\"\n[apps.\"org.gimp.GIMP\"]\ncontrolled_by_homer = \"allow\"\n",
            ),
            None,
        );
        assert!(p.homer_enabled());
        assert_eq!(p.screen_control(), Value::Allow);
        assert!(p.set(&gimp(), Key::ControlledByHomer, Value::Deny).is_ok());
        p.set_homer_enabled(false);
        p.save(&[gimp()]).unwrap();
        let text = std::fs::read_to_string(t.path().join("user.toml")).unwrap();
        assert!(text.contains("[homer]"), "{text}");
        assert!(text.contains("control = false"), "{text}");
        assert!(text.contains("controlled_by_homer"), "{text}");
        let p = policy(t.path(), None, Some("[homer]\ncontrol = false\n"));
        assert!(!p.homer_enabled());
        assert!(p.homer_forced_off());
        let term = AppFacts::unsandboxed("cosmic-term", "Terminal", Source::BuiltIn);
        let mut p = policy(t.path(), None, None);
        assert!(p.set(&term, Key::ControlledByHomer, Value::Allow).is_ok());
        assert!(p.set(&term, Key::Network, Value::Deny).is_err());
        assert_eq!(p.report(&term).get(Key::ControlledByHomer).value, Value::Allow);
    }

    #[test]
    fn homer_reading_notifications_is_in_the_permissions_file() {
        let t = tempfile::tempdir().unwrap();
        let mut p = policy(t.path(), None, None);
        assert_eq!(p.homer_notifications(), Value::Ask);
        p.set_homer_notifications(Value::Allow);
        p.save(&[gimp()]).unwrap();
        let text = std::fs::read_to_string(t.path().join("user.toml")).unwrap();
        assert!(text.contains("notifications = \"allow\""), "{text}");
        let p = policy(t.path(), Some(text.as_str()), None);
        assert_eq!(p.homer_notifications(), Value::Allow);
        // Not tied to using other apps; the computer can only deny it.
        let p = policy(t.path(), Some(text.as_str()), Some("[homer]\ncontrol = false\n"));
        assert_eq!(p.homer_notifications(), Value::Allow);
        let p = policy(t.path(), Some(text.as_str()), Some("[homer]\nnotifications = \"deny\"\n"));
        assert_eq!(p.homer_notifications(), Value::Deny);
        let fresh = tempfile::tempdir().unwrap();
        let p = policy(fresh.path(), Some("[homer]\nnotifications = \"read-only\"\n"), None);
        assert_eq!(p.homer_notifications(), Value::Ask);
        assert!(!p.problems().is_empty());
    }
}
