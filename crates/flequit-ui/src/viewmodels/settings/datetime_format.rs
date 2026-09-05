use slint::ComponentHandle;
use uuid::Uuid;

use super::model::{DateTimeFormatKind, DateTimeFormatPreference};
use super::worker::SettingsQueue;
use crate::adapters::datetime::{
    DateTimeDisplaySettings, DateTimeParts, from_display_parts, try_format,
};
use crate::bindings::{Actions, AppWindow};

pub(super) fn built_in_formats() -> Vec<DateTimeFormatPreference> {
    vec![
        format("-1", "default", "", DateTimeFormatKind::Default, 0),
        format(
            "jp-0",
            "japanese",
            "%Y年%m月%d日 %H:%M:%S",
            DateTimeFormatKind::Preset,
            10,
        ),
        format(
            "en-0",
            "american",
            "%m/%d/%Y %H:%M:%S",
            DateTimeFormatKind::Preset,
            20,
        ),
        format(
            "iso-0",
            "iso",
            "%Y-%m-%dT%H:%M:%S%:z",
            DateTimeFormatKind::Preset,
            30,
        ),
        format("-2", "custom", "", DateTimeFormatKind::Custom, 40),
    ]
}

pub(super) fn all_formats(custom: &[DateTimeFormatPreference]) -> Vec<DateTimeFormatPreference> {
    let mut formats = built_in_formats();
    formats.extend(custom.iter().cloned());
    formats
}

pub(super) fn bind(window: &AppWindow, queue: &SettingsQueue) {
    let actions = window.global::<Actions>();

    let updater = queue.updater(window.as_weak());
    let weak = window.as_weak();
    actions.on_update_timezone(move |timezone| {
        let timezone = timezone.trim().to_string();
        if !timezone.is_empty() {
            updater.update(move |settings| settings.timezone = timezone);
            refresh_display(&weak);
        }
    });

    let updater = queue.updater(window.as_weak());
    let weak = window.as_weak();
    actions.on_update_datetime_format(move |value| {
        let value = value.to_string();
        updater.update(move |settings| {
            settings.datetime_format = all_formats(&settings.datetime_formats)
                .into_iter()
                .find(|candidate| candidate.format == value)
                .unwrap_or_else(|| format("-2", "custom", &value, DateTimeFormatKind::Custom, 40));
        });
        refresh_display(&weak);
    });

    let updater = queue.updater(window.as_weak());
    let weak = window.as_weak();
    actions.on_add_datetime_format(move |name, value| {
        let name = name.trim().to_string();
        let value = value.to_string();
        if name.is_empty() {
            return;
        }
        updater.update(move |settings| {
            let Some(id) = (0..10)
                .map(|_| Uuid::new_v4().to_string())
                .find(|id| !settings.datetime_formats.iter().any(|item| item.id == *id))
            else {
                tracing::error!("could not allocate a unique datetime format id");
                return;
            };
            let order = settings
                .datetime_formats
                .iter()
                .map(|item| item.order)
                .max()
                .unwrap_or(99)
                + 1;
            let added = format(&id, &name, &value, DateTimeFormatKind::CustomFormat, order);
            settings.datetime_format = added.clone();
            settings.datetime_formats.push(added);
        });
        refresh_display(&weak);
    });

    let updater = queue.updater(window.as_weak());
    let weak = window.as_weak();
    actions.on_overwrite_datetime_format(move |id, name, value| {
        let id = id.to_string();
        let name = name.trim().to_string();
        let value = value.to_string();
        if name.is_empty() {
            return;
        }
        updater.update(move |settings| {
            let Some(existing) = settings
                .datetime_formats
                .iter_mut()
                .find(|item| item.id == id)
            else {
                return;
            };
            existing.name = name;
            existing.format = value;
            settings.datetime_format = existing.clone();
        });
        refresh_display(&weak);
    });

    let updater = queue.updater(window.as_weak());
    let weak = window.as_weak();
    actions.on_delete_datetime_format(move |id| {
        let id = id.to_string();
        updater.update(move |settings| {
            settings.datetime_formats.retain(|item| item.id != id);
            if settings.datetime_format.id == id {
                settings.datetime_format = DateTimeFormatPreference::default();
            }
        });
        refresh_display(&weak);
    });

    let formats_reader = queue.clone_reader();
    actions.on_datetime_format_index(move |value| {
        all_formats(&formats_reader())
            .iter()
            .position(|candidate| candidate.format == value.as_str())
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(4)
    });

    let settings_reader = queue.settings_reader();
    actions.on_preview_datetime(move |value, year, month, day, hour, minute| {
        let settings = settings_reader();
        let display = DateTimeDisplaySettings::new(&settings.timezone, &value);
        let parts = DateTimeParts {
            year,
            month,
            day,
            hour,
            minute,
        };
        from_display_parts(parts, display.timezone)
            .and_then(|datetime| try_format(&datetime, &display))
            .unwrap_or_default()
            .into()
    });
}

fn refresh_display(weak: &slint::Weak<AppWindow>) {
    if let Some(window) = weak.upgrade() {
        window.global::<Actions>().invoke_refresh_datetime_display();
    }
}

fn format(
    id: &str,
    name: &str,
    value: &str,
    kind: DateTimeFormatKind,
    order: i32,
) -> DateTimeFormatPreference {
    DateTimeFormatPreference {
        id: id.to_string(),
        name: name.to_string(),
        format: value.to_string(),
        kind,
        order,
    }
}
