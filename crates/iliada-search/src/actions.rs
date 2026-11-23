//! Doing what a result says, and the extra things a file result offers
//! (Reveal in Files, Copy Path, Open With).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::{Action, Hit, Kind};

/// Start a program on its own (its own process group, no output to us),
/// and reap it when it ends.
pub fn spawn(argv: &[String]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let (prog, args) = argv.split_first().context("nothing to run")?;
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting {prog}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The command that opens a path: `.app` bundles start in their sandbox,
/// anything else with its default app.
pub fn open_command(path: &Path) -> Vec<String> {
    let p = path.display().to_string();
    if path.extension().is_some_and(|e| e == "app") && path.is_dir() {
        vec!["iliada-run".into(), p]
    } else {
        vec!["xdg-open".into(), p]
    }
}

/// The command that shows `cmd` running in Terminal (and keeps it open).
pub fn terminal_command(cmd: &str) -> Vec<String> {
    let home = crate::home().map(|h| h.display().to_string()).unwrap_or_else(|| "/".into());
    vec![
        "cosmic-term".into(),
        "-w".into(),
        home,
        "--".into(),
        "sh".into(),
        "-c".into(),
        format!("{cmd}\nexec \"${{SHELL:-sh}}\""),
    ]
}

/// Do `action`. Copying needs the overlay's clipboard: this falls back to
/// `wl-copy`.
pub fn perform(action: &Action) -> Result<()> {
    match action {
        Action::Run(argv) => spawn(argv),
        Action::Open(path) => spawn(&open_command(path)),
        Action::Url(url) => spawn(&["xdg-open".into(), url.clone()]),
        Action::Copy(text) => spawn(&["wl-copy".into(), "--".into(), text.clone()]),
        Action::AskHomer(text) => spawn(&homer_command(text)),
        Action::Appearance(dark) => set_dark(*dark),
    }
}

/// Homer's quick panel, asked `text` at once.
pub fn homer_command(text: &str) -> Vec<String> {
    let mut v = vec!["iliada-assistant-app".into(), "--panel".into()];
    if !text.trim().is_empty() {
        v.push("--ask".into());
        v.push(text.trim().to_owned());
    }
    v
}

/// Dark or light, as Settings → Appearance sets it (COSMIC follows its
/// settings files at once).
pub fn set_dark(dark: bool) -> Result<()> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| crate::home().map(|h| h.join(".config")))
        .context("HOME is not set")?;
    let dir = base.join("cosmic/com.system76.CosmicTheme.Mode/v1");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("is_dark"), if dark { "true" } else { "false" })?;
    Ok(())
}

/// The extra actions for a result (shown with Tab or the arrow on its row).
pub fn extras(hit: &Hit) -> Vec<Hit> {
    let Some(path) = hit.path() else { return Vec::new() };
    let parent = path.parent().unwrap_or(path);
    let p = path.display().to_string();
    let mut v = vec![
        Hit::new(Kind::Action, "Open", Action::Open(path.to_owned())).icon_name("document-open-symbolic"),
        Hit::new(
            Kind::Action,
            "Reveal in Files",
            Action::Run(vec!["cosmic-files".into(), parent.display().to_string()]),
        )
        .subtitle(crate::tilde(parent))
        .icon_name("system-file-manager-symbolic"),
        Hit::new(Kind::Action, "Copy Path", Action::Copy(p.clone()))
            .subtitle(p.clone())
            .icon_name("edit-copy-symbolic"),
    ];
    if hit.kind == Kind::Folder {
        v.push(
            Hit::new(Kind::Action, "Open in Terminal", Action::Run(vec!["cosmic-term".into(), "-w".into(), p.clone()]))
                .icon_name("utilities-terminal-symbolic"),
        );
    }
    v.push(
        Hit::new(Kind::Action, "Ask Homer About It", Action::AskHomer(format!("About the file {p}: ")))
            .icon_name("org.iliada.Assistant"),
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn commands() {
        assert_eq!(open_command(Path::new("/home/a/x.pdf")), vec!["xdg-open", "/home/a/x.pdf"]);
        let t = terminal_command("ls -l");
        assert_eq!(&t[..1], &["cosmic-term".to_owned()]);
        assert!(t.last().unwrap().starts_with("ls -l\n"));
        assert_eq!(homer_command(" hi "), vec!["iliada-assistant-app", "--panel", "--ask", "hi"]);
        assert_eq!(homer_command(""), vec!["iliada-assistant-app", "--panel"]);
    }

    #[test]
    fn file_extras() {
        let h = Hit::new(Kind::File, "x.pdf", Action::Open(PathBuf::from("/home/a/Docs/x.pdf")));
        let titles: Vec<_> = extras(&h).into_iter().map(|h| h.title).collect();
        assert_eq!(titles, vec!["Open", "Reveal in Files", "Copy Path", "Ask Homer About It"]);
        assert!(extras(&Hit::new(Kind::Answer, "= 4", Action::Copy("4".into()))).is_empty());
    }
}
