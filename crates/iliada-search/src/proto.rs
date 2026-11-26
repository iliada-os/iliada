//! The search service's protocol: one JSON object per line over the Unix
//! socket `$XDG_RUNTIME_DIR/iliada-searchd.sock`, a reply per request, on
//! a connection kept open by the overlay.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::Hit;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Results for what is typed. `homer`: Homer is on, so a row may ask him.
    Search {
        query: String,
        homer: bool,
        limit: usize,
    },
    /// Apps that open this file.
    OpenWith {
        path: PathBuf,
    },
    /// This result was chosen (it rises next time).
    Used {
        hit: Hit,
    },
    /// Forget what was chosen (Settings → Privacy).
    Forget,
    Stats,
    /// Clipboard history (Super+V), filtered by `query`.
    Clipboard {
        query: String,
        limit: usize,
    },
    /// Pin, delete, copy and clear actions for one clipboard item.
    ClipboardExtras {
        id: u64,
    },
    ClipboardPin {
        id: u64,
        pinned: bool,
    },
    ClipboardDelete {
        id: u64,
    },
    ClipboardClear,
    /// Put this item back on the clipboard (the overlay then pastes).
    ClipboardRestore {
        id: u64,
    },
    ClipboardSetEnabled {
        enabled: bool,
    },
    ClipboardExclude {
        app: String,
        exclude: bool,
    },
    ClipboardConfig,
    /// Quick Look for a file, folder or app path.
    PreviewPath {
        path: PathBuf,
    },
    /// Quick Look for a clipboard-history item.
    PreviewClipboard {
        id: u64,
    },
    /// Switch to an open window (a `w ` result).
    FocusWindow {
        identifier: String,
    },
    /// Settings → Privacy → Search: show open windows or not.
    ShowWindows {
        enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Response {
    /// Results, and how long finding them took.
    Hits {
        hits: Vec<Hit>,
        micros: u64,
    },
    Done,
    Stats {
        files: usize,
        removed: usize,
        apps: usize,
        offers: usize,
        memory: usize,
        indexing: bool,
    },
    Error {
        message: String,
    },
    /// Clipboard history settings.
    ClipboardConfig {
        enabled: bool,
        exclude: Vec<String>,
    },
    Preview {
        preview: crate::preview::Preview,
    },
}

pub fn socket_path() -> Option<PathBuf> {
    crate::runtime_dir().map(|d| d.join("iliada-searchd.sock"))
}

/// A connection to the search service.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> Result<Self> {
        let path = socket_path().context("XDG_RUNTIME_DIR is not set")?;
        let writer = UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
        writer.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        Ok(Client { reader: BufReader::new(writer.try_clone()?), writer })
    }

    pub fn call(&mut self, req: &Request) -> Result<Response> {
        let mut line = serde_json::to_vec(req)?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            bail!("the search service closed the connection");
        }
        Ok(serde_json::from_str(&reply)?)
    }

    /// Results for `query`.
    pub fn search(&mut self, query: &str, homer: bool, limit: usize) -> Result<(Vec<Hit>, u64)> {
        match self.call(&Request::Search { query: query.to_owned(), homer, limit })? {
            Response::Hits { hits, micros } => Ok((hits, micros)),
            Response::Error { message } => bail!(message),
            other => bail!("unexpected reply {other:?}"),
        }
    }

    pub fn clipboard(&mut self, query: &str, limit: usize) -> Result<(Vec<Hit>, u64)> {
        match self.call(&Request::Clipboard { query: query.to_owned(), limit })? {
            Response::Hits { hits, micros } => Ok((hits, micros)),
            Response::Error { message } => bail!(message),
            other => bail!("unexpected reply {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        let r = Request::Search { query: "fire".into(), homer: true, limit: 8 };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"op":"search","query":"fire","homer":true,"limit":8}"#);
        assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), r);
    }
}
