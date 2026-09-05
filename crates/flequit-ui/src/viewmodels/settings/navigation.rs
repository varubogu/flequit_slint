use slint::{ComponentHandle, SharedString};

use crate::bindings::{
    Actions, AppState, AppWindow, SettingsCategory, SettingsState as UiSettingsState,
};

pub(super) fn bind(window: &AppWindow) {
    let actions = window.global::<Actions>();

    let weak = window.as_weak();
    actions.on_open_settings(move || {
        if let Some(window) = weak.upgrade() {
            window.global::<AppState>().set_sidebar_open(false);
            let settings = window.global::<UiSettingsState>();
            settings.set_open(true);
            settings.set_selected_category(SettingsCategory::Basic);
            settings.set_compact_detail_open(false);
            settings.set_search_query(SharedString::default());
            settings.set_show_basic(true);
            settings.set_show_date_time(true);
            settings.set_show_appearance(true);
            settings.set_show_account(true);
        }
    });

    let weak = window.as_weak();
    actions.on_close_settings(move || {
        if let Some(window) = weak.upgrade() {
            window.global::<UiSettingsState>().set_open(false);
        }
    });

    let weak = window.as_weak();
    actions.on_select_settings_category(move |category| {
        if let Some(window) = weak.upgrade() {
            let settings = window.global::<UiSettingsState>();
            settings.set_selected_category(category);
            settings.set_compact_detail_open(true);
        }
    });

    let weak = window.as_weak();
    actions.on_search_settings(move |query, basic, date_time, appearance, account| {
        if let Some(window) = weak.upgrade() {
            let settings = window.global::<UiSettingsState>();
            let query = query.trim().to_lowercase();
            settings.set_search_query(query.clone().into());
            settings.set_show_basic(matches_search(&query, &basic));
            settings.set_show_date_time(matches_search(&query, &date_time));
            settings.set_show_appearance(matches_search(&query, &appearance));
            settings.set_show_account(matches_search(&query, &account));
        }
    });
}

fn matches_search(query: &str, haystack: &SharedString) -> bool {
    query.is_empty() || haystack.to_lowercase().contains(query)
}
