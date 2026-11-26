//! The Notification Center in the person's language: English and Greek.

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader.load_fallback_language(&Localizations).expect("the English messages are built in");
    let _ = i18n_embed::select(&loader, &Localizations, &DesktopLanguageRequester::requested_languages());
    loader.set_use_isolating(false);
    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id)
    }};
    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

/// "5 minutes ago".
pub fn ago(at: u64) -> String {
    if at == 0 {
        return String::new();
    }
    let secs = iliada_notify::now().saturating_sub(at);
    match secs {
        s if s < 90 => crate::fl!("ago-now"),
        s if s < 90 * 60 => {
            let count = s / 60;
            crate::fl!("ago-minutes", count = count)
        }
        s if s < 36 * 3600 => {
            let count = s / 3600;
            crate::fl!("ago-hours", count = count)
        }
        s => {
            let count = s / 86400;
            crate::fl!("ago-days", count = count)
        }
    }
}

/// Whether Do Not Disturb is on, in words ("On until 07:00").
pub fn dnd_status(status: Option<iliada_notify::dnd::Why>) -> String {
    use iliada_notify::dnd::{Why, clock_time};
    match status {
        None => crate::fl!("dnd-off"),
        Some(Why::Manual) => crate::fl!("dnd-on"),
        Some(Why::Until { until }) => crate::fl!("dnd-on-until", time = clock_time(until)),
        Some(Why::Schedule { until }) => crate::fl!("dnd-on-schedule", time = clock_time(until)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_in_both_languages() {
        let keys = |lang: &str| -> Vec<String> {
            let path = format!("{}/i18n/{lang}/iliada-notification-center.ftl", env!("CARGO_MANIFEST_DIR"));
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .filter_map(|l| l.split_once(" = ").map(|(k, _)| k.trim().to_owned()))
                .filter(|k| !k.starts_with('#') && !k.starts_with(' '))
                .collect()
        };
        assert_eq!(keys("en"), keys("el"));
    }

    #[test]
    fn do_not_disturb_in_words() {
        assert!(!dnd_status(None).is_empty());
        assert!(!dnd_status(Some(iliada_notify::dnd::Why::Manual)).is_empty());
        assert!(ago(0).is_empty());
        assert!(!ago(iliada_notify::now()).is_empty());
    }
}
