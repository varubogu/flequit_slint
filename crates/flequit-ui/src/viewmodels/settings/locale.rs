//! UI language selection.
//!
//! The bundled catalogues decide what can be shown, so a stored preference is
//! only a request: anything the build does not carry falls back to the system
//! locale and then to the base language.

/// Languages the bundled translations cover. The first entry is the base
/// language every `@tr()` string is written in, and therefore the last resort.
pub const SUPPORTED_LOCALES: &[&str] = &["en", "ja"];

/// Picks the language to display.
///
/// `preferred` is the stored setting, `system` whatever the OS reports (in any
/// of the usual shapes, e.g. `ja_JP.UTF-8`). Both are matched on the language
/// subtag alone: the catalogues are per language, not per region.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::settings::resolve_locale;
///
/// assert_eq!(resolve_locale("ja", Some("en_US.UTF-8")), "ja");
/// assert_eq!(resolve_locale("", Some("ja_JP.UTF-8")), "ja");
/// assert_eq!(resolve_locale("de", Some("fr_FR")), "en");
/// ```
pub fn resolve_locale(preferred: &str, system: Option<&str>) -> &'static str {
    supported(preferred)
        .or_else(|| system.and_then(supported))
        .unwrap_or(SUPPORTED_LOCALES[0])
}

/// Matches a locale string against the bundled catalogues by language subtag.
pub fn supported(locale: &str) -> Option<&'static str> {
    let language = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if language.is_empty() {
        return None;
    }
    SUPPORTED_LOCALES
        .iter()
        .find(|tag| **tag == language)
        .copied()
}

/// Reads the locale the OS reports, if any.
///
/// The environment variables are the portable answer; Slint has no API for it
/// and reaching for an OS call here would cross into `flequit-platform`'s
/// territory for a value the process already carries.
pub fn system_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty() && value != "C" && value != "POSIX")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_language_wins_over_the_system_one() {
        assert_eq!(resolve_locale("en", Some("ja_JP.UTF-8")), "en");
    }

    #[test]
    fn an_unsupported_language_falls_through_to_the_system_locale() {
        assert_eq!(resolve_locale("de", Some("ja_JP.UTF-8")), "ja");
    }

    #[test]
    fn everything_unsupported_lands_on_the_base_language() {
        assert_eq!(resolve_locale("", None), "en");
        assert_eq!(resolve_locale("zz", Some("xx_YY")), "en");
    }

    #[test]
    fn region_and_encoding_suffixes_are_ignored() {
        assert_eq!(supported("ja_JP.UTF-8"), Some("ja"));
        assert_eq!(supported("en-GB"), Some("en"));
        assert_eq!(supported("JA"), Some("ja"));
        assert_eq!(supported("ja@calendar=japanese"), Some("ja"));
    }

    #[test]
    fn an_empty_locale_matches_nothing() {
        assert_eq!(supported(""), None);
        assert_eq!(supported("  "), None);
    }
}
