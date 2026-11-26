//! Layer 3: the Chrome bridge (Chrome DevTools Protocol).
//!
//! Homer's Chrome is Google Chrome from Applications, started by Homer in
//! its sandbox with `--remote-debugging-pipe`: the protocol runs over two
//! pipes (fds 3 and 4) that only this process holds. There is no port, so
//! nothing else on the computer (or the network) can drive it.
//!
//! It uses a profile of its own (`~/.config/homer-chrome` in Chrome's
//! sandbox folder). Chrome refuses remote control of the everyday profile
//! (since Chrome 136, against cookie theft), and that is right: the person
//! signs in to what Homer should use (Google Docs) once in Homer's window,
//! and Homer never sees the cookies of their everyday browsing. Their
//! everyday Chrome window is still reachable through accessibility
//! (layer 1) and the screen (layer 2).

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use command_fds::{CommandFdExt, FdMapping};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

pub const PAGE_SCRIPT: &str = include_str!("page.js");
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub struct Chrome {
    writer: tokio::sync::Mutex<tokio::net::unix::pipe::Sender>,
    pending: Pending,
    next: std::sync::atomic::AtomicU64,
    child: tokio::process::Child,
    /// Short tab ids ("t1") for Chrome's long target ids, both ways.
    tabs: Mutex<Vec<String>>,
    sessions: Mutex<HashMap<String, String>>,
    pub app: String,
}

impl std::fmt::Debug for Chrome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Chrome")
    }
}

/// A tab as Homer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub url: String,
}

/// Arguments for Homer's Chrome (after the program).
pub fn chrome_args(profile: &str) -> Vec<String> {
    vec![
        "--remote-debugging-pipe".into(),
        format!("--user-data-dir={profile}"),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        // Its accessibility tree too, for layer 1.
        "--force-renderer-accessibility".into(),
        "about:blank".into(),
    ]
}

