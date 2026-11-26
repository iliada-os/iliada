//! Layer 1: the accessibility tree (AT-SPI over D-Bus).
//!
//! Every toolkit Iliada ships describes its windows to the accessibility
//! bus: GTK 3 and 4, Qt (with `QT_LINUX_ACCESSIBILITY_ALWAYS_ON`), Chrome
//! and Electron (`ACCESSIBILITY_ENABLED=1`) and libcosmic apps built with
//! iced's accessibility support. Homer reads a compact tree (role, name,
//! value, states, an id per element) and acts on an element by id: press,
//! focus, set its text, select it, scroll it into view.
//!
//! Sandboxed apps reach the accessibility bus only through a filtering
//! proxy (`iliada-run`): they can publish their own tree but cannot read
//! other apps' trees. Homer, outside every sandbox, reads them all, subject
//! to the person's per-app answers.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use zbus::{Connection, OwnedObjectPath, OwnedVariant as OwnedValue};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
/// The most elements one read returns: enough for a dialog or a page's
/// controls, small enough for the model.
pub const MAX_NODES: usize = 500;
const MAX_DEPTH: usize = 40;
const MAX_TEXT: i32 = 300;
/// One slow app must not hang Homer.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// An element: its bus and object path, stable while the app runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NodeRef {
    pub bus: String,
    pub path: String,
}

impl NodeRef {
    pub fn encode(&self) -> String {
        format!("{}{}", self.bus, self.path)
    }

    pub fn decode(s: &str) -> Option<Self> {
        let i = s.find('/')?;
        Some(Self { bus: s[..i].to_owned(), path: s[i..].to_owned() })
    }
}

/// An app on the accessibility bus.
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub bus: String,
    pub pid: Option<u32>,
    pub windows: Vec<String>,
    #[serde(skip)]
    pub root: NodeRef,
}

/// AT-SPI states (atspi-constants.h), the ones worth showing.
const STATES: [(u32, &str); 22] = [
    (1, "active"),
    (4, "checked"),
    (5, "collapsed"),
    (7, "editable"),
    (10, "expanded"),
    (11, "focusable"),
    (12, "focused"),
    (16, "modal"),
    (17, "multi_line"),
    (20, "pressed"),
    (22, "selectable"),
    (23, "selected"),
    (24, "sensitive"),
    (25, "showing"),
    (26, "single_line"),
    (30, "visible"),
    (32, "indeterminate"),
    (33, "required"),
    (36, "invalid_entry"),
    (41, "checkable"),
    (42, "has_popup"),
    (43, "read_only"),
];

pub fn state_names(words: &[u32]) -> Vec<&'static str> {
    let has = |bit: u32| words.get((bit / 32) as usize).is_some_and(|w| w & (1 << (bit % 32)) != 0);
    STATES.iter().filter(|(b, _)| has(*b)).map(|(_, n)| *n).collect()
}

/// Roles that are only structure: shown only when they have a name.
fn is_filler(role: &str) -> bool {
    matches!(
        role,
        "filler"
            | "panel"
            | "section"
            | "unknown"
            | "redundant object"
            | "invalid"
            | "grouping"
            | "layered pane"
            | "scroll pane"
            | "viewport"
            | "box"
            | "block quote"
            | "static"
            | "generic"
    )
}

/// The element as Homer sees it, one line.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub depth: usize,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<&'static str>,
    pub actions: Vec<String>,
    pub r: NodeRef,
}

pub struct A11y {
    bus: Connection,
}

impl std::fmt::Debug for A11y {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("A11y")
    }
}

async fn timed<T>(f: impl std::future::Future<Output = zbus::Result<T>>) -> Result<T> {
    match tokio::time::timeout(CALL_TIMEOUT, f).await {
        Ok(r) => Ok(r?),
        Err(_) => bail!("the app did not answer in time"),
    }
}

