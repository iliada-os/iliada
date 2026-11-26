//! Talking to `iliada-notifyd` over its socket
//! (`$XDG_RUNTIME_DIR/iliada-notifyd.sock`, the person's own): one JSON
//! request per line, one reply per line.
//!
//! The panel lists and opens notifications this way, and clearing goes
//! through the service so it can tell the apps and take down banners still
//! showing. When the service is not running, clearing edits the history
//! file directly.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::Notification;
use crate::history::History;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    /// The kept notifications, newest first.
    List,
    /// Clicked: run its action (`None`: the default one, if it has one)
    /// and take it out of the list.
    Activate {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
    },
    /// Remove one, an app's, or every notification.
    Dismiss {
        id: u32,
    },
    ClearApp {
        app: String,
    },
    ClearAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Response {
    Notifications {
        items: Vec<Notification>,
    },
    /// Done; `count` notifications were affected.
    Done {
        count: usize,
    },
    Error {
        message: String,
    },
}

/// Ask the service once.
pub fn call(request: &Request) -> Result<Response> {
    let path = crate::socket_path().context("no runtime folder")?;
    let stream = UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut w = &stream;
    writeln!(w, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    match serde_json::from_str(&line).context("reading the notification service's reply")? {
        Response::Error { message } => bail!(message),
        r => Ok(r),
    }
}

/// The most recent notifications, newest first: from the service, or the
/// history file when it is not running.
pub fn recent(limit: usize) -> Vec<Notification> {
    match call(&Request::List) {
        Ok(Response::Notifications { mut items }) => {
            items.truncate(limit);
            items
        }
        _ => crate::history_path().map(|p| History::load(&p).recent(limit)).unwrap_or_default(),
    }
}

/// Clear one app's notifications (`Some`) or all of them; how many were
/// cleared.
pub fn clear(app: Option<&str>) -> Result<usize> {
    let request = match app {
        Some(app) => Request::ClearApp { app: app.to_owned() },
        None => Request::ClearAll,
    };
    match call(&request) {
        Ok(Response::Done { count }) => return Ok(count),
        Ok(other) => bail!("unexpected reply {other:?}"),
        // Not running: change the file.
        Err(_) => {}
    }
    let path = crate::history_path().context("no home folder")?;
    let mut h = History::load(&path);
    let count = match app {
        Some(app) => h.clear_app(app).len(),
        None => h.clear_all().len(),
    };
    if count > 0 {
        h.save(&path)?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_on_the_wire() {
        let r = Request::Activate { id: 3, action: None };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"request":"activate","id":3}"#);
        let back: Request = serde_json::from_str(r#"{"request":"clear_app","app":"mail"}"#).unwrap();
        assert_eq!(back, Request::ClearApp { app: "mail".into() });
        let done: Response = serde_json::from_str(r#"{"reply":"done","count":2}"#).unwrap();
        assert_eq!(done, Response::Done { count: 2 });
    }
}