impl Chrome {
    /// Starts `program` with `args`, the protocol on fds 3 (to Chrome) and
    /// 4 (from Chrome).
    pub async fn launch(program: &str, args: &[String], app: &str) -> Result<Self> {
        let (to_chrome_r, to_chrome_w) = std::io::pipe()?;
        let (from_chrome_r, from_chrome_w) = std::io::pipe()?;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        cmd.fd_mappings(vec![
            FdMapping { parent_fd: OwnedFd::from(to_chrome_r), child_fd: 3 },
            FdMapping { parent_fd: OwnedFd::from(from_chrome_w), child_fd: 4 },
        ])
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let child = cmd.spawn().with_context(|| format!("starting {program}"))?;
        drop(cmd);
        let writer = tokio::net::unix::pipe::Sender::from_owned_fd(OwnedFd::from(to_chrome_w))?;
        let mut reader = tokio::net::unix::pipe::Receiver::from_owned_fd(OwnedFd::from(from_chrome_r))?;
        let pending: Pending = Arc::default();
        let p2 = pending.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&chunk[..n]);
                while let Some(end) = buf.iter().position(|b| *b == 0) {
                    let msg: Vec<u8> = buf.drain(..=end).collect();
                    let Ok(v) = serde_json::from_slice::<Value>(&msg[..msg.len() - 1]) else { continue };
                    if let Some(id) = v["id"].as_u64()
                        && let Some(tx) = p2.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
                    {
                        let _ = tx.send(v);
                    }
                }
            }
            // Chrome closed: every waiting call fails.
            p2.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        let c = Self {
            writer: tokio::sync::Mutex::new(writer),
            pending,
            next: 1.into(),
            child,
            tabs: Mutex::default(),
            sessions: Mutex::default(),
            app: app.to_owned(),
        };
        // Chrome answers once it is up.
        let mut last = None;
        for _ in 0..60 {
            match tokio::time::timeout(Duration::from_secs(1), c.call("Browser.getVersion", json!({}), None)).await {
                Ok(Ok(_)) => return Ok(c),
                Ok(Err(e)) => last = Some(e),
                Err(_) => {}
            }
        }
        bail!("Chrome did not start: {}", last.map(|e| format!("{e:#}")).unwrap_or_else(|| "no answer".into()))
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        let mut bytes = serde_json::to_vec(&msg)?;
        bytes.push(0);
        self.writer.lock().await.write_all(&bytes).await.context("Chrome has closed")?;
        let v = tokio::time::timeout(CALL_TIMEOUT, rx)
            .await
            .with_context(|| format!("Chrome did not answer {method}"))?
            .context("Chrome has closed")?;
        if let Some(e) = v.get("error") {
            bail!("Chrome: {}", e["message"].as_str().unwrap_or("error"));
        }
        Ok(v["result"].clone())
    }

    fn short_id(&self, target: &str) -> String {
        let mut t = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        let i = t.iter().position(|x| x == target).unwrap_or_else(|| {
            t.push(target.to_owned());
            t.len() - 1
        });
        format!("t{}", i + 1)
    }

    fn target_of(&self, tab: &str) -> Result<String> {
        let t = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        tab.strip_prefix('t')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| t.get(n.wrapping_sub(1)).cloned())
            .with_context(|| format!("there is no tab {tab:?}: list them with browser_tabs"))
    }

    pub async fn tabs(&self) -> Result<Vec<Tab>> {
        let r = self.call("Target.getTargets", json!({}), None).await?;
        Ok(r["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["type"] == "page")
            .map(|t| Tab {
                id: self.short_id(t["targetId"].as_str().unwrap_or("")),
                title: t["title"].as_str().unwrap_or("").to_owned(),
                url: t["url"].as_str().unwrap_or("").to_owned(),
            })
            .collect())
    }

    async fn session(&self, tab: &str) -> Result<String> {
        let target = self.target_of(tab)?;
        if let Some(s) = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&target) {
            return Ok(s.clone());
        }
        let r = self.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
        let s = r["sessionId"].as_str().context("Chrome did not attach to the tab")?.to_owned();
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(target, s.clone());
        Ok(s)
    }

    pub async fn open(&self, url: &str) -> Result<Tab> {
        check_url(url)?;
        let r = self.call("Target.createTarget", json!({ "url": url }), None).await?;
        let target = r["targetId"].as_str().context("no tab")?.to_owned();
        let id = self.short_id(&target);
        let _ = self.call("Target.activateTarget", json!({ "targetId": target }), None).await;
        self.wait_loaded(&id).await;
        let tab = self.tabs().await?.into_iter().find(|t| t.id == id);
        Ok(tab.unwrap_or(Tab { id, title: String::new(), url: url.to_owned() }))
    }

    pub async fn close(&self, tab: &str) -> Result<()> {
        let target = self.target_of(tab)?;
        self.call("Target.closeTarget", json!({ "targetId": target }), None).await?;
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&target);
        Ok(())
    }

    pub async fn navigate(&self, tab: &str, url: &str) -> Result<()> {
        check_url(url)?;
        let s = self.session(tab).await?;
        let r = self.call("Page.navigate", json!({ "url": url }), Some(&s)).await?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            bail!("the page did not open: {e}");
        }
        self.wait_loaded(tab).await;
        Ok(())
    }

    async fn wait_loaded(&self, tab: &str) {
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Ok(v) = self.eval(tab, "document.readyState").await
                && v == "complete"
            {
                return;
            }
        }
    }

    /// Runs `expr` in the page and returns its value.
    pub async fn eval(&self, tab: &str, expr: &str) -> Result<Value> {
        let s = self.session(tab).await?;
        let r = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": expr, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
                Some(&s),
            )
            .await?;
        if let Some(e) = r.get("exceptionDetails") {
            bail!("the page script failed: {}", e["exception"]["description"].as_str().unwrap_or("error"));
        }
        Ok(r["result"]["value"].clone())
    }

    /// The page's readable text and its controls.
    pub async fn read(&self, tab: &str) -> Result<Value> {
        let mut v = self.eval(tab, PAGE_SCRIPT).await?;
        // Google Docs draws the document on a canvas: its text comes from
        // the document's own plain-text export, in the signed-in session.
        let url = v["url"].as_str().unwrap_or("").to_owned();
        if url.starts_with("https://docs.google.com/document/d/") {
            let js = "fetch(location.pathname.replace(/\\/(edit|view).*$/, '') + '/export?format=txt', {credentials: 'include'}).then(r => r.ok ? r.text() : '')";
            if let Ok(Value::String(t)) = self.eval(tab, js).await
                && !t.trim().is_empty()
            {
                v["document_text"] = json!(t.chars().take(12000).collect::<String>());
            }
        }
        Ok(v)
    }

    /// What an element is, before acting on it.
    pub async fn element(&self, tab: &str, id: &str) -> Result<Value> {
        let js = format!(
            "(() => {{ const q = (d) => d.querySelector('[data-homer-id=\"{id}\"]');
              let el = q(document);
              if (!el) for (const f of document.querySelectorAll('iframe')) {{ try {{ el = q(f.contentDocument); if (el) break; }} catch (e) {{}} }}
              if (!el) return null;
              const label = (el.getAttribute('aria-label') || el.innerText || el.value || el.getAttribute('title') || el.getAttribute('placeholder') || '').replace(/\\s+/g, ' ').trim().slice(0, 120);
              return {{ tag: el.tagName.toLowerCase(), type: el.type || '', label, password: el.type === 'password',
                        form: el.form ? (el.form.method || 'get').toLowerCase() : '',
                        submits: !!el.form && (el.type === 'submit' || (el.tagName === 'BUTTON' && !el.getAttribute('type'))) }};
            }})()",
            id = js_id(id)?
        );
        let v = self.eval(tab, &js).await?;
        anyhow::ensure!(!v.is_null(), "element {id} is not on the page any more: read the page again");
        Ok(v)
    }

    /// Clicks an element as a person would (real mouse events at its centre).
    pub async fn click(&self, tab: &str, id: &str) -> Result<()> {
        let js = format!(
            "(() => {{ const q = (d) => d.querySelector('[data-homer-id=\"{id}\"]');
              let el = q(document), dx = 0, dy = 0;
              if (!el) for (const f of document.querySelectorAll('iframe')) {{
                try {{ el = q(f.contentDocument); if (el) {{ const r = f.getBoundingClientRect(); dx = r.left; dy = r.top; break; }} }} catch (e) {{}} }}
              if (!el) return null;
              el.scrollIntoView({{ block: 'center', inline: 'center' }});
              const r = el.getBoundingClientRect();
              return {{ x: dx + r.left + r.width / 2, y: dy + r.top + r.height / 2 }};
            }})()",
            id = js_id(id)?
        );
        let p = self.eval(tab, &js).await?;
        let (x, y) = (p["x"].as_f64().context("element not found")?, p["y"].as_f64().unwrap_or(0.0));
        let s = self.session(tab).await?;
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": if kind == "mouseMoved" { 0 } else { 1 } }),
                Some(&s),
            )
            .await?;
        }
        Ok(())
    }

    /// Focuses an element and replaces its text (as typed, so pages and
    /// editors see real input). Returns what was there before.
    pub async fn fill(&self, tab: &str, id: &str, text: &str) -> Result<String> {
        let js = format!(
            "(() => {{ const q = (d) => d.querySelector('[data-homer-id=\"{id}\"]');
              let el = q(document);
              if (!el) for (const f of document.querySelectorAll('iframe')) {{ try {{ el = q(f.contentDocument); if (el) break; }} catch (e) {{}} }}
              if (!el) return null;
              el.scrollIntoView({{ block: 'center' }});
              el.focus();
              const before = el.isContentEditable ? el.innerText : el.value;
              if (el.select) el.select(); else el.ownerDocument.execCommand('selectAll');
              return before || '';
            }})()",
            id = js_id(id)?
        );
        let before = self.eval(tab, &js).await?;
        anyhow::ensure!(!before.is_null(), "element {id} is not on the page any more: read the page again");
        self.insert_text(tab, text).await?;
        Ok(before.as_str().unwrap_or("").to_owned())
    }

    /// Types into whatever has the focus (a Google Docs document, a chat
    /// box).
    pub async fn insert_text(&self, tab: &str, text: &str) -> Result<()> {
        let s = self.session(tab).await?;
        // Line by line: Enter between lines makes new paragraphs in editors.
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.key(tab, "Enter").await?;
            }
            if !line.is_empty() {
                self.call("Input.insertText", json!({ "text": line }), Some(&s)).await?;
            }
        }
        Ok(())
    }

    /// Is the focused element a single-line field in a form (Enter submits)?
    pub async fn focus_in_form(&self, tab: &str) -> bool {
        self.eval(
            tab,
            "(() => { const a = document.activeElement; return !!(a && a.form && a.tagName === 'INPUT'); })()",
        )
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    }

    /// Presses a key or a combination ("Enter", "ctrl+b", "shift+Tab").
    pub async fn key(&self, tab: &str, combo: &str) -> Result<()> {
        let s = self.session(tab).await?;
        let (mods, key) = cdp_key(combo)?;
        let mut down = json!({ "type": "keyDown", "modifiers": mods, "key": key.key, "code": key.code,
                               "windowsVirtualKeyCode": key.vk, "nativeVirtualKeyCode": key.vk });
        if let Some(t) = &key.text
            && mods & !8 == 0
        {
            down["text"] = json!(t);
        }
        self.call("Input.dispatchKeyEvent", down.clone(), Some(&s)).await?;
        let mut up = down;
        up["type"] = json!("keyUp");
        up.as_object_mut().map(|o| o.remove("text"));
        self.call("Input.dispatchKeyEvent", up, Some(&s)).await?;
        Ok(())
    }

    pub async fn screenshot(&self, tab: &str) -> Result<Vec<u8>> {
        use base64::Engine;
        let s = self.session(tab).await?;
        let r = self.call("Page.captureScreenshot", json!({ "format": "png" }), Some(&s)).await?;
        Ok(base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or(""))?)
    }
}

