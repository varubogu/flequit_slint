use slint::ComponentHandle;

use super::locale;
use super::model::{CustomDueFilter, CustomDueUnit, RecurrencePreset};
use super::worker::SettingsQueue;
use crate::bindings::{Actions, AppWindow, DueUnit};

pub(super) fn bind(window: &AppWindow, queue: &SettingsQueue) {
    let actions = window.global::<Actions>();

    // Applying the translation before the save keeps the switch instant; the
    // publisher writes `I18n.current-locale` back from the stored value, so a
    // failed save also puts the displayed language back.
    let updater = queue.updater(window.as_weak());
    actions.on_update_language(move |locale| {
        let Some(locale) = locale::supported(locale.as_str()) else {
            tracing::warn!(%locale, "no bundled translation for locale");
            return;
        };
        apply_locale(locale);
        updater.update(move |settings| settings.language = locale.to_string());
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_week_start(move |week_start| {
        let week_start = week_start.to_string();
        if week_start == "sunday" || week_start == "monday" {
            updater.update(move |settings| settings.week_start = week_start);
        }
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_vim_mode(move |enabled| {
        updater.update(move |settings| settings.vim_mode = enabled);
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_due_button(move |key, visible| {
        let key = key.to_string();
        updater.update(move |settings| {
            if let Some(button) = settings
                .due_buttons
                .iter_mut()
                .find(|button| button.key == key)
            {
                button.visible = visible;
            }
        });
    });

    let updater = queue.updater(window.as_weak());
    actions.on_add_custom_due_filter(move |value, unit| {
        let filter = CustomDueFilter::new(value, custom_due_unit(unit));
        if !filter.is_valid() {
            return;
        }
        updater.update(move |settings| {
            if settings.custom_due_filters.len() < 20
                && !settings.custom_due_filters.contains(&filter)
            {
                settings.custom_due_filters.push(filter);
                settings.custom_due_filters.sort_unstable();
            }
        });
    });

    let updater = queue.updater(window.as_weak());
    actions.on_remove_custom_due_filter(move |value, unit| {
        let filter = CustomDueFilter::new(value, custom_due_unit(unit));
        updater.update(move |settings| {
            settings
                .custom_due_filters
                .retain(|candidate| *candidate != filter);
        });
    });

    let updater = queue.updater(window.as_weak());
    actions.on_add_recurrence_preset(move |interval, unit| {
        let preset = RecurrencePreset::new(interval, unit);
        if !preset.is_valid() {
            return;
        }
        updater.update(move |settings| {
            if settings.recurrence_presets.len() < 20
                && !settings.recurrence_presets.contains(&preset)
            {
                settings.recurrence_presets.push(preset);
                settings
                    .recurrence_presets
                    .sort_unstable_by_key(RecurrencePreset::sort_key);
            }
        });
    });

    let updater = queue.updater(window.as_weak());
    actions.on_remove_recurrence_preset(move |interval, unit| {
        let preset = RecurrencePreset::new(interval, unit);
        updater.update(move |settings| {
            settings
                .recurrence_presets
                .retain(|candidate| *candidate != preset);
        });
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_theme_mode(move |mode| {
        updater.update(move |settings| settings.theme_mode = mode);
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_font(move |font| {
        let font = font.to_string();
        updater.update(move |settings| settings.font = font);
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_font_size(move |font_size| {
        if (8..=72).contains(&font_size) {
            updater.update(move |settings| settings.font_size = font_size);
        }
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_font_color(move |color| {
        let color = color.to_string();
        updater.update(move |settings| settings.font_color = color);
    });

    let updater = queue.updater(window.as_weak());
    actions.on_update_background_color(move |color| {
        let color = color.to_string();
        updater.update(move |settings| settings.background_color = color);
    });
}

/// Selects a bundled translation catalogue.
///
/// Bundled translations re-evaluate every `@tr()` automatically, so nothing
/// else has to be refreshed.
pub(super) fn apply_locale(locale: &str) {
    if slint::select_bundled_translation(locale).is_err() {
        tracing::warn!(%locale, "could not select the bundled translation");
    }
}

fn custom_due_unit(unit: DueUnit) -> CustomDueUnit {
    match unit {
        DueUnit::Minute => CustomDueUnit::Minute,
        DueUnit::Hour => CustomDueUnit::Hour,
        DueUnit::Day => CustomDueUnit::Day,
    }
}
