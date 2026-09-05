//! Candidate list behind the timezone combo box.
//!
//! Matching runs here rather than in `.slint` because the IANA database has
//! several hundred entries: rebuilding a filtered model in a binding would walk
//! the whole list on every keystroke, and `.slint` has no way to express the
//! case-insensitive substring match the field needs.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bindings::{Actions, AppWindow, SettingsState as UiSettingsState};

/// Accepted alongside the IANA names, mirroring `DisplayTimezone::from_setting`.
const SHORTHANDS: &[&str] = &["system", "UTC"];

pub(super) fn bind(window: &AppWindow) {
    let weak = window.as_weak();
    window
        .global::<Actions>()
        .on_filter_timezones(move |query| {
            if let Some(window) = weak.upgrade() {
                publish_matches(&window, query.as_str());
            }
        });
}

/// Fills the dropdown with every timezone whose name contains `query`.
///
/// An empty query lists all of them, which is what opening the dropdown without
/// typing should show.
pub(super) fn publish_matches(window: &AppWindow, query: &str) {
    let needle = query.trim().to_lowercase();
    let matches = SHORTHANDS
        .iter()
        .copied()
        .chain(
            chrono_tz::TZ_VARIANTS
                .iter()
                .map(|timezone| timezone.name()),
        )
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .map(SharedString::from)
        .collect::<Vec<_>>();

    window
        .global::<UiSettingsState>()
        .set_timezone_options(ModelRc::new(VecModel::from(matches)));
}
