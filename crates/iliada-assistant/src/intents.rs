//! App intents: actions an app declares for Homer ("open a folder", "new
//! note", "search agora"), exposed to the model as tools of their own.
//! Exact and fast where driving the app's window would be slow and
//! fragile, like App Intents on macOS. The format is in docs/app-intents.md.
//!
//! Where they come from:
//! * an app bundle's `Contents/Intents.toml`: run inside the app's sandbox
//!   (`iliada-run <bundle> -- <arguments>`), so an intent can do no more
//!   than the app itself;
//! * `share/iliada/intents/*.toml` in the system (Iliada's own apps, from
//!   Nix): a program and its arguments, or a D-Bus call.
//!
//! Arguments are never given to a shell: each parameter becomes exactly
//! the argument it stands for.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentsFile {
    /// The app the intents belong to (system files; a bundle's are its own).
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "intent")]
    pub intents: Vec<IntentDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentDef {
    /// Lower case, digits and `_`: `open_folder`.
    pub name: String,
    /// How it reads to the person: "Open a folder".
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// `send`, `submit`, `delete`, `purchase` or `post`: always shown to the
    /// person first, in Auto mode too.
    #[serde(default)]
    pub important: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
    /// Program and arguments (system intents) or the app's arguments
    /// (bundles); `{param}` stands for a parameter's value.
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub dbus: Option<DbusCall>,
    /// Wait for it to finish and pass its output back (otherwise it is
    /// started and left running, like opening a window).
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// `string`, `integer`, `boolean`, `path` (inside the home folder) or
    /// `enum` (one of `values`).
    #[serde(rename = "type", default = "string")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub values: Vec<String>,
}

fn string() -> String {
    "string".into()
}

/// A method on the session bus; arguments are strings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbusCall {
    pub service: String,
    pub path: String,
    pub interface: String,
    pub method: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One intent, ready to offer.
#[derive(Debug, Clone)]
pub struct Intent {
    /// The tool's name: `intent_<app>_<name>`.
    pub tool: String,
    pub app_id: String,
    pub app_name: String,
    /// Run in this bundle's sandbox.
    pub bundle: Option<PathBuf>,
    pub def: IntentDef,
}

fn plain(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `com.system76.CosmicFiles` → `cosmicfiles`; `org.debian.google-chrome` →
/// `google_chrome`.
fn short(app_id: &str) -> String {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    last.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// Checks one file's intents; the bad ones are left out (with why).
pub fn check(file: &IntentsFile, bundle: bool) -> (Vec<IntentDef>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for d in &file.intents {
        let placeholders: Vec<String> = d
            .run
            .iter()
            .chain(d.dbus.iter().flat_map(|c| c.args.iter()))
            .flat_map(|a| a.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_owned())))
            .collect();
        let why = if !plain(&d.name) {
            Some("its name must be lower case letters, digits and _".to_owned())
        } else if d.run.is_empty() == d.dbus.is_none() {
            Some("it needs exactly one of run or dbus".to_owned())
        } else if bundle && d.dbus.is_some() {
            Some("an app's own intents run in its sandbox: use run".to_owned())
        } else if let Some(p) = d.params.keys().find(|p| !plain(p)) {
            Some(format!("parameter {p:?} must be lower case letters, digits and _"))
        } else if let Some(p) = placeholders.iter().find(|p| !d.params.contains_key(*p)) {
            Some(format!("{{{p}}} is not one of its parameters"))
        } else if let Some((n, _)) = d.params.iter().find(|(_, p)| {
            !["string", "integer", "boolean", "path", "enum"].contains(&p.kind.as_str())
                || (p.kind == "enum" && p.values.is_empty())
        }) {
            Some(format!("parameter {n:?} has an unknown type (or an enum without values)"))
        } else if d.important.as_deref().is_some_and(|k| !["send", "submit", "delete", "purchase", "post"].contains(&k))
        {
            Some("important must be send, submit, delete, purchase or post".to_owned())
        } else {
            None
        };
        match why {
            None => ok.push(d.clone()),
            Some(w) => bad.push(format!("{}: {w}", d.name)),
        }
    }
    (ok, bad)
}

