//! Talking to the system service (used by the `iliada` command and the apps).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::protocol::{Event, Request, Status};

/// Where the service listens (`ILIADA_SYSTEM_SOCKET` overrides it).
pub fn socket_path() -> PathBuf {
    std::env::var_os("ILIADA_SYSTEM_SOCKET").map(PathBuf::from).unwrap_or_else(|| crate::SOCKET.into())
}

/// Send `request`; `on_event` sees every progress event; returns the result.
pub async fn call(request: &Request, on_event: &mut (dyn FnMut(&Event) + Send)) -> Result<serde_json::Value> {
    // In a terminal (over SSH, say) there may be no password window: polkit
    // then asks here. It stays out of the way of a desktop's own window.
    let _agent = request.admin_action().and_then(|_| TerminalAgent::start());
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("the Iliada system service is not running ({})", path.display()))?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    write.shutdown().await?;
    let mut lines = BufReader::new(read).lines();
    while let Some(l) = lines.next_line().await? {
        let event: Event = serde_json::from_str(&l).with_context(|| format!("bad reply from the service: {l}"))?;
        match event {
            Event::Done { result } => return Ok(result),
            Event::Failed { error } => bail!("{error}"),
            other => on_event(&other),
        }
    }
    bail!("the Iliada system service stopped before answering")
}

/// polkit's terminal password prompt for this process, while it lives.
struct TerminalAgent(std::process::Child);

impl TerminalAgent {
    fn start() -> Option<TerminalAgent> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return None;
        }
        let bin = ["/run/current-system/sw/bin/pkttyagent", "/usr/bin/pkttyagent"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())?;
        let child = std::process::Command::new(bin)
            .args(["--process", &std::process::id().to_string(), "--fallback"])
            .spawn()
            .ok()?;
        // It registers with polkit in a few milliseconds.
        std::thread::sleep(std::time::Duration::from_millis(300));
        Some(TerminalAgent(child))
    }
}

impl Drop for TerminalAgent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// [`call`] and decode the result.
pub async fn call_as<T: DeserializeOwned>(request: &Request, on_event: &mut (dyn FnMut(&Event) + Send)) -> Result<T> {
    Ok(serde_json::from_value(call(request, on_event).await?)?)
}

pub async fn status() -> Result<Status> {
    call_as(&Request::Status, &mut |_| {}).await
}
