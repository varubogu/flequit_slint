//! Reminder calendar constraints, evaluated in the configured display timezone.

use chrono::{DateTime, NaiveDate, Utc};
use slint::ComponentHandle;

use super::worker::SettingsQueue;
use crate::adapters::datetime::{
    DateTimeParts, DisplayTimezone, from_display_parts, to_display_parts,
};
use crate::bindings::{Actions, AppWindow};

pub(super) fn bind(window: &AppWindow, queue: &SettingsQueue) {
    let reader = queue.settings_reader();
    window
        .global::<Actions>()
        .on_reminder_date_selectable(move |year, month, day| {
            let timezone = DisplayTimezone::from_setting(&reader().timezone);
            date_selectable(year, month, day, timezone, &Utc::now())
        });
    let reader = queue.settings_reader();
    window
        .global::<Actions>()
        .on_reminder_datetime_valid(move |year, month, day, hour, minute| {
            let timezone = DisplayTimezone::from_setting(&reader().timezone);
            datetime_valid(
                DateTimeParts {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                },
                timezone,
                &Utc::now(),
            )
        });
}

fn date_selectable(
    year: i32,
    month: i32,
    day: i32,
    timezone: DisplayTimezone,
    now: &DateTime<Utc>,
) -> bool {
    let Some(_) = u32::try_from(month)
        .ok()
        .zip(u32::try_from(day).ok())
        .and_then(|(month, day)| NaiveDate::from_ymd_opt(year, month, day))
    else {
        return false;
    };
    let today = to_display_parts(now, timezone);
    (year, month, day) >= (today.year, today.month, today.day)
}

fn datetime_valid(parts: DateTimeParts, timezone: DisplayTimezone, now: &DateTime<Utc>) -> bool {
    from_display_parts(parts, timezone).is_some_and(|value| value > *now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn selectable_days_follow_the_display_timezone() {
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 16, 0, 0).unwrap();
        let tokyo = DisplayTimezone::from_setting("Asia/Tokyo");
        assert!(!date_selectable(2026, 9, 9, tokyo, &now));
        assert!(date_selectable(2026, 9, 10, tokyo, &now));
        assert!(date_selectable(2026, 9, 11, tokyo, &now));
        assert!(date_selectable(2026, 9, 9, DisplayTimezone::Utc, &now));
        assert!(!date_selectable(2026, 2, 30, tokyo, &now));
    }

    #[test]
    fn only_strictly_future_instants_are_accepted() {
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 16, 0, 30).unwrap();
        let timezone = DisplayTimezone::from_setting("Asia/Tokyo");
        let parts = DateTimeParts {
            year: 2026,
            month: 9,
            day: 10,
            hour: 1,
            minute: 0,
        };
        assert!(!datetime_valid(parts, timezone, &now));
        assert!(datetime_valid(
            DateTimeParts { minute: 1, ..parts },
            timezone,
            &now
        ));
        assert!(!datetime_valid(
            DateTimeParts { day: 9, ..parts },
            timezone,
            &now
        ));
        assert!(!datetime_valid(
            DateTimeParts { hour: 24, ..parts },
            timezone,
            &now
        ));
        let exact = Utc.with_ymd_and_hms(2026, 9, 9, 16, 0, 0).unwrap();
        assert!(!datetime_valid(parts, timezone, &exact));
    }

    #[test]
    fn nonexistent_daylight_saving_times_are_rejected() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        let parts = DateTimeParts {
            year: 2026,
            month: 3,
            day: 8,
            hour: 2,
            minute: 30,
        };
        assert!(!datetime_valid(
            parts,
            DisplayTimezone::from_setting("America/New_York"),
            &now
        ));
    }
}
