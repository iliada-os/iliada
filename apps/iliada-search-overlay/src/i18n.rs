//! Iliada's search overlay in the person's language: English and Greek.

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

pub fn ago(at: u64) -> String {
    if at == 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(at);
    ago_from(now.saturating_sub(at))
}

fn ago_from(secs: u64) -> String {
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

pub fn kind_label(kind: iliada_search::Kind) -> String {
    use iliada_search::Kind::*;
    match kind {
        App => crate::fl!("kind-app"),
        Folder => crate::fl!("kind-folder"),
        File => crate::fl!("kind-file"),
        Setting => crate::fl!("kind-setting"),
        Action => crate::fl!("kind-action"),
        Agora => crate::fl!("kind-agora"),
        Answer => crate::fl!("kind-answer"),
        Homer => crate::fl!("kind-homer"),
        Web => crate::fl!("kind-web"),
        Command => crate::fl!("kind-command"),
        Clipboard => crate::fl!("kind-clipboard"),
        Window => crate::fl!("kind-window"),
        Extension => crate::fl!("kind-extension"),
    }
}