fn add(out: &mut Vec<Intent>, app_id: &str, app_name: &str, bundle: Option<PathBuf>, defs: Vec<IntentDef>) {
    for def in defs {
        let mut tool: String = format!("intent_{}_{}", short(app_id), def.name).chars().take(60).collect();
        while out.iter().any(|i| i.tool == tool) {
            tool.push('_');
        }
        out.push(Intent {
            tool,
            app_id: app_id.to_owned(),
            app_name: app_name.to_owned(),
            bundle: bundle.clone(),
            def,
        });
    }
}

/// Where Iliada's own apps' intents are installed.
pub fn system_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/run/current-system/sw/share/iliada/intents")];
    if let Ok(x) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(x.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("iliada/intents")));
    }
    dirs.dedup();
    dirs
}

/// Every intent on this computer: the system's, then installed apps'.
pub fn load() -> Vec<Intent> {
    load_from(&system_dirs(), Path::new("/Applications"))
}

pub fn load_from(system: &[PathBuf], applications: &Path) -> Vec<Intent> {
    let mut out = Vec::new();
    let mut seen_files = Vec::new();
    for dir in system {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for f in files {
            let Some(name) = f.file_name().map(|n| n.to_owned()) else { continue };
            if seen_files.contains(&name) {
                continue;
            }
            seen_files.push(name);
            let Ok(file) = std::fs::read_to_string(&f)
                .map_err(anyhow::Error::from)
                .and_then(|t| Ok(toml::from_str::<IntentsFile>(&t)?))
            else {
                tracing::warn!(file = %f.display(), "unreadable intents file");
                continue;
            };
            let Some(app) = file.app.clone() else { continue };
            let (ok, bad) = check(&file, false);
            for b in bad {
                tracing::warn!(file = %f.display(), "intent left out: {b}");
            }
            add(&mut out, &app, file.name.as_deref().unwrap_or(&app), None, ok);
        }
    }
    let mut bundles: Vec<PathBuf> =
        std::fs::read_dir(applications).into_iter().flatten().flatten().map(|e| e.path()).collect();
    bundles.sort();
    for root in bundles {
        let path = root.join("Contents/Intents.toml");
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(b) = iliada_bundle::Bundle::open(&root) else { continue };
        let Ok(file) = toml::from_str::<IntentsFile>(&text) else {
            tracing::warn!(file = %path.display(), "unreadable intents file");
            continue;
        };
        let (ok, _) = check(&file, true);
        add(&mut out, b.info.id.as_str(), &b.info.name, Some(root.clone()), ok);
    }
    out
}

/// The tools the model sees.
pub fn definitions(intents: &[Intent]) -> Vec<Value> {
    intents
        .iter()
        .map(|i| {
            let mut props = serde_json::Map::new();
            let mut required = Vec::new();
            for (n, p) in &i.def.params {
                let mut s = match p.kind.as_str() {
                    "integer" => json!({ "type": "integer" }),
                    "boolean" => json!({ "type": "boolean" }),
                    "enum" => json!({ "type": "string", "enum": p.values }),
                    _ => json!({ "type": "string" }),
                };
                let desc = if p.kind == "path" && !p.description.contains("home") {
                    format!("{} (a path in the home folder, ~ works)", p.description).trim().to_owned()
                } else {
                    p.description.clone()
                };
                if !desc.is_empty() {
                    s["description"] = json!(desc);
                }
                props.insert(n.clone(), s);
                if p.required {
                    required.push(n.clone());
                }
            }
            let mut description = format!("{} ({}).", i.def.title, i.app_name);
            if !i.def.description.is_empty() {
                description.push(' ');
                description.push_str(&i.def.description);
            }
            json!({ "name": i.tool, "description": description,
                    "input_schema": { "type": "object", "properties": props, "required": required } })
        })
        .collect()
}

pub fn find<'a>(intents: &'a [Intent], tool: &str) -> Option<&'a Intent> {
    intents.iter().find(|i| i.tool == tool)
}

