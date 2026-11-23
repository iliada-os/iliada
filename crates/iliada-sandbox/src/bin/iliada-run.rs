//! `iliada-run`: start an app from /Applications inside its sandbox.
//!
//!     iliada-run /Applications/Calculator.app
//!     iliada-run org.debian.jq -- --version
//!
//! This is what desktop entries exported by `iliada-appd` execute, so apps
//! are never started outside a sandbox.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use iliada_bundle::policy::{self, AppFacts, Key, Policy, Value};
use iliada_bundle::{Bundle, IsolationTier};
use iliada_sandbox::cloud_hypervisor::CloudHypervisorBackend;
use iliada_sandbox::{Grants, HostIsolationConfig, NetworkPolicy, SandboxSpec, SoundSockets, backend_for};
use iliada_types::KnownDir;

const APPLICATIONS: &str = "/Applications";

fn usage() -> ! {
    eprintln!("usage: iliada-run [--config <isolation.json>] <bundle.app | app-id> [args...]");
    std::process::exit(2);
}

fn find_bundle(target: &str) -> Result<Bundle> {
    if Path::new(target).exists() {
        return Ok(Bundle::open(target)?);
    }
    for e in std::fs::read_dir(APPLICATIONS).with_context(|| format!("reading {APPLICATIONS}"))?.flatten() {
        if let Ok(b) = Bundle::open(e.path())
            && b.info.id.as_str() == target
        {
            return Ok(b);
        }
    }
    bail!("no bundle {target:?} (not a path, and no app with that id in {APPLICATIONS})")
}

/// Per-user private storage, like macOS `~/Library/Containers/<id>`.
fn data_dir(app: &str) -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".local/share"),
    };
    Ok(base.join("iliada/apps").join(app))
}

fn wayland_socket() -> Option<PathBuf> {
    let display = std::env::var_os("WAYLAND_DISPLAY")?;
    let path = PathBuf::from(display);
    let path = if path.is_absolute() { path } else { PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(path) };
    path.exists().then_some(path)
}

/// The user asked for this app to run in its own micro-VM (Settings →
/// Privacy → Extra isolation): `~/.config/iliada/extra-isolation` lists app ids.
fn extra_isolation(app: &str) -> bool {
    let Some(home) = home() else { return false };
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(cfg.join("iliada/extra-isolation"))
        .map(|t| t.lines().any(|l| l.trim() == app))
        .unwrap_or(false)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| Some(home()?.join(".config")))
}

fn sound_sockets() -> Option<SoundSockets> {
    let run = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let (pw, pulse) = (run.join("pipewire-0"), run.join("pulse/native"));
    let s = SoundSockets { pipewire: pw.exists().then_some(pw), pulse: pulse.exists().then_some(pulse) };
    (s.pipewire.is_some() || s.pulse.is_some()).then_some(s)
}

/// What the prompt says the app wants to do.
fn wants(key: Key) -> String {
    match key {
        Key::Folder(d) => format!("access your {} folder", d.name()),
        Key::Network => "use the internet".into(),
        Key::Sound => "play sound".into(),
        other => format!("use your {}", other.label().to_lowercase()),
    }
}

