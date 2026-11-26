//! Open windows (toplevels), as the compositor lists them, and the windows
//! that must never be shown to anyone but the person.
//!
//! Homer's hands (`iliada-control`) and Search (`iliada-searchd`) both read
//! window lists: from `lswt -j` (ext-foreign-toplevel-list), or straight
//! from the compositor. Both skip the same protected windows.

use serde_json::Value;

/// A window on the screen (from the compositor, so also windows that have
/// no accessibility).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toplevel {
    pub title: String,
    pub app_id: String,
    pub identifier: String,
}

/// The list in `lswt -j`'s output: either `{"toplevels": […]}` or a bare
/// array.
fn lswt_list(json_text: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(json_text).unwrap_or(Value::Null);
    let list = v.get("toplevels").cloned().unwrap_or(v);
    match list {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

fn field(t: &Value, keys: &[&str]) -> String {
    keys.iter().find_map(|k| t.get(*k).and_then(Value::as_str)).unwrap_or_default().to_owned()
}

const APP_ID_KEYS: [&str; 3] = ["app-id", "app_id", "appid"];

/// `lswt -j`: tolerant of the field names lswt's versions use.
pub fn parse_lswt(json_text: &str) -> Vec<Toplevel> {
    lswt_list(json_text)
        .iter()
        .map(|t| Toplevel {
            title: field(t, &["title"]),
            app_id: field(t, &APP_ID_KEYS),
            identifier: field(t, &["identifier", "id"]),
        })
        .collect()
}

/// The app id of the focused window in `lswt -j`'s output: one marked
/// `activated`, `focused`, or with either in its `state` list.
pub fn focused_app_id(json_text: &str) -> Option<String> {
    lswt_list(json_text).iter().find_map(|t| {
        let app = field(t, &APP_ID_KEYS);
        if app.is_empty() {
            return None;
        }
        let on = t.get("activated").and_then(Value::as_bool).unwrap_or(false)
            || t.get("focused").and_then(Value::as_bool).unwrap_or(false)
            || t.get("state").and_then(Value::as_array).is_some_and(|a| {
                a.iter().any(|s| {
                    s.as_str().is_some_and(|s| s.eq_ignore_ascii_case("activated") || s.eq_ignore_ascii_case("focused"))
                })
            });
        on.then_some(app)
    })
}

/// Windows that are never shown, listed or clicked through by anything
/// but the person: the prompts where the person decides (permissions, the
/// portal's consent, passwords) and Homer's own banner.
pub const PROTECTED_APP_IDS: [&str; 6] = [
    "org.iliada.Prompt",
    "iliada-prompt",
    "org.iliada.HomerBanner",
    "xdg-desktop-portal-cosmic",
    "com.system76.CosmicOsd",
    "cosmic-osd",
];

/// Whether windows of this app are protected.
pub fn is_protected(app_id: &str) -> bool {
    PROTECTED_APP_IDS.iter().any(|p| app_id.eq_ignore_ascii_case(p))
}

/// The first protected window open, if any.
pub fn protected_open(tops: &[Toplevel]) -> Option<&Toplevel> {
    tops.iter().find(|t| is_protected(&t.app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lswt_output_in_either_shape() {
        let wrapped = r#"{"toplevels":[{"title":"Inbox","app-id":"firefox","identifier":"a1"}]}"#;
        let bare = r#"[{"title":"Inbox","app_id":"firefox","id":"a1"}]"#;
        let want = vec![Toplevel { title: "Inbox".into(), app_id: "firefox".into(), identifier: "a1".into() }];
        assert_eq!(parse_lswt(wrapped), want);
        assert_eq!(parse_lswt(bare), want);
        assert!(parse_lswt("not json").is_empty());
        assert!(parse_lswt(r#"{"toplevels":{}}"#).is_empty());
    }

    #[test]
    fn focused_window() {
        let json = r#"{"toplevels":[
            {"title":"Inbox","app-id":"firefox","state":["maximized"]},
            {"title":"Notes","app-id":"com.system76.CosmicEdit","state":["Activated"]}
        ]}"#;
        assert_eq!(focused_app_id(json).as_deref(), Some("com.system76.CosmicEdit"));
        assert!(focused_app_id(r#"[{"app-id":"","focused":true}]"#).is_none());
    }

    #[test]
    fn protected_windows() {
        assert!(is_protected("org.iliada.Prompt") && is_protected("ORG.ILIADA.HOMERBANNER"));
        assert!(!is_protected("firefox") && !is_protected(""));
    }
}