/// How an intent reads to the person: "Files: Open a folder (path: ~/Music)".
pub fn describe(i: &Intent, input: &Value) -> String {
    let args: Vec<String> = i
        .def
        .params
        .keys()
        .filter_map(|k| match &input[k] {
            Value::Null => None,
            Value::String(s) => Some(format!("{k}: {s}")),
            v => Some(format!("{k}: {v}")),
        })
        .collect();
    let mut s = format!("{}: {}", i.app_name, i.def.title);
    if !args.is_empty() {
        s.push_str(&format!(" ({})", args.join(", ")));
    }
    s
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// A parameter's value as one argument.
fn value(name: &str, p: &Param, v: &Value, home: Option<&Path>) -> Result<Option<String>> {
    if v.is_null() {
        anyhow::ensure!(!p.required, "{name} is needed");
        return Ok(None);
    }
    let text = match p.kind.as_str() {
        "integer" => v.as_i64().map(|n| n.to_string()).with_context(|| format!("{name} must be a whole number"))?,
        "boolean" => v.as_bool().map(|b| b.to_string()).with_context(|| format!("{name} must be true or false"))?,
        "enum" => {
            let s = v.as_str().unwrap_or("");
            anyhow::ensure!(p.values.iter().any(|x| x == s), "{name} must be one of {}", p.values.join(", "));
            s.to_owned()
        }
        "path" => {
            let s = v.as_str().with_context(|| format!("{name} must be a path"))?;
            let home = home.context("HOME is not set")?;
            let raw = if s == "~" {
                home.to_path_buf()
            } else if let Some(rest) = s.strip_prefix("~/") {
                home.join(rest)
            } else if Path::new(s).is_absolute() {
                PathBuf::from(s)
            } else {
                home.join(s)
            };
            // Resolved (symlinks, ..) before the check.
            let real = raw.canonicalize().with_context(|| format!("{} does not exist", raw.display()))?;
            let home_real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
            anyhow::ensure!(real.starts_with(&home_real), "{name} must be inside the home folder");
            real.display().to_string()
        }
        _ => v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()),
    };
    Ok(Some(text))
}

/// The arguments with the parameters put in.
pub fn arguments(def: &IntentDef, template: &[String], input: &Value, home: Option<&Path>) -> Result<Vec<String>> {
    let mut values = BTreeMap::new();
    for (n, p) in &def.params {
        if let Some(v) = value(n, p, &input[n.as_str()], home)? {
            values.insert(n.clone(), v);
        }
    }
    let mut out = Vec::new();
    for a in template {
        // A missing optional parameter that is a whole argument: left out.
        if let Some(n) = a.strip_prefix('{').and_then(|r| r.strip_suffix('}'))
            && def.params.contains_key(n)
        {
            match values.get(n) {
                Some(v) => {
                    // A value can't turn into an option of the program.
                    anyhow::ensure!(!v.starts_with('-'), "{n} cannot start with \"-\"");
                    out.push(v.clone());
                }
                None => continue,
            }
            continue;
        }
        let mut s = a.clone();
        for (n, v) in &values {
            s = s.replace(&format!("{{{n}}}"), v);
        }
        anyhow::ensure!(
            !s.contains('{') || !def.params.keys().any(|n| s.contains(&format!("{{{n}}}"))),
            "a value is missing in {a:?}"
        );
        out.push(s);
    }
    Ok(out)
}

