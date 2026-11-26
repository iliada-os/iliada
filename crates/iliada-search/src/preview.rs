//! Quick Look: a fast, lazy preview of a search result (text, images, the
//! first page of a PDF, a folder's contents, or an app's details).
//!
//! Reading is capped so the search overlay never waits on a large file.
//! PDF pages use `pdftoppm` when it is on the path; otherwise the preview
//! says it is a PDF and stops there.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::clipboard::{ClipKind, Item};
use crate::{Hit, tilde};

/// How much of a text file is shown.
pub const MAX_TEXT: usize = 64 * 1024;
/// Images and PDF renders larger than this are not opened.
pub const MAX_IMAGE: u64 = 20 * 1024 * 1024;
/// Folder rows shown.
pub const MAX_FOLDER: usize = 40;

/// What kind of preview this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Text,
    Image,
    Pdf,
    Folder,
    Info,
    Unavailable,
}

/// A result the overlay can draw without doing more I/O.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub title: String,
    pub kind: PreviewKind,
    /// Text to show (a file's contents, or a short message).
    #[serde(default)]
    pub text: String,
    /// Language hint for light syntax colouring: "rs", "toml", "py", ...
    #[serde(default)]
    pub language: String,
    /// A raster image (an original, or a PDF's first page).
    #[serde(default)]
    pub image: Option<PathBuf>,
    /// Folder entries or app facts, one per line.
    #[serde(default)]
    pub lines: Vec<String>,
}

impl Preview {
    fn info(title: impl Into<String>, text: impl Into<String>) -> Self {
        Preview {
            title: title.into(),
            kind: PreviewKind::Info,
            text: text.into(),
            language: String::new(),
            image: None,
            lines: Vec::new(),
        }
    }

    fn unavailable(title: impl Into<String>, text: impl Into<String>) -> Self {
        Preview {
            title: title.into(),
            kind: PreviewKind::Unavailable,
            text: text.into(),
            language: String::new(),
            image: None,
            lines: Vec::new(),
        }
    }
}

/// A preview of `path` (a file, folder, or `.app` bundle).
pub fn for_path(path: &Path) -> Preview {
    let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tilde(path));
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => return Preview::unavailable(title, format!("This item could not be read: {e}")),
    };
    if meta.file_type().is_symlink() && fs::metadata(path).is_err() {
        return Preview::unavailable(title, "This link is broken.");
    }
    if path.is_dir() {
        return folder(path, title);
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if is_image(&ext) {
        return image(path, title, meta.len());
    }
    if ext == "pdf" {
        return pdf(path, title, meta.len());
    }
    if ext == "svg" {
        return image(path, title, meta.len());
    }
    text_file(path, title, &ext, meta.len())
}

/// A preview of a clipboard-history item. Images are written to `image_out`
/// (a file the overlay can show).
pub fn for_clipboard(item: &Item, blob: Option<&[u8]>, image_out: Option<&Path>) -> Preview {
    let title = item.preview.clone();
    match item.kind {
        ClipKind::Text => Preview {
            title,
            kind: PreviewKind::Text,
            text: item.text.clone().unwrap_or_default(),
            language: String::new(),
            image: None,
            lines: Vec::new(),
        },
        ClipKind::Files => {
            let text = item.text.clone().unwrap_or_default();
            let lines: Vec<String> =
                text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.trim().to_owned()).collect();
            Preview {
                title,
                kind: PreviewKind::Folder,
                text: String::new(),
                language: String::new(),
                image: None,
                lines,
            }
        }
        ClipKind::Image => {
            if let (Some(bytes), Some(out)) = (blob, image_out) {
                if let Some(parent) = out.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::write(out, bytes).is_ok() {
                    return Preview {
                        title,
                        kind: PreviewKind::Image,
                        text: String::new(),
                        language: String::new(),
                        image: Some(out.to_path_buf()),
                        lines: Vec::new(),
                    };
                }
            }
            Preview::unavailable(title, "This image is no longer on disk.")
        }
    }
}

/// A short preview of an application or other non-file result.
pub fn for_hit(hit: &Hit) -> Preview {
    if let Some(path) = hit.path() {
        return for_path(path);
    }
    let mut lines = Vec::new();
    if !hit.subtitle.is_empty() {
        lines.push(hit.subtitle.clone());
    }
    if !hit.key.is_empty() {
        lines.push(hit.key.clone());
    }
    match &hit.action {
        crate::Action::Run(argv) => lines.push(argv.join(" ")),
        crate::Action::Url(u) => lines.push(u.clone()),
        crate::Action::Copy(t) => {
            return Preview {
                title: hit.title.clone(),
                kind: PreviewKind::Text,
                text: t.clone(),
                language: String::new(),
                image: None,
                lines: Vec::new(),
            };
        }
        crate::Action::AskHomer(t) => lines.push(t.clone()),
        _ => {}
    }
    Preview {
        title: hit.title.clone(),
        kind: PreviewKind::Info,
        text: String::new(),
        language: String::new(),
        image: None,
        lines,
    }
}