impl A11y {
    /// Connects to the accessibility bus (asking the session bus where it
    /// is), switching accessibility on for toolkits that wait to be told.
    pub async fn connect() -> Result<Self> {
        let address = match std::env::var("AT_SPI_BUS_ADDRESS") {
            Ok(a) if !a.is_empty() => a,
            _ => {
                let session = Connection::session().await.context("connecting to the session bus")?;
                // GTK 3, Qt and Chrome read this to decide whether to describe
                // themselves; GTK 4 always does.
                let _ = session
                    .call_method(
                        Some("org.a11y.Bus"),
                        "/org/a11y/bus",
                        Some("org.freedesktop.DBus.Properties"),
                        "Set",
                        &("org.a11y.Status", "IsEnabled", zbus::Variant::from(true)),
                    )
                    .await;
                let reply = session
                    .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
                    .await
                    .context("the accessibility bus is not running (at-spi2-core)")?;
                reply.body().deserialize::<String>()?
            }
        };
        let bus = zbus::connection::Builder::address(address.as_str())
            .build()
            .await
            .context("connecting to the accessibility bus")?;
        Ok(Self { bus })
    }

    async fn call<B, R>(&self, r: &NodeRef, iface: &str, method: &str, body: &B) -> Result<R>
    where
        B: serde::Serialize + zbus::Type,
        R: for<'d> serde::Deserialize<'d> + zbus::Type,
    {
        let msg = timed(self.bus.call_method(Some(r.bus.as_str()), r.path.as_str(), Some(iface), method, body)).await?;
        Ok(msg.body().deserialize::<R>()?)
    }

    async fn prop(&self, r: &NodeRef, iface: &str, name: &str) -> Result<OwnedValue> {
        self.call(r, "org.freedesktop.DBus.Properties", "Get", &(iface, name)).await
    }

    async fn string_prop(&self, r: &NodeRef, iface: &str, name: &str) -> String {
        self.prop(r, iface, name).await.ok().and_then(|v| String::try_from(v).ok()).unwrap_or_default()
    }

    pub async fn children(&self, r: &NodeRef) -> Result<Vec<NodeRef>> {
        let kids: Vec<(String, OwnedObjectPath)> = self.call(r, ACCESSIBLE, "GetChildren", &()).await?;
        Ok(kids.into_iter().map(|(bus, path)| NodeRef { bus, path: path.as_str().to_owned() }).collect())
    }

    pub async fn name(&self, r: &NodeRef) -> String {
        self.string_prop(r, ACCESSIBLE, "Name").await
    }

    pub async fn role(&self, r: &NodeRef) -> String {
        self.call::<_, String>(r, ACCESSIBLE, "GetRoleName", &()).await.unwrap_or_default()
    }

