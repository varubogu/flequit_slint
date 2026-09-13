use super::model::{
    BUILTIN_DUE_FILTERS, CustomDueFilter, CustomDueUnit, DateTimeFormatKind,
    DateTimeFormatPreference, DueButtonPreference, SettingsModel, UserSettings,
};

#[test]
fn normalize_fills_defaults_and_sanitizes_custom_due_filters() {
    let settings = UserSettings {
        due_buttons: vec![DueButtonPreference {
            key: "today".to_string(),
            query: "@today".to_string(),
            visible: false,
            name: "  きょう  ".to_string(),
        }],
        custom_due_filters: vec![
            CustomDueFilter::new(5, CustomDueUnit::Day),
            CustomDueFilter::new(-1, CustomDueUnit::Day),
            CustomDueFilter::new(5, CustomDueUnit::Day),
            CustomDueFilter::new(4000, CustomDueUnit::Day),
            CustomDueFilter::new(2, CustomDueUnit::Day),
            CustomDueFilter::new(90, CustomDueUnit::Minute),
            // Out of range for hours, so it is dropped like the 4000-day one.
            CustomDueFilter::new(9000, CustomDueUnit::Hour),
        ],
        ..UserSettings::default()
    }
    .normalize();

    assert_eq!(settings.due_buttons.len(), BUILTIN_DUE_FILTERS.len());
    assert!(!settings.due_buttons[1].visible);
    assert_eq!(settings.due_buttons[1].name, "きょう", "names are trimmed");
    assert_eq!(
        settings.custom_due_filters,
        [
            CustomDueFilter::new(90, CustomDueUnit::Minute),
            CustomDueFilter::new(2, CustomDueUnit::Day),
            CustomDueFilter::new(5, CustomDueUnit::Day),
        ],
        "invalid entries are dropped and the rest sort shortest horizon first"
    );
}

#[test]
fn filters_differing_only_by_name_are_one_filter() {
    let long_name = "あ".repeat(60);
    let settings = UserSettings {
        custom_due_filters: vec![
            CustomDueFilter::named(90, CustomDueUnit::Day, "今期".to_string()),
            CustomDueFilter::named(90, CustomDueUnit::Day, "今Q".to_string()),
            CustomDueFilter::named(3, CustomDueUnit::Day, long_name),
        ],
        ..UserSettings::default()
    }
    .normalize();

    assert_eq!(settings.custom_due_filters.len(), 2);
    assert_eq!(
        settings.custom_due_filters[1].name, "今期",
        "the first name written wins"
    );
    assert_eq!(
        settings.custom_due_filters[0].name.chars().count(),
        super::model::MAX_NAME_CHARS
    );
}

#[test]
fn normalize_keeps_only_custom_datetime_formats_in_order() {
    let settings = UserSettings {
        datetime_formats: vec![
            DateTimeFormatPreference {
                id: "second".to_string(),
                name: "Second".to_string(),
                format: "%Y".to_string(),
                kind: DateTimeFormatKind::CustomFormat,
                order: 2,
            },
            DateTimeFormatPreference::default(),
            DateTimeFormatPreference {
                id: "first".to_string(),
                name: "First".to_string(),
                format: "%m".to_string(),
                kind: DateTimeFormatKind::CustomFormat,
                order: 1,
            },
        ],
        ..UserSettings::default()
    }
    .normalize();

    assert_eq!(
        settings
            .datetime_formats
            .iter()
            .map(|format| format.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn rollback_restores_the_last_persisted_revision() {
    let original = UserSettings::default();
    let mut model = SettingsModel::new(original.clone());

    let (first, first_revision) = model
        .update(|settings| settings.week_start = "monday".to_string())
        .expect("the first update should change state");
    model.mark_persisted(first);
    let (_, second_revision) = model
        .update(|settings| settings.font_size = 20)
        .expect("the second update should change state");

    assert!(model.rollback_if_current(first_revision).is_none());
    let rolled_back = model
        .rollback_if_current(second_revision)
        .expect("the current failed revision should roll back");
    assert_eq!(rolled_back.week_start, "monday");
    assert_eq!(rolled_back.font_size, original.font_size);
}

#[test]
fn reminder_choices_are_sanitized_without_restoring_deleted_defaults() {
    use super::model::ReminderPreset;
    use crate::bindings::ReminderUnit::{Day, Hour, Minute};

    let settings = UserSettings {
        reminder_presets: vec![
            ReminderPreset::new(2, Day),
            ReminderPreset::new(90, Minute),
            ReminderPreset::new(1, Hour),
            ReminderPreset::new(2, Day),
            ReminderPreset::new(0, Minute),
            ReminderPreset::new(3651, Day),
        ],
        ..UserSettings::default()
    }
    .normalize();
    assert_eq!(
        settings.reminder_presets,
        [
            ReminderPreset::new(1, Hour),
            ReminderPreset::new(90, Minute),
            ReminderPreset::new(2, Day),
        ]
    );
    let mut model = SettingsModel::new(settings.clone());
    let (_, revision) = model
        .update(|settings| settings.reminder_presets.clear())
        .unwrap();
    assert!(
        model
            .current
            .clone()
            .normalize()
            .reminder_presets
            .is_empty()
    );
    assert_eq!(
        model
            .rollback_if_current(revision)
            .unwrap()
            .reminder_presets,
        settings.reminder_presets
    );
}
