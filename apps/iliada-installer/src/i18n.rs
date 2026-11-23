//! The installer in the language chosen on its first page (at first the
//! live system's), as Iliada's other apps do it (`iliada_settings::i18n`).

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

/// A message whose id is only known at run time, or `fallback`.
pub fn lookup_or(id: &str, fallback: &str) -> String {
    if LANGUAGE_LOADER.has(id) { LANGUAGE_LOADER.get(id) } else { fallback.to_owned() }
}

/// Shows everything in another language from now on (`en`, `el`).
pub fn choose(language: &str) {
    if let Ok(id) = language.parse::<i18n_embed::unic_langid::LanguageIdentifier>() {
        let _ = i18n_embed::select(&*LANGUAGE_LOADER, &Localizations, &[id]);
    }
}

/// The language shown now (`en`, `el`).
pub fn current() -> String {
    LANGUAGE_LOADER.current_language().language.as_str().to_owned()
}
