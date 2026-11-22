//! UI translations, from `locales/app.yml`. gpui-kit translates its own
//! widgets with the same rust-i18n locale, so one switch covers both.

use crate::settings::Language;
use gpui_kit::SharedString;

/// The translation of `key` in the current language.
pub fn tr(key: &str) -> SharedString {
    rust_i18n::t!(key).to_string().into()
}

/// Switch the UI to `language` (or the system's, for `Language::System`).
pub fn apply(language: Language) {
    rust_i18n::set_locale(resolve(language, system_locale().as_deref()));
}

fn resolve(language: Language, system: Option<&str>) -> &'static str {
    match language {
        Language::English => "en",
        Language::Japanese => "ja",
        Language::System => match system {
            Some(locale) if locale.starts_with("ja") => "ja",
            _ => "en",
        },
    }
}

/// The user's language, looked up like gettext does: `LANGUAGE` (its first
/// entry), then `LC_ALL`, `LC_MESSAGES` and `LANG`; empty values are skipped.
fn system_locale() -> Option<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|value| value.split(':').next().unwrap_or_default().to_string())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_follows_the_locale() {
        assert_eq!(resolve(Language::System, Some("ja_JP.UTF-8")), "ja");
        assert_eq!(resolve(Language::System, Some("fr_FR.UTF-8")), "en");
        assert_eq!(resolve(Language::System, Some("C")), "en");
        assert_eq!(resolve(Language::System, None), "en");
        assert_eq!(resolve(Language::English, Some("ja_JP.UTF-8")), "en");
        assert_eq!(resolve(Language::Japanese, None), "ja");
    }

    /// Every key in the locale file has its own English and Japanese text.
    #[test]
    fn every_string_is_translated() {
        let file = include_str!("../locales/app.yml");
        let keys: Vec<&str> = file
            .lines()
            .filter(|line| !line.starts_with([' ', '#', '_']) && line.ends_with(':'))
            .map(|line| line.trim_end_matches(':'))
            .collect();
        assert!(keys.len() > 50, "found only {} keys", keys.len());
        for key in keys {
            let en = rust_i18n::t!(key, locale = "en");
            let ja = rust_i18n::t!(key, locale = "ja");
            assert_ne!(en, key, "{key} has no English text");
            assert_ne!(ja, en, "{key} has no Japanese text");
        }
    }
}
