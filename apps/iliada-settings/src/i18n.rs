//! Iliada's apps in the person's language: English and Greek (Ελληνικά).
//!
//! As COSMIC's apps do it: Fluent files (`i18n/<lang>/<crate>.ftl`) built
//! into the program, the language picked from the locale (`LANG`), English
//! for anything missing. `fl!("key")` gives a message; agora and the
//! installer have files of their own.

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// The messages, in the language the locale asks for (loaded on first use,
/// also when COSMIC Settings shows Iliada's pages).
pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader.load_fallback_language(&Localizations).expect("the English messages are built in");
    let _ = i18n_embed::select(&loader, &Localizations, &DesktopLanguageRequester::requested_languages());
    // No invisible direction marks around values ("{ $name }"): Greek and
    // English both run left to right.
    loader.set_use_isolating(false);
    loader
});

/// A message by its id, in the person's language: `fl!("updates")`,
/// `fl!("signed-in-as", account = name)`.
#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id)
    }};
    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

/// A message whose id is only known at run time (a service's or a
/// permission's name); None when there is no such message.
pub fn lookup(id: &str) -> Option<String> {
    LANGUAGE_LOADER.has(id).then(|| LANGUAGE_LOADER.get(id))
}

/// The language the messages are shown in ("en", "el").
pub fn language() -> String {
    LANGUAGE_LOADER.current_language().language.as_str().to_owned()
}

/// Whether Iliada is shown in Greek.
pub fn is_greek() -> bool {
    language() == "el"
}

/// The languages as their speakers write them ("English", "Ελληνικά"), in
/// the order of `iliada_system::settings::LANGUAGES`.
pub static LANGUAGE_NAMES: LazyLock<Vec<String>> =
    LazyLock::new(|| iliada_system::settings::LANGUAGES.iter().map(|(_, n)| (*n).to_owned()).collect());

/// "Ελληνικά" for `el_GR.UTF-8`.
pub fn language_name(locale: &str) -> &'static str {
    iliada_system::settings::LANGUAGES.iter().find(|(l, _)| *l == locale).map_or("English", |(_, n)| n)
}

/// Whether this session already shows `locale`'s language (`LANG` is set
/// at login, so a new one shows after logging in again).
pub fn session_has_language(locale: &str) -> bool {
    let lang = std::env::var("LANG").unwrap_or_default();
    let base = |l: &str| l.split(['_', '.']).next().unwrap_or("").to_owned();
    let plain = lang.is_empty() || lang == "C" || lang.starts_with("C.") || lang == "POSIX";
    base(&lang) == base(locale) || (plain && base(locale) == "en")
}

/// What the system service says it is doing (`Request::activity`, in
/// English), in the person's language.
pub fn activity(english: &str) -> String {
    match english {
        "Installing tools" => fl!("activity-installing-tools"),
        "Removing tools" => fl!("activity-removing-tools"),
        "Applying settings" => fl!("activity-applying"),
        "Saving assistant settings" => fl!("activity-assistant"),
        "Changing the language" => fl!("activity-language"),
        "Updating apps" => fl!("activity-updating-apps"),
        "Updating Iliada" => fl!("activity-updating-iliada"),
        "Changing how Iliada updates" => fl!("activity-update-channel"),
        "Signing in to GitHub" => fl!("activity-github-sign-in"),
        "Backing up" => fl!("activity-backing-up"),
        "Restoring a backup" => fl!("activity-restoring"),
        "Signing out" => fl!("activity-signing-out"),
        "Bringing back your apps and tools" => fl!("activity-restore-settings"),
        "" => fl!("activity-working"),
        other => other.to_owned(),
    }
}

/// "5 minutes ago", "πριν από 5 λεπτά".
pub fn ago(t: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(t);
    ago_from(now.saturating_sub(t))
}

fn ago_from(secs: u64) -> String {
    match secs {
        s if s < 90 => fl!("ago-now"),
        s if s < 90 * 60 => {
            let count = s / 60;
            fl!("ago-minutes", count = count)
        }
        s if s < 36 * 3600 => {
            let count = s / 3600;
            fl!("ago-hours", count = count)
        }
        s => {
            let count = s / 86400;
            fl!("ago-days", count = count)
        }
    }
}

/// A version as people read it: "0.2.7", or for a test update
/// "0.2.7, test update of 3 Nov 2026 14:05 UTC" (iliada_system::update::describe).
pub fn describe(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if let [a, b, c, stamp] = parts.as_slice()
        && stamp.len() == 12
        && stamp.bytes().all(|d| d.is_ascii_digit())
        && let Ok(month @ 1..=12) = stamp[4..6].parse::<u32>()
    {
        let day = stamp[6..8].trim_start_matches('0');
        let date = format!("{day} {} {} {}:{} UTC", month_name(month), &stamp[..4], &stamp[8..10], &stamp[10..]);
        return fl!("test-update-version", version = format!("{a}.{b}.{c}"), date = date);
    }
    version.to_owned()
}

fn month_name(m: u32) -> String {
    match m {
        1 => fl!("month-1"),
        2 => fl!("month-2"),
        3 => fl!("month-3"),
        4 => fl!("month-4"),
        5 => fl!("month-5"),
        6 => fl!("month-6"),
        7 => fl!("month-7"),
        8 => fl!("month-8"),
        9 => fl!("month-9"),
        10 => fl!("month-10"),
        11 => fl!("month-11"),
        _ => fl!("month-12"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    fn ids(ftl: &str) -> BTreeSet<String> {
        ftl.lines()
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_owned()))
            .collect()
    }

    /// Every message is in both languages, and every file parses.
    #[test]
    fn greek_has_every_english_message() {
        for dir in [
            concat!(env!("CARGO_MANIFEST_DIR"), "/i18n"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../iliada-store-gui/i18n"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../iliada-installer/i18n"),
        ] {
            for entry in std::fs::read_dir(format!("{dir}/en")).unwrap() {
                let name = entry.unwrap().file_name();
                let en = std::fs::read_to_string(format!("{dir}/en/{}", name.to_string_lossy())).unwrap();
                let el = std::fs::read_to_string(format!("{dir}/el/{}", name.to_string_lossy())).unwrap();
                for (lang, text) in [("en", &en), ("el", &el)] {
                    if let Err((_, errors)) = fluent_syntax::parser::parse(text.as_str()) {
                        panic!("{dir}/{lang}/{name:?}: {errors:?}");
                    }
                }
                let (en_ids, el_ids) = (ids(&en), ids(&el));
                let missing: Vec<_> = en_ids.difference(&el_ids).collect();
                let extra: Vec<_> = el_ids.difference(&en_ids).collect();
                assert!(missing.is_empty(), "{dir}/el/{name:?} lacks {missing:?}");
                assert!(extra.is_empty(), "{dir}/el/{name:?} has messages English lacks: {extra:?}");
            }
        }
    }

    #[test]
    fn times_read_like_a_person_would_say_them() {
        assert_eq!(super::ago_from(10), crate::fl!("ago-now"));
        assert!(super::ago_from(5 * 60).contains('5'));
        assert_eq!(super::describe("0.2.7"), "0.2.7");
        assert!(super::describe("0.2.7.202611031405").contains("2026 14:05 UTC"));
    }
}