    pub async fn states(&self, r: &NodeRef) -> Vec<&'static str> {
        self.call::<_, Vec<u32>>(r, ACCESSIBLE, "GetState", &()).await.map(|w| state_names(&w)).unwrap_or_default()
    }

    pub async fn interfaces(&self, r: &NodeRef) -> Vec<String> {
        self.call(r, ACCESSIBLE, "GetInterfaces", &()).await.unwrap_or_default()
    }

    /// The process behind an app's bus name (for sandboxed apps, their
    /// accessibility proxy's).
    pub async fn pid(&self, bus: &str) -> Option<u32> {
        let msg = timed(self.bus.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &(bus,),
        ))
        .await
        .ok()?;
        msg.body().deserialize::<u32>().ok()
    }

    /// The apps on the bus, with their windows' titles.
    pub async fn apps(&self) -> Result<Vec<AppInfo>> {
        let root = NodeRef { bus: REGISTRY.into(), path: ROOT.into() };
        let mut out = Vec::new();
        for app in self.children(&root).await? {
            let name = self.name(&app).await;
            let windows = match self.children(&app).await {
                Ok(w) => {
                    let mut titles = Vec::new();
                    for win in w.iter().take(20) {
                        titles.push(self.name(win).await);
                    }
                    titles
                }
                Err(_) => continue,
            };
            out.push(AppInfo { pid: self.pid(&app.bus).await, name, bus: app.bus.clone(), windows, root: app });
        }
        Ok(out)
    }

    async fn value_of(&self, r: &NodeRef, ifaces: &[String]) -> Option<String> {
        if ifaces.iter().any(|i| i == "org.a11y.atspi.Text") {
            let count =
                self.prop(r, "org.a11y.atspi.Text", "CharacterCount").await.ok().and_then(|v| i32::try_from(v).ok());
            if let Some(n) = count.filter(|n| *n > 0) {
                let mut t: String =
                    self.call(r, "org.a11y.atspi.Text", "GetText", &(0i32, n.min(MAX_TEXT))).await.ok()?;
                if n > MAX_TEXT {
                    t.push('…');
                }
                return Some(t);
            }
        }
        if ifaces.iter().any(|i| i == "org.a11y.atspi.Value") {
            let v = self.prop(r, "org.a11y.atspi.Value", "CurrentValue").await.ok()?;
            return f64::try_from(v).ok().map(|f| f.to_string());
        }
        None
    }

    async fn action_names(&self, r: &NodeRef, ifaces: &[String]) -> Vec<String> {
        if !ifaces.iter().any(|i| i == "org.a11y.atspi.Action") {
            return Vec::new();
        }
        let acts: Vec<(String, String, String)> =
            self.call(r, "org.a11y.atspi.Action", "GetActions", &()).await.unwrap_or_default();
        acts.into_iter().map(|(name, _, _)| name).filter(|n| !n.is_empty()).collect()
    }

    /// Reads the tree under `start`: only what is showing, structure
    /// without a name folded away, at most [`MAX_NODES`] elements.
    pub async fn tree(&self, start: &NodeRef) -> Result<(Vec<Node>, bool)> {
        let mut out = Vec::new();
        let mut stack = vec![(start.clone(), 0usize)];
        let mut cut = false;
        while let Some((r, depth)) = stack.pop() {
            if out.len() >= MAX_NODES {
                cut = true;
                break;
            }
            let (role, name, states, ifaces) =
                tokio::join!(self.role(&r), self.name(&r), self.states(&r), self.interfaces(&r));
            // Hidden parts of a window (other tabs, closed menus).
            if depth > 0 && !states.is_empty() && !states.contains(&"showing") && !states.contains(&"visible") {
                continue;
            }
            let shown = depth == 0 || !(is_filler(&role) && name.trim().is_empty());
            let (value, actions) = if shown {
                tokio::join!(self.value_of(&r, &ifaces), self.action_names(&r, &ifaces))
            } else {
                (None, Vec::new())
            };
            if shown {
                out.push(Node { depth, role: role.clone(), name, value, states, actions, r: r.clone() });
            }
            if depth < MAX_DEPTH {
                let kids = self.children(&r).await.unwrap_or_default();
                let next = if shown { depth + 1 } else { depth };
                for k in kids.into_iter().rev() {
                    stack.push((k, next));
                }
            }
        }
        Ok((out, cut))
    }

    /// Runs the element's action called `wanted` ("click", "press",
    /// "activate", …), or its first action.
    pub async fn do_action(&self, r: &NodeRef, wanted: &[&str]) -> Result<String> {
        let acts: Vec<(String, String, String)> =
            self.call(r, "org.a11y.atspi.Action", "GetActions", &()).await.context("it has no actions")?;
        anyhow::ensure!(!acts.is_empty(), "it has no actions");
        let i = acts.iter().position(|(n, _, _)| wanted.iter().any(|w| n.eq_ignore_ascii_case(w))).unwrap_or(0);
        let ok: bool = self.call(r, "org.a11y.atspi.Action", "DoAction", &(i as i32,)).await?;
        anyhow::ensure!(ok, "the app refused \"{}\"", acts[i].0);
        Ok(acts[i].0.clone())
    }

    pub async fn focus(&self, r: &NodeRef) -> Result<()> {
        let ok: bool = self.call(r, "org.a11y.atspi.Component", "GrabFocus", &()).await?;
        anyhow::ensure!(ok, "the app did not move the focus");
        Ok(())
    }

    /// The element's text now (for the log and Undo).
    pub async fn text(&self, r: &NodeRef) -> Option<String> {
        let ifaces = self.interfaces(r).await;
        self.value_of(r, &ifaces).await
    }

    pub async fn set_text(&self, r: &NodeRef, text: &str) -> Result<()> {
        let ok: bool = self
            .call(r, "org.a11y.atspi.EditableText", "SetTextContents", &(text,))
            .await
            .context("this element does not take text")?;
        anyhow::ensure!(ok, "the app refused the text");
        Ok(())
    }

    /// Selects an element in its list, table or tab bar.
    pub async fn select(&self, r: &NodeRef) -> Result<()> {
        let index: i32 = self.call(r, ACCESSIBLE, "GetIndexInParent", &()).await?;
        let parent: OwnedValue = self.prop(r, ACCESSIBLE, "Parent").await?;
        let (bus, parent): (String, OwnedObjectPath) = parent.try_into().context("no parent")?;
        let parent = NodeRef { bus, path: parent.as_str().to_owned() };
        let ok: bool = match self.call(&parent, "org.a11y.atspi.Selection", "SelectChild", &(index,)).await {
            Ok(ok) => ok,
            // Not in a selection container: its own "select" or "click".
            Err(_) => return self.do_action(r, &["select", "click", "press", "activate"]).await.map(|_| ()),
        };
        anyhow::ensure!(ok, "the app did not select it");
        Ok(())
    }

    pub async fn scroll_to(&self, r: &NodeRef) -> Result<()> {
        // ATSPI_SCROLL_ANYWHERE
        let ok: bool = self.call(r, "org.a11y.atspi.Component", "ScrollTo", &(4u32,)).await?;
        anyhow::ensure!(ok, "the app did not scroll");
        Ok(())
    }
}

