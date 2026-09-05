//! Candidate list behind the font combo box.
//!
//! Like the timezone list, matching runs here: a system can carry hundreds of
//! font families, and filtering them in a `.slint` binding would walk the whole
//! list on every keystroke.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bindings::{Actions, AppWindow, SettingsState as UiSettingsState};

/// Choices that are always offered, whatever the system carries.
///
/// `default` is the family Slint picks, `system` the OS UI font. Both are
/// stored as-is, so they must not collide with a real family name.
pub(super) const BUILTIN_FONTS: &[&str] = &["default", "system"];

/// The families the platform reported, kept so the filter can rerun without
/// rescanning the font directories.
#[derive(Debug, Clone, Default)]
pub(super) struct FontCatalogue(Arc<Mutex<Vec<String>>>);

impl FontCatalogue {
    pub(super) fn replace(&self, families: Vec<String>) {
        *self.0.lock().expect("font catalogue poisoned") = families;
    }

    fn snapshot(&self) -> Vec<String> {
        self.0.lock().expect("font catalogue poisoned").clone()
    }
}

pub(super) fn bind(window: &AppWindow, catalogue: &FontCatalogue) {
    let weak = window.as_weak();
    let catalogue = catalogue.clone();
    window.global::<Actions>().on_filter_fonts(move |query| {
        if let Some(window) = weak.upgrade() {
            publish_matches(&window, &catalogue, query.as_str());
        }
    });
}

/// Fills the dropdown with every family whose name contains `query`.
pub(super) fn publish_matches(window: &AppWindow, catalogue: &FontCatalogue, query: &str) {
    let matches = filter(&catalogue.snapshot(), query)
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>();

    window
        .global::<UiSettingsState>()
        .set_font_options(ModelRc::new(VecModel::from(matches)));
}

/// Matches `families` case-insensitively, keeping the built-in choices first.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::settings::filter_fonts;
///
/// let families = ["DejaVu Sans".to_string(), "Noto Serif".to_string()];
/// assert_eq!(filter_fonts(&families, "serif"), ["Noto Serif"]);
/// assert_eq!(filter_fonts(&families, "sys"), ["system"]);
/// ```
pub fn filter(families: &[String], query: &str) -> Vec<String> {
    let needle = query.trim().to_lowercase();
    BUILTIN_FONTS
        .iter()
        .map(|name| (*name).to_string())
        .chain(families.iter().cloned())
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn families() -> Vec<String> {
        vec![
            "DejaVu Sans".to_string(),
            "Noto Sans CJK JP".to_string(),
            "Noto Serif".to_string(),
        ]
    }

    #[test]
    fn an_empty_query_lists_the_built_ins_first() {
        let matches = filter(&families(), "");

        assert_eq!(matches[0], "default");
        assert_eq!(matches[1], "system");
        assert_eq!(matches.len(), 5);
    }

    #[test]
    fn matching_ignores_case_and_position() {
        assert_eq!(filter(&families(), "NOTO").len(), 2);
        assert_eq!(filter(&families(), "cjk"), ["Noto Sans CJK JP"]);
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        assert!(filter(&families(), "comic sans").is_empty());
    }

    #[test]
    fn the_catalogue_hands_back_what_was_stored() {
        let catalogue = FontCatalogue::default();
        catalogue.replace(families());

        assert_eq!(catalogue.snapshot(), families());
    }
}