/// Ask the user, in a window Iliada draws (never the app). `None`: no
/// answer (closed, or no display): denied this time, asked again next time.
fn ask(bundle: &Bundle, key: Key, purpose: &str) -> Option<bool> {
    // Automated tests (no one to click): ILIADA_ASSUME=allow|deny answers
    // questions not decided yet, and is not remembered.
    match std::env::var("ILIADA_ASSUME").as_deref() {
        Ok("allow") => return Some(true),
        Ok("deny") => return Some(false),
        _ => {}
    }
    let mut cmd = std::process::Command::new("iliada-prompt");
    cmd.args(["--app", &bundle.info.name, "--wants", &wants(key), "--purpose", purpose]);
    if let Some(icon) = &bundle.info.icon {
        cmd.arg("--icon").arg(bundle.layout.contents().join(icon));
    }
    match cmd.status().ok()?.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// How `bundle` is isolated: its own setting, or the person's Extra
/// isolation for it, or High security mode for every app.
fn tier_for(bundle: &Bundle, host: &HostIsolationConfig) -> IsolationTier {
    if host.high_security || extra_isolation(bundle.info.id.as_str()) {
        IsolationTier::MicroVm
    } else {
        bundle.info.isolation
    }
}

/// Tell the person, in a window, that their permissions file has a mistake.
fn notice(bundle: &Bundle, problems: &[policy::Problem]) {
    let first = problems.first().map(ToString::to_string).unwrap_or_default();
    eprintln!("iliada-run: {first}");
    if wayland_socket().is_none() {
        return;
    }
    let text = format!(
        "Your permissions file has a mistake, so {} opens without your folders, camera, microphone, sound or internet until it is fixed. {first} Settings > Privacy can reset it.",
        bundle.info.name
    );
    let mut cmd = std::process::Command::new("iliada-prompt");
    cmd.args(["--app", &bundle.info.name, "--notice", &text]);
    let _ = cmd.spawn();
}

/// What this app may use (`~/.config/iliada/permissions.toml` and the
/// computer's limits in `/etc/iliada/permissions.toml`), asking now about
/// anything not decided yet, and the network it gets.
fn grants(bundle: &Bundle, tier: IsolationTier) -> (Grants, NetworkPolicy) {
    let facts = AppFacts::from_bundle(bundle, tier);
    let mut policy = Policy::load();
    if !policy.readable() {
        notice(bundle, &policy.problems());
    }
    let mut answers = Vec::new();
    for key in policy.report(&facts).asks {
        let purpose = facts.declared.get(&key).cloned().unwrap_or_default();
        if let Some(allow) = ask(bundle, key, &purpose) {
            let yes = if key.folder().is_some() { Value::ReadWrite } else { Value::Allow };
            answers.push((key, if allow { yes } else { Value::Deny }));
        }
    }
    if std::env::var_os("ILIADA_ASSUME").is_some() {
        // Not remembered.
        for (k, v) in &answers {
            let _ = policy.set(&facts, *k, *v);
        }
    } else if policy.readable() && (policy.user.problems.is_empty() || !answers.is_empty()) {
        // Remember the answers, and keep the file's summaries current.
        let apps = policy::installed_programs();
        let result = policy.edit(&apps, |p| {
            for (k, v) in &answers {
                p.set(&facts, *k, *v)?;
            }
            Ok(())
        });
        if let Err(e) = result {
            eprintln!("iliada-run: could not save the permissions file: {e}");
            for (k, v) in &answers {
                let _ = policy.set(&facts, *k, *v);
            }
        }
    }

    let report = policy.report(&facts);
    let mut g = Grants::default();
    let (home, cfg) = (home(), config_home());
    for d in KnownDir::ALL {
        let dir = match (&home, &cfg) {
            (Some(h), Some(c)) => policy::user_dir(h, c, d),
            _ => None,
        };
        let Some(dir) = dir else { continue };
        let list = match report.get(Key::Folder(d)).value {
            Value::ReadWrite => &mut g.folders,
            Value::ReadOnly => &mut g.read_only_folders,
            _ => continue,
        };
        let _ = std::fs::create_dir_all(&dir);
        list.push((dir, d.name().to_owned()));
    }
    g.camera = report.get(Key::Camera).value == Value::Allow;
    // Playing and recording go through the same sound server.
    if report.get(Key::Sound).value == Value::Allow || report.get(Key::Microphone).value == Value::Allow {
        g.sound = sound_sockets();
    }
    let network = if report.get(Key::Network).value == Value::Allow { NetworkPolicy::Nat } else { NetworkPolicy::None };
    (g, network)
}

/// Arguments that are files the user opened (absolute paths, or file://
/// URLs, that exist): the app gets exactly those. System paths never.
fn opened_files(args: &[String]) -> Vec<PathBuf> {
    args.iter()
        .filter_map(|a| {
            let p = PathBuf::from(a.strip_prefix("file://").unwrap_or(a));
            let system = ["/nix", "/proc", "/sys", "/dev", "/run", "/etc", "/boot", "/Applications"];
            (p.is_absolute() && p.exists() && !system.iter().any(|s| p.starts_with(s))).then_some(p)
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = PathBuf::from(HostIsolationConfig::DEFAULT_PATH);
    if args.first().map(String::as_str) == Some("--config") {
        if args.len() < 2 {
            usage();
        }
        config = PathBuf::from(args.remove(1));
        args.remove(0);
    }
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        usage();
    }
    let target = args.remove(0);
    if args.first().map(String::as_str) == Some("--") {
        args.remove(0);
    }

    let host = HostIsolationConfig::load(&config)?;
    let bundle = find_bundle(&target)?;
    if let Some(why) = bundle.wrong_processor() {
        // Started from the desktop there is no terminal: say it in a window.
        if wayland_socket().is_some() {
            let mut cmd = std::process::Command::new("iliada-prompt");
            cmd.args(["--app", &bundle.info.name, "--notice", &why]);
            if let Some(icon) = &bundle.info.icon {
                cmd.arg("--icon").arg(bundle.layout.contents().join(icon));
            }
            let _ = cmd.status();
        }
        bail!("{why}");
    }
    // The app's own setting (or the user's "Extra isolation" for it), unless
    // High security mode puts every app in a VM.
    let mut tier = tier_for(&bundle, &host);
    if tier == IsolationTier::MicroVm && !CloudHypervisorBackend::available(&host) {
        eprintln!(
            "iliada-run: micro-VMs are not available on this computer; running {} in its sandbox",
            bundle.info.id
        );
        tier = IsolationTier::Namespace;
    }
    // Only apps that declare a network (shown in agora before installing)
    // get one, unless the person or the computer's limits say no.
    let (granted, network) = grants(&bundle, tier);
    let data = data_dir(bundle.info.id.as_str())?;
    // The usual folders of a home, so apps that create only the last level
    // (gnome-sudoku's ~/.local/share/gnome-sudoku) can save.
    for d in [".config", ".cache", ".local/share", ".local/state"] {
        let _ = std::fs::create_dir_all(data.join(d));
    }
    let mut spec = SandboxSpec::from_bundle(&bundle, &host, network, data)?;
    spec.display = wayland_socket();
    spec.files = opened_files(&args);
    // file:// URLs become plain paths (the file is at the same path inside).
    let args: Vec<String> =
        args.into_iter().map(|a| a.strip_prefix("file://").map(str::to_owned).unwrap_or(a)).collect();
    spec.args = bundle.info.args.iter().cloned().chain(args).collect();
    spec.grants = granted;
    spec.env.extend(bundle.info.env.iter().map(|(k, v)| (k.clone(), v.clone())));

    let backend = backend_for(tier, &host);
    let mut handle = backend.launch(&spec).await?;
    let code = handle.wait().await?.unwrap_or(1);
    std::process::exit(code);
}
