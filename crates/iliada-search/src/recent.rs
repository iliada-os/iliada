//! Recently used files, as apps record them (`recently-used.xbel`, the
//! freedesktop list GTK and COSMIC apps write).

use std::path::{Path, PathBuf};

/// The list's usual place.
pub fn default_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::home().map(|h| h.join(".local/share")))?;
    Some(data.join("recently-used.xbel"))
}

/// Undo %XX escapes in a file:// URL.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
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

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[at..].find('"')?;
    Some(&tag[at..at + len])
}

/// The files in an xbel list, most recently used first.
pub fn parse(xbel: &str) -> Vec<PathBuf> {
    let mut items: Vec<(String, PathBuf)> = xbel
        .split("<bookmark ")
        .skip(1)
        .filter_map(|tag| {
            let tag = format!(" {}", &tag[..tag.find('>')?]);
            let tag = tag.as_str();
            let href = attr(tag, "href")?.strip_prefix("file://")?;
            let when = attr(tag, "visited").or_else(|| attr(tag, "modified")).unwrap_or("").to_owned();
            Some((when, PathBuf::from(unescape(&href.replace("&amp;", "&")))))
        })
        .collect();
    // ISO 8601 times sort as text.
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.into_iter().map(|(_, p)| p).collect()
}

/// Recent files that still exist, most recent first.
pub fn load(path: &Path, n: usize) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&text).into_iter().filter(|p| p.exists()).take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_xbel() {
        let x = r#"<?xml version="1.0"?>
<xbel version="1.0">
  <bookmark href="file:///home/a/Old%20File.txt" added="2026-01-01T10:00:00Z" modified="2026-01-01T10:00:00Z" visited="2026-01-01T10:00:00Z">
  </bookmark>
  <bookmark href="file:///home/a/new.pdf" added="2026-11-20T10:00:00Z" modified="2026-11-20T10:00:00Z" visited="2026-11-20T10:00:00Z">
  </bookmark>
  <bookmark href="https://example.com/" visited="2026-11-21T10:00:00Z"></bookmark>
</xbel>"#;
        assert_eq!(parse(x), vec![PathBuf::from("/home/a/new.pdf"), PathBuf::from("/home/a/Old File.txt")]);
    }
}
