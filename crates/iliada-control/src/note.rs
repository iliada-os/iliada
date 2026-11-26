//! `iliada-control note <title> <text>`: Text Editor's "Write a note"
//! intent. The note is a new file in ~/Documents/Notes, never over another.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A file name from a title: no slashes, no dot files, not too long.
pub fn file_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clean = clean.trim_start_matches(['.', ' ']).trim_end();
    let clean: String = clean.chars().take(80).collect();
    if clean.is_empty() { "Note".to_owned() } else { clean }
}

/// Writes the note; returns its path.
pub fn write(home: &Path, title: &str, text: &str) -> Result<PathBuf> {
    let dir = home.join("Documents/Notes");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let base = file_name(title);
    for n in 1..1000 {
        let name = if n == 1 { format!("{base}.txt") } else { format!("{base} {n}.txt") };
        let path = dir.join(name);
        // create_new: never over an existing note.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())?;
                if !text.ends_with('\n') {
                    f.write_all(b"\n")?;
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("too many notes called {base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_new_files_with_safe_names() {
        let t = tempfile::tempdir().unwrap();
        let n = file_name("../../.bashrc");
        assert!(!n.contains('/') && !n.starts_with('.'), "{n}");
        assert_eq!(file_name("Shopping / list"), "Shopping list");
        assert_eq!(file_name(""), "Note");
        let a = write(t.path(), "Shopping", "milk").unwrap();
        let b = write(t.path(), "Shopping", "eggs").unwrap();
        assert!(a.ends_with("Documents/Notes/Shopping.txt"));
        assert!(b.ends_with("Documents/Notes/Shopping 2.txt"));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "milk\n");
        let evil = write(t.path(), "../../../etc/x", "y").unwrap();
        assert!(evil.parent().unwrap().ends_with("Documents/Notes"));
    }
}
