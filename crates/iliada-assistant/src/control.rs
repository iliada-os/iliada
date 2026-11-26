//! Homer using other apps: the tools, and the client for the
//! `iliada-control` service that runs them (see crates/iliada-control).
//!
//! The service decides what Homer may touch and asks for the person's yes
//! when it must; the question reaches the person through the assistant's
//! own approval prompt (never through the model), and the answer goes back
//! with a one-time token.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// `(name, description, schema, changes something)`.
pub fn tools() -> Vec<(&'static str, &'static str, Value, bool)> {
    let tab = || json!({ "type": "string", "description": "Tab id from browser_tabs or browser_open, like \"t1\"" });
    let element = || json!({ "type": "string", "description": "Element id from browser_read, like \"12\"" });
    vec![
        (
            "list_windows",
            "The windows open on the screen, by app: which describe themselves (use read_ui) and which only show pixels (use screenshot).",
            obj(json!({}), &[]),
            false,
        ),
        (
            "read_ui",
            "Read an app's window through accessibility: one line per element (role, name, value, states, actions), each with an id like e12 for act_ui. Exact and cheap: prefer it to screenshots. app: its name as list_windows shows it. window: part of a window title, to read only that window.",
            obj(json!({ "app": { "type": "string" }, "window": { "type": "string" } }), &["app"]),
            false,
        ),
        (
            "act_ui",
            "Act on an element read with read_ui: press (buttons, menu items, links, checkboxes), focus, set_text (replace a field's text), select (a list item or tab), scroll (into view).",
            obj(
                json!({
                    "id": { "type": "string" },
                    "action": { "type": "string", "enum": ["press", "focus", "set_text", "select", "scroll"] },
                    "text": { "type": "string", "description": "For set_text" }
                }),
                &["id", "action"],
            ),
            true,
        ),
        (
            "screenshot",
            "See the screen (or one window, by title or app id) as an image, for apps read_ui cannot read. Coordinates for click and scroll are this image's pixels (of a whole-screen screenshot).",
            obj(json!({ "window": { "type": "string" } }), &[]),
            false,
        ),
        (
            "click",
            "Click at a point of the last whole-screen screenshot with the virtual mouse. target: what you are clicking, in words (\"the Save button\"): the person sees it, and it decides whether to ask them first.",
            obj(
                json!({
                    "x": { "type": "number" }, "y": { "type": "number" },
                    "target": { "type": "string" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"] },
                    "double": { "type": "boolean" }
                }),
                &["x", "y", "target"],
            ),
            true,
        ),
        (
            "type_text",
            "Type text with the virtual keyboard into whatever has the focus (any language).",
            obj(json!({ "text": { "type": "string" } }), &["text"]),
            true,
        ),
        (
            "key_combo",
            "Press a key or a combination with the virtual keyboard: \"Enter\", \"ctrl+s\", \"alt+Tab\", \"ctrl+shift+t\". target: what it does, in words.",
            obj(json!({ "keys": { "type": "string" }, "target": { "type": "string" } }), &["keys"]),
            true,
        ),
        (
            "scroll",
            "Scroll at a point of the last whole-screen screenshot, by wheel clicks (dy positive: down).",
            obj(
                json!({ "x": { "type": "number" }, "y": { "type": "number" }, "dx": { "type": "integer" }, "dy": { "type": "integer" } }),
                &["x", "y"],
            ),
            true,
        ),
        (
            "browser_tabs",
            "The tabs of Homer's own Chrome window (started when first needed). It has its own profile: the person signs in there once (for Google Docs, Gmail) and Homer can use those sites; their everyday Chrome profile stays private.",
            obj(json!({}), &[]),
            false,
        ),
        (
            "browser_open",
            "Open a web address in a new tab of Homer's Chrome.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            true,
        ),
        (
            "browser_navigate",
            "Go to a web address in a tab.",
            obj(json!({ "tab": tab(), "url": { "type": "string" } }), &["tab", "url"]),
            true,
        ),
        (
            "browser_read",
            "Read a tab: its title, address, readable text (a Google Docs document's text too) and its controls (links, buttons, fields, editors), each with an element id.",
            obj(json!({ "tab": tab() }), &["tab"]),
            false,
        ),
        (
            "browser_click",
            "Click an element of a tab (as a real click). Clicks that send, submit, delete, buy or post are shown to the person first.",
            obj(json!({ "tab": tab(), "element": element() }), &["tab", "element"]),
            true,
        ),
        (
            "browser_fill",
            "Replace the text of a field or editor in a tab (typed as real input). Never passwords.",
            obj(
                json!({ "tab": tab(), "element": element(), "text": { "type": "string" } }),
                &["tab", "element", "text"],
            ),
            true,
        ),
        (
            "browser_type",
            "Type text where the focus is in a tab (after clicking into an editor such as a Google Doc). New lines become Enter.",
            obj(json!({ "tab": tab(), "text": { "type": "string" } }), &["tab", "text"]),
            true,
        ),
        (
            "browser_key",
            "Press a key or combination in a tab: \"Enter\", \"Tab\", \"ctrl+b\", \"ctrl+z\".",
            obj(json!({ "tab": tab(), "keys": { "type": "string" } }), &["tab", "keys"]),
            true,
        ),
        ("browser_screenshot", "See a tab as an image.", obj(json!({ "tab": tab() }), &["tab"]), false),
        ("browser_close", "Close a tab of Homer's Chrome.", obj(json!({ "tab": tab() }), &["tab"]), true),
    ]
}

pub fn is_control(name: &str) -> bool {
    tools().iter().any(|t| t.0 == name)
}

/// Changes something in an app (logged in Activity), as opposed to reading.
pub fn acts(name: &str) -> bool {
    tools().iter().any(|t| t.0 == name && t.3)
}

/// The tool definitions, when Homer may use other apps at all.
pub fn definitions() -> Vec<Value> {
    let p = iliada_bundle::control::ControlPolicy::load_user();
    if !p.enabled || iliada_bundle::control::SystemControl::load().off {
        return Vec::new();
    }
    tools()
        .into_iter()
        .map(|(name, description, schema, _)| json!({ "name": name, "description": description, "input_schema": schema }))
        .collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// How a control action reads in Activity and in progress lines.
pub fn describe(name: &str, input: &Value) -> String {
    match name {
        "act_ui" => match s(input, "action") {
            "set_text" => format!("Type into an app: {}", s(input, "text")),
            a => format!("{} an element of an app ({})", capitalise(a), s(input, "id")),
        },
        "click" => format!("Click {}", s(input, "target")),
        "type_text" => format!("Type: {}", s(input, "text")),
        "key_combo" => format!("Press {}", s(input, "keys")),
        "scroll" => "Scroll".into(),
        "browser_open" => format!("Open {} in Homer's Chrome", s(input, "url")),
        "browser_navigate" => format!("Go to {} in Homer's Chrome", s(input, "url")),
        "browser_click" => format!("Click on a page in Homer's Chrome (element {})", s(input, "element")),
        "browser_fill" => format!("Fill in a field in Homer's Chrome: {}", s(input, "text")),
        "browser_type" => format!("Type in Homer's Chrome: {}", s(input, "text")),
        "browser_key" => format!("Press {} in Homer's Chrome", s(input, "keys")),
        "browser_close" => "Close a tab in Homer's Chrome".into(),
        other => other.to_owned(),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// What the service answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Done { text: String, image: Option<Value> },
    Confirm { question: String, token: String },
    Stopped,
    Failed(String),
}

fn socket() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::PathBuf::from(d).join("iliada-control.sock"))
}

async fn request(req: &Value) -> Result<Value> {
    request_at(&socket().context("XDG_RUNTIME_DIR is not set")?, req).await
}

async fn request_at(path: &std::path::Path, req: &Value) -> Result<Value> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .context("Homer's app control is not running (iliada-control user service)")?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{req}\n").as_bytes()).await?;
    let line = BufReader::new(r).lines().next_line().await?.context("the app control service closed")?;
    Ok(serde_json::from_str(&line)?)
}

