//! agora in the person's language (see `iliada_settings::i18n`).

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

/// A category's name ("Developer tools": `cat-developer-tools`).
pub fn category(name: &str) -> String {
    lookup_or(&format!("cat-{}", name.to_lowercase().replace(' ', "-")), name)
}

/// What a piece of software or a service does.
pub fn summary(item: &iliada_system::software::Item) -> String {
    lookup_or(&format!("sw-{}", item.id), item.summary)
}

#[cfg(test)]
mod tests {
    use iliada_system::software::CATALOG;

    #[test]
    fn every_category_and_summary_has_a_message() {
        for c in super::super::CATEGORIES.iter().chain(CATALOG.iter().map(|i| &i.category)) {
            let id = format!("cat-{}", c.to_lowercase().replace(' ', "-"));
            assert!(super::LANGUAGE_LOADER.has(&id), "no {id}");
        }
        let english = include_str!("../i18n/en/iliada-store-gui.ftl");
        for i in CATALOG {
            // In English, the catalog's own words.
            let line = format!("sw-{} = {}", i.id, i.summary);
            assert!(english.lines().any(|l| l == line), "{line:?} is not in the English messages");
        }
    }
}
