use chrono::Utc;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::datetime_format;
use super::locale::{resolve_locale, system_locale};
use super::model::{CustomDueUnit, DateTimeFormatKind, UserSettings};
use super::mutations::apply_locale;
use crate::adapters::datetime::{DateTimeDisplaySettings, to_display_parts};
use crate::bindings::{
    AppState, AppWindow, CustomDueFilterSetting, DateTimeFormatSetting, DueButtonSetting,
    DueFilterItem, DueUnit, I18n, RecurrencePresetSetting, SettingsState as UiSettingsState, Theme,
};

pub(super) fn publish(window: &AppWindow, settings: &UserSettings) {
    let ui = window.global::<UiSettingsState>();
    publish_locale(window, settings);
    ui.set_week_start(settings.week_start.clone().into());
    ui.set_due_buttons(ModelRc::new(VecModel::from(
        settings
            .due_buttons
            .iter()
            .map(|button| DueButtonSetting {
                key: button.key.clone().into(),
                visible: button.visible,
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_custom_due_filters(ModelRc::new(VecModel::from(
        settings
            .custom_due_filters
            .iter()
            .map(|filter| CustomDueFilterSetting {
                value: filter.value,
                unit: due_unit(filter.unit),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_recurrence_presets(ModelRc::new(VecModel::from(
        settings
            .recurrence_presets
            .iter()
            .map(|preset| RecurrencePresetSetting {
                interval: preset.interval,
                unit: preset.unit,
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_timezone(settings.timezone.clone().into());
    ui.set_current_datetime_format(settings.datetime_format.format.clone().into());
    publish_datetime_formats(window, settings);
    ui.set_theme_mode(settings.theme_mode);
    ui.set_font(settings.font.clone().into());
    ui.set_font_size(settings.font_size);
    ui.set_font_color(settings.font_color.clone().into());
    ui.set_background_color(settings.background_color.clone().into());

    let theme = window.global::<Theme>();
    theme.set_mode(settings.theme_mode);
    theme.set_font_choice(settings.font.clone().into());
    theme.set_font_size_base(settings.font_size as f32);
    theme.set_font_color_choice(settings.font_color.clone().into());
    theme.set_background_color_choice(settings.background_color.clone().into());
    publish_due_filters(window, settings);
}

/// Selects the language and mirrors the resolved tag into `I18n`.
///
/// Resolving here rather than at startup means a stored language that the build
/// no longer carries is corrected on every publish, not just the first one.
fn publish_locale(window: &AppWindow, settings: &UserSettings) {
    let locale = resolve_locale(&settings.language, system_locale().as_deref());
    let i18n = window.global::<I18n>();
    if i18n.get_current_locale() == locale {
        return;
    }
    apply_locale(locale);
    i18n.set_current_locale(locale.into());
}

fn publish_datetime_formats(window: &AppWindow, settings: &UserSettings) {
    let i18n = window.global::<I18n>();
    let formats = datetime_format::all_formats(&settings.datetime_formats);
    let display_names = formats
        .iter()
        .map(|format| {
            i18n.invoke_datetime_format_name(format.name.clone().into(), format.name.clone().into())
        })
        .collect::<Vec<_>>();
    let ids = formats
        .iter()
        .map(|format| SharedString::from(format.id.clone()))
        .collect::<Vec<_>>();
    let values = formats
        .iter()
        .map(|format| SharedString::from(format.format.clone()))
        .collect::<Vec<_>>();
    let kinds = formats
        .iter()
        .map(|format| SharedString::from(kind_name(format.kind)))
        .collect::<Vec<_>>();
    let items = formats
        .iter()
        .zip(display_names.iter())
        .map(|(format, name)| DateTimeFormatSetting {
            id: format.id.clone().into(),
            name: name.clone(),
            format: format.format.clone().into(),
            kind: kind_name(format.kind).into(),
        })
        .collect::<Vec<_>>();

    let ui = window.global::<UiSettingsState>();
    ui.set_datetime_formats(ModelRc::new(VecModel::from(items)));
    ui.set_datetime_format_names(ModelRc::new(VecModel::from(display_names)));
    ui.set_datetime_format_ids(ModelRc::new(VecModel::from(ids)));
    ui.set_datetime_format_values(ModelRc::new(VecModel::from(values)));
    ui.set_datetime_format_kinds(ModelRc::new(VecModel::from(kinds)));

    if !ui.get_preview_initialized() {
        let display =
            DateTimeDisplaySettings::new(&settings.timezone, &settings.datetime_format.format);
        let parts = to_display_parts(&Utc::now(), display.timezone);
        ui.set_preview_year(parts.year);
        ui.set_preview_month(parts.month);
        ui.set_preview_day(parts.day);
        ui.set_preview_hour(parts.hour);
        ui.set_preview_minute(parts.minute);
        ui.set_preview_initialized(true);
    }
}

fn due_unit(unit: CustomDueUnit) -> DueUnit {
    match unit {
        CustomDueUnit::Minute => DueUnit::Minute,
        CustomDueUnit::Hour => DueUnit::Hour,
        CustomDueUnit::Day => DueUnit::Day,
    }
}

fn kind_name(kind: DateTimeFormatKind) -> &'static str {
    match kind {
        DateTimeFormatKind::Default => "default",
        DateTimeFormatKind::Preset => "preset",
        DateTimeFormatKind::Custom => "custom",
        DateTimeFormatKind::CustomFormat => "custom-format",
    }
}

fn publish_due_filters(window: &AppWindow, settings: &UserSettings) {
    let current = window.global::<AppState>().get_due_filters();
    let count_for = |key: &str| {
        (0..current.row_count())
            .filter_map(|index| current.row_data(index))
            .find(|item| item.key.as_str() == key)
            .map_or(0, |item| item.count)
    };

    let mut filters = settings
        .due_buttons
        .iter()
        .map(|button| DueFilterItem {
            key: button.key.clone().into(),
            label: button.key.clone().into(),
            query: button.query.clone().into(),
            count: count_for(&button.key),
            visible: button.visible,
            custom_value: 0,
            custom_unit: DueUnit::Day,
        })
        .collect::<Vec<_>>();

    filters.extend(settings.custom_due_filters.iter().map(|filter| {
        let key = filter.key();
        DueFilterItem {
            key: key.clone().into(),
            label: key.clone().into(),
            query: filter.query().into(),
            count: count_for(&key),
            visible: true,
            custom_value: filter.value,
            custom_unit: due_unit(filter.unit),
        }
    }));

    window
        .global::<AppState>()
        .set_due_filters(ModelRc::new(VecModel::from(filters)));
}