/// Runs an intent; the result is text for the model.
pub async fn run(i: &Intent, input: &Value) -> Result<String> {
    let policy = iliada_bundle::control::ControlPolicy::load_user();
    let system = iliada_bundle::control::SystemControl::load();
    if policy.decide(&system, &i.app_id) == iliada_bundle::control::Control::Deny {
        bail!("the person has not let Homer use {} (Settings → Homer → Use other apps)", i.app_name);
    }
    let home = home();
    let (program, args): (String, Vec<String>) = if let Some(call) = &i.def.dbus {
        let args = arguments(&i.def, &call.args, input, home.as_deref())?;
        let mut a = vec![
            "--user".to_owned(),
            "call".into(),
            call.service.clone(),
            call.path.clone(),
            call.interface.clone(),
            call.method.clone(),
        ];
        if !args.is_empty() {
            a.push("s".repeat(args.len()));
            a.extend(args);
        }
        ("busctl".into(), a)
    } else if let Some(bundle) = &i.bundle {
        let mut a = vec![bundle.display().to_string(), "--".into()];
        a.extend(arguments(&i.def, &i.def.run, input, home.as_deref())?);
        ("iliada-run".into(), a)
    } else {
        let all = arguments(&i.def, &i.def.run, input, home.as_deref())?;
        let (p, rest) = all.split_first().context("nothing to run")?;
        (p.clone(), rest.to_vec())
    };
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args).stdin(std::process::Stdio::null());
    if let Some(h) = &home {
        cmd.current_dir(h);
    }
    if i.def.wait || i.def.dbus.is_some() {
        let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
            .await
            .context("it took longer than a minute")??;
        let text: String = String::from_utf8_lossy(&out.stdout).chars().take(4000).collect();
        if !out.status.success() {
            bail!("{} failed: {}", i.def.title, String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(json!({ "done": i.def.title, "output": text }).to_string())
    } else {
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().with_context(|| format!("starting {program}"))?;
        Ok(json!({ "done": i.def.title }).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: &str = r#"
app = "com.system76.CosmicFiles"
name = "Files"

[[intent]]
name = "open_folder"
title = "Open a folder"
run = ["cosmic-files", "{path}"]
[intent.params.path]
type = "path"
required = true

[[intent]]
name = "Bad Name"
title = "x"
run = ["true"]

[[intent]]
name = "two_ways"
title = "x"
run = ["true"]
dbus = { service = "a.b", path = "/", interface = "a.b", method = "C" }

[[intent]]
name = "unknown_param"
title = "x"
run = ["echo", "{nope}"]
"#;

    #[test]
    fn files_are_checked_and_become_tools() {
        let t = tempfile::tempdir().unwrap();
        let sys = t.path().join("intents");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("files.toml"), FILES).unwrap();
        let file: IntentsFile = toml::from_str(FILES).unwrap();
        let (ok, bad) = check(&file, false);
        assert_eq!(ok.len(), 1);
        assert_eq!(bad.len(), 3, "{bad:?}");
        let all = load_from(&[sys], &t.path().join("Applications"));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tool, "intent_cosmicfiles_open_folder");
        let defs = definitions(&all);
        assert_eq!(defs[0]["input_schema"]["required"], json!(["path"]));
        assert!(defs[0]["description"].as_str().unwrap().starts_with("Open a folder (Files)."));
        assert_eq!(describe(&all[0], &json!({ "path": "~/Music" })), "Files: Open a folder (path: ~/Music)");
    }

    #[test]
    fn iliadas_own_intents_are_all_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../nix/intents");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let path = e.unwrap().path();
            let file: IntentsFile = toml::from_str(&std::fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let (ok, bad) = check(&file, false);
            assert!(bad.is_empty(), "{}: {bad:?}", path.display());
            assert!(file.app.is_some());
            n += ok.len();
        }
        assert!(n >= 6);
        let all = load_from(&[dir], Path::new("/nonexistent"));
        let names: Vec<&str> = all.iter().map(|i| i.tool.as_str()).collect();
        assert!(names.contains(&"intent_cosmicfiles_open_folder"), "{names:?}");
        assert!(names.contains(&"intent_agora_search"), "{names:?}");
    }

    #[test]
    fn values_become_single_arguments_and_paths_stay_home() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        std::fs::create_dir_all(home.join("Music")).unwrap();
        let file: IntentsFile = toml::from_str(
            r#"app = "x"
[[intent]]
name = "note"
title = "New note"
run = ["notes", "--title={title}", "{text}", "{folder}"]
[intent.params.title]
required = true
[intent.params.text]
[intent.params.folder]
type = "path"
"#,
        )
        .unwrap();
        let d = &file.intents[0];
        let h = Some(home.as_path());
        let args = arguments(d, &d.run, &json!({ "title": "a b; rm -rf ~", "text": "hi $(x)" }), h).unwrap();
        assert_eq!(args, ["notes", "--title=a b; rm -rf ~", "hi $(x)"]);
        let real_music = home.join("Music").canonicalize().unwrap().display().to_string();
        let args = arguments(d, &d.run, &json!({ "title": "t", "folder": "~/Music" }), h).unwrap();
        assert_eq!(args, ["notes", "--title=t", real_music.as_str()]);
        assert!(arguments(d, &d.run, &json!({ "title": "t", "folder": "/etc" }), h).is_err());
        assert!(arguments(d, &d.run, &json!({ "title": "t", "folder": "~/../../etc" }), h).is_err());
        assert!(arguments(d, &d.run, &json!({ "text": "x" }), h).is_err(), "title is required");
        assert!(arguments(d, &d.run, &json!({ "title": "t", "text": "--help" }), h).is_err(), "no options smuggled in");
    }
}
