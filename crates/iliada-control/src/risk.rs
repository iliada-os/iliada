//! Which actions are consequential: they send, submit, delete, buy or
//! post something in the person's name. Those always wait for the person's
//! yes, in Ask and Auto mode alike, whatever a page or app says.
//!
//! The words come from what Homer is about to press (a button's name, a
//! link's text, a form's submit button) or, for a click at a point on the
//! screen, from what Homer says it is clicking.

/// A kind of consequential action, with how it reads ("send").
pub const KINDS: [(&str, &[&str]); 5] = [
    (
        "send",
        &[
            "send",
            "reply",
            "forward",
            "invite",
            "share",
            "transfer",
            "αποστολή",
            "αποστολη",
            "στείλε",
            "απάντηση",
            "κοινοποίηση",
        ],
    ),
    (
        "submit",
        &["submit", "confirm", "sign up", "register", "save changes", "accept", "agree", "υποβολή", "επιβεβαίωση"],
    ),
    (
        "delete",
        &[
            "delete",
            "remove",
            "erase",
            "trash",
            "discard",
            "empty",
            "uninstall",
            "wipe",
            "clear all",
            "format",
            "διαγραφή",
            "αφαίρεση",
        ],
    ),
    (
        "purchase",
        &[
            "buy",
            "purchase",
            "pay",
            "checkout",
            "check out",
            "place order",
            "order now",
            "subscribe",
            "donate",
            "book now",
            "add card",
            "αγορά",
            "πληρωμή",
            "παραγγελία",
        ],
    ),
    ("post", &["post", "publish", "tweet", "upload", "comment", "go live", "δημοσίευση"]),
];

/// The kind of consequential action `label` (a button's name, a link's
/// text, Homer's description of a click) is, if any.
pub fn classify(label: &str) -> Option<&'static str> {
    let l = label.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let joined = words.join(" ");
    for (kind, needles) in KINDS {
        for n in needles {
            let hit = if n.contains(' ') {
                joined.contains(n)
            } else {
                // Whole words: "Sender" is not a send.
                words.iter().any(|w| w == n)
            };
            if hit {
                return Some(kind);
            }
        }
    }
    None
}

/// Key presses that commonly send or submit: Enter in a single-line field,
/// Ctrl+Enter in a message box.
pub fn classify_keys(keys: &str, in_form_field: bool) -> Option<&'static str> {
    let k = keys.to_lowercase().replace(' ', "");
    let enter = k.ends_with("enter") || k.ends_with("return");
    if enter && (k.contains("ctrl") || k.contains("control") || k.contains("super") || k.contains("alt")) {
        return Some("send");
    }
    if enter && in_form_field {
        return Some("submit");
    }
    if k.ends_with("delete") && k.contains("shift") {
        return Some("delete");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consequential_labels_are_recognised() {
        assert_eq!(classify("Send"), Some("send"));
        assert_eq!(classify("  Reply all "), Some("send"));
        assert_eq!(classify("Place order"), Some("purchase"));
        assert_eq!(classify("Move to Trash"), Some("delete"));
        assert_eq!(classify("Post"), Some("post"));
        assert_eq!(classify("Αποστολή"), Some("send"));
        assert_eq!(classify("Submit form"), Some("submit"));
        assert_eq!(classify("the blue Pay button"), Some("purchase"));
        assert_eq!(classify("Bold"), None);
        assert_eq!(classify("Sender name"), None);
        assert_eq!(classify("Payload viewer"), None);
        assert_eq!(classify("Search"), None);
        assert_eq!(classify("Untitled document"), None);
    }

    #[test]
    fn sending_keys_are_recognised() {
        assert_eq!(classify_keys("ctrl+Enter", false), Some("send"));
        assert_eq!(classify_keys("Enter", true), Some("submit"));
        assert_eq!(classify_keys("Enter", false), None);
        assert_eq!(classify_keys("ctrl+b", true), None);
        assert_eq!(classify_keys("shift+Delete", false), Some("delete"));
    }
}