/// Turns a service answer into an [`Answer`].
pub fn answer(v: &Value) -> Answer {
    if v["stopped"] == true {
        Answer::Stopped
    } else if let (Some(q), Some(t)) = (v["confirm"].as_str(), v["token"].as_str()) {
        Answer::Confirm { question: q.to_owned(), token: t.to_owned() }
    } else if let Some(e) = v["error"].as_str() {
        Answer::Failed(e.to_owned())
    } else {
        Answer::Done { text: s(v, "text").to_owned(), image: v.get("image").cloned() }
    }
}

/// Runs a control tool. `confirmed` is a token from a question the person
/// said yes to. Anything the model put in the input under `ctx` is ignored:
/// only the assistant says which conversation and mode this is.
pub async fn call(name: &str, input: &Value, session: &str, auto: bool, confirmed: Option<&str>) -> Answer {
    match request(&call_request(name, input, session, auto, confirmed)).await {
        Ok(v) => answer(&v),
        Err(e) => Answer::Failed(format!("{e:#}")),
    }
}

fn call_request(name: &str, input: &Value, session: &str, auto: bool, confirmed: Option<&str>) -> Value {
    json!({ "tool": name, "input": input,
            "ctx": { "session": session, "mode": if auto { "auto" } else { "ask" }, "confirmed": confirmed } })
}