fn folder(path: &Path, title: String) -> Preview {
    let mut entries: Vec<(bool, String)> = match fs::read_dir(path) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    || e.path().extension().is_some_and(|x| x == "app") && e.path().is_dir();
                (dir, name)
            })
            .filter(|(_, n)| !n.starts_with('.'))
            .collect(),
        Err(e) => return Preview::unavailable(title, format!("This folder could not be read: {e}")),
    };
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let total = entries.len();
    let mut lines: Vec<String> =
        entries.into_iter().take(MAX_FOLDER).map(|(dir, name)| if dir { format!("{name}/") } else { name }).collect();
    if total > lines.len() {
        lines.push(format!("and {} more", total - lines.len()));
    }
    let text = if lines.is_empty() { "This folder is empty.".into() } else { String::new() };
    Preview { title, kind: PreviewKind::Folder, text, language: String::new(), image: None, lines }
}

fn is_image(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg")
}

fn image(path: &Path, title: String, len: u64) -> Preview {
    if len > MAX_IMAGE {
        return Preview::unavailable(title, "This image is too large to preview.");
    }
    Preview {
        title,
        kind: PreviewKind::Image,
        text: String::new(),
        language: String::new(),
        image: Some(path.to_path_buf()),
        lines: Vec::new(),
    }
}

fn pdf(path: &Path, title: String, len: u64) -> Preview {
    if len > MAX_IMAGE * 4 {
        return Preview::info(title, "This PDF is too large to preview.");
    }
    let Some(out_dir) = crate::runtime_dir().or_else(crate::cache_dir) else {
        return Preview::info(title, "This is a PDF. A first-page preview needs a place to write a picture.");
    };
    let dir = out_dir.join("quicklook");
    let _ = fs::create_dir_all(&dir);
    let prefix = dir.join(format!("pdf-{}", std::process::id()));
    let status = Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile", "-r", "110"])
        .arg(path)
        .arg(&prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let png = prefix.with_extension("png");
    if status.is_ok_and(|s| s.success()) && png.is_file() {
        return Preview {
            title,
            kind: PreviewKind::Pdf,
            text: String::new(),
            language: String::new(),
            image: Some(png),
            lines: Vec::new(),
        };
    }
    Preview::info(title, "This is a PDF. A first-page preview appears when pdftoppm is available.")
}

fn text_file(path: &Path, title: String, ext: &str, len: u64) -> Preview {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return Preview::unavailable(title, format!("This file could not be read: {e}")),
    };
    let mut buf = vec![0u8; MAX_TEXT.min(len as usize + 1).max(1)];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(e) => return Preview::unavailable(title, format!("This file could not be read: {e}")),
    };
    buf.truncate(n);
    if buf.contains(&0) {
        return Preview::info(title, "This is a binary file.");
    }
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if len as usize > n {
        text.push_str("\n...");
    }
    Preview { title, kind: PreviewKind::Text, text, language: language(ext, path), image: None, lines: Vec::new() }
}

/// A short language id used for light syntax colouring.
pub fn language(ext: &str, path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match ext {
        "rs" => "rs",
        "toml" => "toml",
        "py" => "py",
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" => "js",
        "json" => "json",
        "md" | "markdown" => "md",
        "nix" => "nix",
        "sh" | "bash" | "zsh" => "sh",
        "html" | "htm" | "xml" => "html",
        "c" | "h" | "cc" | "hh" | "cpp" | "hpp" => "c",
        "go" => "go",
        "rb" => "rb",
        "swift" => "swift",
        "kt" => "kt",
        "java" => "java",
        "css" => "css",
        "yml" | "yaml" => "yaml",
        "ftl" => "ftl",
        "ini" | "cfg" | "conf" => "ini",
        "txt" | "text" | "log" => "txt",
        _ if name == "Makefile" || name == "Dockerfile" => "sh",
        _ if name == "Cargo.lock" => "toml",
        _ => "",
    }
    .into()
}

/// Light syntax colouring: comment and string ranges in `text`, as byte
/// offsets, for the overlay to colour without a heavy highlighter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Normal,
    Comment,
    String,
}

