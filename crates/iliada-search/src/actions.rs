//! Doing what a result says, and the extra things a file result offers
//! (Reveal in Files, Copy Path, Open With, Move to).

use std::path::{Path, PathBuf};
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
        Action::OpenAll(paths) => {
            let mut first_err = None;
            for path in paths {
                if let Err(e) = spawn(&open_command(path)) {
                    first_err = first_err.or(Some(e));
                }
            }
            first_err.map_or(Ok(()), Err)
        }
        Action::Url(url) => spawn(&["xdg-open".into(), url.clone()]),
        Action::Copy(text) => spawn(&["wl-copy".into(), "--".into(), text.clone()]),
        Action::AskHomer(text) => spawn(&homer_command(text)),
        Action::Appearance(dark) => set_dark(*dark),
        Action::RestoreClipboard { id, paste } => restore_clipboard(*id, *paste),
        Action::FocusWindow { identifier } => focus_window(identifier),
        Action::ClipboardPin { .. } | Action::ClipboardDelete(_) | Action::ClipboardClear | Action::MoveTo(_) => {
            Ok(()) // the overlay talks to the search service, or handles these itself
        }
    }
}

fn restore_clipboard(id: u64, paste: bool) -> Result<()> {
    let mut c = crate::proto::Client::connect().context("the search service is not running")?;
    c.call(&crate::Request::ClipboardRestore { id }).context("putting the copy back on the clipboard")?;
    if paste {
        crate::clipboard::paste()?;
    }
    Ok(())
}

/// Only the search service knows the open windows (and may switch to one).
pub fn focus_window(identifier: &str) -> Result<()> {
    let mut c = crate::proto::Client::connect().context("the search service is not running")?;
    match c.call(&crate::Request::FocusWindow { identifier: identifier.to_owned() })? {
        crate::Response::Error { message } => anyhow::bail!(message),
        _ => Ok(()),
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
        Hit::new(Kind::Action, "Move to", Action::MoveTo(path.to_owned()))
            .subtitle("Choose a folder")
            .icon_name("folder-symbolic"),
    );
    v.push(
        Hit::new(Kind::Action, "Ask Homer About It", Action::AskHomer(format!("About the file {p}: ")))
            .icon_name("org.iliada.Assistant"),
    );
    v
}

/// Move `src` into `dest_dir`, keeping its name.
pub fn move_into(src: &Path, dest_dir: &Path) -> Result<PathBuf> {
    anyhow::ensure!(dest_dir.is_dir(), "{} is not a folder", dest_dir.display());
    let name = src.file_name().context("the path has no file name")?;
    let dest = dest_dir.join(name);
    anyhow::ensure!(!dest.exists(), "{} already exists", dest.display());
    std::fs::rename(src, &dest).with_context(|| format!("moving {} to {}", src.display(), dest.display()))?;
    Ok(dest)
}

/// Paths from a `text/uri-list` drop (or a plain path). Comments and empty
/// lines are ignored. `file://` URLs are decoded.
pub fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(data);
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let s = l.strip_prefix("file://").unwrap_or(l);
            let decoded = decode_url(s);
            if decoded.is_empty() { None } else { Some(PathBuf::from(decoded)) }
        })
        .collect()
}

fn decode_url(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn file_url(path: &Path) -> String {
    let s = path.display().to_string();
    let mut out = String::from("file://");
    for b in s.bytes() {
        if should_percent_encode(b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

fn should_percent_encode(b: u8) -> bool {
    matches!(b, b'%' | b' ' | b'#' | b'?' | 0..=0x1f | 0x7f..=0xff)
}

/// The `text/uri-list` body for these paths (for dragging out of Search).
pub fn uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| file_url(p)).collect::<Vec<_>>().join("\r\n")
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
        assert_eq!(titles, vec!["Open", "Reveal in Files", "Copy Path", "Move to", "Ask Homer About It"]);
        assert!(extras(&Hit::new(Kind::Answer, "= 4", Action::Copy("4".into()))).is_empty());
    }

    #[test]
    fn uri_lists_become_paths() {
        let data = b"# comment\r\nfile:///home/a/Notes.txt\r\nfile:///tmp/b%20c.png\n/absolute/plain\n";
        assert_eq!(
            parse_uri_list(data),
            [PathBuf::from("/home/a/Notes.txt"), PathBuf::from("/tmp/b c.png"), PathBuf::from("/absolute/plain")]
        );
        assert!(parse_uri_list(b"").is_empty());
        let round = parse_uri_list(uri_list(&[PathBuf::from("/tmp/b c.png")]).as_bytes());
        assert_eq!(round, [PathBuf::from("/tmp/b c.png")]);
    }

    #[test]
    fn move_into_a_folder() {
        let root = std::env::temp_dir().join(format!("iliada-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dest")).unwrap();
        let src = root.join("note.txt");
        std::fs::write(&src, "hi").unwrap();
        let dest = move_into(&src, &root.join("dest")).unwrap();
        assert_eq!(dest, root.join("dest/note.txt"));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hi");
        assert!(!src.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