/// The tree as compact text: one element a line, indented, with its id.
pub fn render(nodes: &[Node], ids: &HashMap<NodeRef, String>) -> String {
    let mut s = String::new();
    for n in nodes {
        let id = ids.get(&n.r).map(String::as_str).unwrap_or("?");
        let mut line = format!("{}[{id}] {}", "  ".repeat(n.depth.min(12)), n.role);
        if !n.name.trim().is_empty() {
            line.push_str(&format!(" \"{}\"", one_line(&n.name, 120)));
        }
        if let Some(v) = &n.value
            && v.trim() != n.name.trim()
            && !v.trim().is_empty()
        {
            line.push_str(&format!(" value=\"{}\"", one_line(v, 200)));
        }
        let shown: Vec<&str> = n
            .states
            .iter()
            .copied()
            .filter(|s| !matches!(*s, "showing" | "visible" | "sensitive" | "enabled"))
            .collect();
        if !shown.is_empty() {
            line.push_str(&format!(" ({})", shown.join(",")));
        }
        if !n.states.is_empty() && !n.states.contains(&"sensitive") {
            line.push_str(" (disabled)");
        }
        if !n.actions.is_empty() {
            line.push_str(&format!(" actions={}", n.actions.join("|")));
        }
        s.push_str(&line);
        s.push('\n');
    }
    s
}

fn one_line(s: &str, max: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max { format!("{}…", t.chars().take(max).collect::<String>()) } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_come_from_both_words() {
        let words = [(1 << 12) | (1 << 25), 1 << (43 - 32)];
        assert_eq!(state_names(&words), vec!["focused", "showing", "read_only"]);
    }

    #[test]
    fn refs_round_trip_and_the_tree_renders_compactly() {
        let r = NodeRef { bus: ":1.42".into(), path: "/org/a11y/atspi/accessible/7".into() };
        assert_eq!(NodeRef::decode(&r.encode()), Some(r.clone()));
        let nodes = vec![
            Node {
                depth: 0,
                role: "frame".into(),
                name: "Mail".into(),
                value: None,
                states: vec!["active", "sensitive"],
                actions: vec![],
                r: r.clone(),
            },
            Node {
                depth: 1,
                role: "push button".into(),
                name: "Send".into(),
                value: None,
                states: vec!["focusable", "showing", "sensitive"],
                actions: vec!["click".into()],
                r: NodeRef { bus: ":1.42".into(), path: "/8".into() },
            },
            Node {
                depth: 1,
                role: "entry".into(),
                name: "To".into(),
                value: Some("ana@example.com".into()),
                states: vec!["editable"],
                actions: vec![],
                r: NodeRef { bus: ":1.42".into(), path: "/9".into() },
            },
        ];
        let ids: HashMap<NodeRef, String> =
            nodes.iter().enumerate().map(|(i, n)| (n.r.clone(), format!("e{i}"))).collect();
        assert_eq!(
            render(&nodes, &ids),
            "[e0] frame \"Mail\" (active)\n  [e1] push button \"Send\" (focusable) actions=click\n  [e2] entry \"To\" value=\"ana@example.com\" (editable) (disabled)\n"
        );
    }
}