pub fn highlight(text: &str, language: &str) -> Vec<(SpanKind, usize, usize)> {
    // Byte ranges, covering the whole text in order.
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let line_comment: &[&[u8]] = match language {
        "rs" | "js" | "c" | "go" | "swift" | "kt" | "java" | "css" => &[b"//"],
        "py" | "sh" | "toml" | "nix" | "yaml" | "ini" | "ftl" => &[b"#"],
        "html" | "md" => &[],
        "rb" => &[b"#"],
        _ => &[],
    };
    let block: Option<(&[u8], &[u8])> = match language {
        "rs" | "js" | "c" | "go" | "swift" | "kt" | "java" | "css" => Some((b"/*", b"*/")),
        _ => None,
    };
    while i < b.len() {
        if (language == "html" || language == "md")
            && let (start, end) = (b"<!--".as_slice(), b"-->".as_slice())
            && b[i..].starts_with(start)
        {
            let rel = find_from(b, i + start.len(), end).unwrap_or(b.len());
            let end_i = (rel + end.len()).min(b.len());
            out.push((SpanKind::Comment, i, end_i));
            i = end_i;
            continue;
        }
        if let Some((start, end)) = block.filter(|_| language != "html" && language != "md")
            && b[i..].starts_with(start)
        {
            let rel = find_from(b, i + start.len(), end).unwrap_or(b.len());
            let end_i = (rel + end.len()).min(b.len());
            out.push((SpanKind::Comment, i, end_i));
            i = end_i;
            continue;
        }
        if line_comment.iter().any(|m| b[i..].starts_with(m)) {
            let end_i = b[i..].iter().position(|c| *c == b'\n').map(|p| i + p).unwrap_or(b.len());
            out.push((SpanKind::Comment, i, end_i));
            i = end_i;
            continue;
        }
        if b[i] == b'"' || (language != "html" && b[i] == b'\'') {
            let quote = b[i];
            let mut j = i + 1;
            while j < b.len() {
                if b[j] == b'\\' && j + 1 < b.len() {
                    j += 2;
                    continue;
                }
                if b[j] == quote {
                    j += 1;
                    break;
                }
                if b[j] == b'\n' && language != "json" {
                    break;
                }
                j += 1;
            }
            out.push((SpanKind::String, i, j));
            i = j;
            continue;
        }
        let start = i;
        i += 1;
        while i < b.len() {
            let comment_here = line_comment.iter().any(|m| b[i..].starts_with(m))
                || block.is_some_and(|(s, _)| language != "html" && language != "md" && b[i..].starts_with(s))
                || ((language == "html" || language == "md") && b[i..].starts_with(b"<!--"))
                || b[i] == b'"'
                || (language != "html" && b[i] == b'\'');
            if comment_here {
                break;
            }
            i += 1;
        }
        if i > start {
            out.push((SpanKind::Normal, start, i));
        }
    }
    out
}

fn find_from(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b[from..].windows(pat.len()).position(|w| w == pat).map(|p| from + p)
}

/// Write `bytes` to a unique file in the runtime (or cache) directory.
pub fn write_temp(name: &str, bytes: &[u8]) -> Option<PathBuf> {
    let dir = crate::runtime_dir().or_else(crate::cache_dir)?.join("quicklook");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    let mut f = File::create(&path).ok()?;
    f.write_all(bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_folders_and_binary_are_previewed() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("hello.rs");
        fs::write(&f, "fn main() {\n    // hi\n    println!(\"ok\");\n}\n").unwrap();
        let p = for_path(&f);
        assert_eq!(p.kind, PreviewKind::Text);
        assert_eq!(p.language, "rs");
        assert!(p.text.contains("println!"));
        let spans = highlight(&p.text, "rs");
        assert!(spans.iter().any(|(k, _, _)| *k == SpanKind::Comment));
        assert!(spans.iter().any(|(k, _, _)| *k == SpanKind::String));

        fs::create_dir_all(d.path().join("docs")).unwrap();
        fs::write(d.path().join("docs/a.md"), "# A").unwrap();
        fs::write(d.path().join("z.txt"), "z").unwrap();
        let folder = for_path(d.path());
        assert_eq!(folder.kind, PreviewKind::Folder);
        assert!(folder.lines.iter().any(|l| l == "docs/"));
        assert!(folder.lines.iter().any(|l| l == "hello.rs"));

        let bin = d.path().join("blob.bin");
        fs::write(&bin, [0u8, 1, 2, 0]).unwrap();
        assert_eq!(for_path(&bin).kind, PreviewKind::Info);
        assert!(for_path(&bin).text.contains("binary"));
    }

    #[test]
    fn highlight_covers_the_whole_string() {
        let s = "a = \"b\" # c\n";
        let spans = highlight(s, "toml");
        let covered: usize = spans.iter().map(|(_, a, b)| b - a).sum();
        assert_eq!(covered, s.len());
        assert!(spans.iter().any(|(k, a, b)| *k == SpanKind::String && &s[*a..*b] == "\"b\""));
        assert!(spans.iter().any(|(k, a, b)| *k == SpanKind::Comment && s[*a..*b].contains('#')));
    }
}