/// A new message from the person: Stop no longer holds.
pub async fn begin() {
    if definitions().is_empty() {
        return;
    }
    let _ = request(&json!({ "tool": "begin" })).await;
}

/// The tool result content: text, and the image as an image block.
pub fn result_content(text: &str, image: Option<&Value>) -> Value {
    match image {
        None => json!(text),
        Some(i) => json!([
            { "type": "text", "text": text },
            { "type": "image", "source": { "type": "base64", "media_type": i["media_type"], "data": i["data"] } }
        ]),
    }
}

/// Keeps only the newest `keep` images in a conversation (older
/// screenshots are replaced by a note): they are large and stale.
pub fn prune_images(messages: &mut [Value], keep: usize) {
    let mut seen = 0;
    for m in messages.iter_mut().rev() {
        // (get_mut: indexing would add a "content" to every block.)
        let Some(blocks) = m.get_mut("content").and_then(Value::as_array_mut) else { continue };
        for b in blocks.iter_mut().rev() {
            let Some(inner) = b.get_mut("content").and_then(Value::as_array_mut) else { continue };
            for part in inner.iter_mut().rev() {
                if part["type"] == "image" {
                    seen += 1;
                    if seen > keep {
                        *part = json!({ "type": "text", "text": "(an earlier screenshot, removed to save space)" });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_and_old_screenshots_dropped() {
        assert_eq!(answer(&json!({ "stopped": true, "error": "x" })), Answer::Stopped);
        assert_eq!(
            answer(&json!({ "confirm": "Click Send?", "token": "ab" })),
            Answer::Confirm { question: "Click Send?".into(), token: "ab".into() }
        );
        assert_eq!(answer(&json!({ "error": "no" })), Answer::Failed("no".into()));
        let img = json!({ "media_type": "image/png", "data": "AAAA" });
        let block = |n| {
            json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": format!("c{n}"),
                                                  "content": result_content("shot", Some(&img)) }] })
        };
        let mut m = vec![block(1), block(2), block(3)];
        prune_images(&mut m, 2);
        assert_eq!(m[0]["content"][0]["content"][1]["type"], "text");
        assert_eq!(m[1]["content"][0]["content"][1]["type"], "image");
        assert_eq!(m[2]["content"][0]["content"][1]["source"]["data"], "AAAA");
    }

    #[tokio::test]
    async fn the_service_is_asked_over_its_socket() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("c.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (s, _) = listener.accept().await.unwrap();
                let (r, mut w) = s.into_split();
                let line = BufReader::new(r).lines().next_line().await.unwrap().unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["ctx"]["session"], "conv");
                let reply = if req["ctx"]["confirmed"] == "tok" {
                    json!({ "text": "clicked" })
                } else {
                    json!({ "confirm": "Click Send? It sends something in your name.", "token": "tok" })
                };
                w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            }
        });
        let input = json!({ "tab": "t1", "element": "3" });
        let first =
            answer(&request_at(&path, &call_request("browser_click", &input, "conv", true, None)).await.unwrap());
        let Answer::Confirm { token, .. } = first else { panic!("{first:?}") };
        let second = answer(
            &request_at(&path, &call_request("browser_click", &input, "conv", true, Some(&token))).await.unwrap(),
        );
        assert_eq!(second, Answer::Done { text: "clicked".into(), image: None });
        server.await.unwrap();
    }

    #[test]
    fn control_tools_are_named_and_described() {
        assert!(is_control("browser_click") && acts("browser_click") && !acts("browser_read"));
        assert!(!is_control("install_app"));
        assert_eq!(describe("click", &json!({ "target": "the Save button" })), "Click the Save button");
        assert_eq!(describe("act_ui", &json!({ "action": "press", "id": "e3" })), "Press an element of an app (e3)");
    }
}