/// Element ids are the numbers page.js gave them.
fn js_id(id: &str) -> Result<&str> {
    anyhow::ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_digit()), "element ids are numbers, like \"12\"");
    Ok(id)
}

/// Only web pages: never `file:`, `chrome:` or `javascript:` addresses.
pub fn check_url(url: &str) -> Result<()> {
    let u = url.trim();
    anyhow::ensure!(
        (u.starts_with("https://") || u.starts_with("http://") || u == "about:blank")
            && !u.chars().any(char::is_whitespace),
        "Homer opens only web addresses (http or https)"
    );
    Ok(())
}

#[derive(Debug, PartialEq)]
pub struct CdpKey {
    pub key: String,
    pub code: String,
    pub vk: u32,
    pub text: Option<String>,
}

/// "ctrl+b" → modifiers (Alt 1, Ctrl 2, Meta 4, Shift 8) and the key.
pub fn cdp_key(combo: &str) -> Result<(u32, CdpKey)> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, mods) = parts.split_last().context("no key given")?;
    let mut m = 0;
    for x in mods {
        m |= match x.to_lowercase().as_str() {
            "alt" => 1,
            "ctrl" | "control" => 2,
            "meta" | "super" | "cmd" => 4,
            "shift" => 8,
            other => bail!("{other:?} is not a modifier key"),
        };
    }
    let named = |k: &str, code: &str, vk: u32, text: Option<&str>| CdpKey {
        key: k.into(),
        code: code.into(),
        vk,
        text: text.map(str::to_owned),
    };
    let k = match key.to_lowercase().as_str() {
        "enter" | "return" => named("Enter", "Enter", 13, Some("\r")),
        "tab" => named("Tab", "Tab", 9, None),
        "escape" | "esc" => named("Escape", "Escape", 27, None),
        "backspace" => named("Backspace", "Backspace", 8, None),
        "delete" | "del" => named("Delete", "Delete", 46, None),
        "space" => named(" ", "Space", 32, Some(" ")),
        "up" => named("ArrowUp", "ArrowUp", 38, None),
        "down" => named("ArrowDown", "ArrowDown", 40, None),
        "left" => named("ArrowLeft", "ArrowLeft", 37, None),
        "right" => named("ArrowRight", "ArrowRight", 39, None),
        "home" => named("Home", "Home", 36, None),
        "end" => named("End", "End", 35, None),
        "pageup" => named("PageUp", "PageUp", 33, None),
        "pagedown" => named("PageDown", "PageDown", 34, None),
        k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
            let c = k.chars().next().unwrap_or('a');
            let up = c.to_ascii_uppercase();
            let code = if c.is_ascii_digit() { format!("Digit{c}") } else { format!("Key{up}") };
            let shown = if m & 8 != 0 { up.to_string() } else { c.to_string() };
            CdpKey { key: shown.clone(), code, vk: up as u32, text: Some(shown) }
        }
        other => bail!("Homer cannot press {other:?} here"),
    };
    Ok((m, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_addresses() {
        assert_eq!(
            cdp_key("ctrl+b").unwrap(),
            (2, CdpKey { key: "b".into(), code: "KeyB".into(), vk: 66, text: Some("b".into()) })
        );
        assert_eq!(cdp_key("Enter").unwrap().1.vk, 13);
        assert_eq!(cdp_key("shift+Tab").unwrap().0, 8);
        assert!(cdp_key("ctrl+§").is_err());
        assert!(check_url("https://docs.google.com/").is_ok());
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "chrome://settings", "https://a b"] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
        assert!(js_id("12").is_ok() && js_id("1'] , x").is_err());
    }
}
