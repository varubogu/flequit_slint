//! Date and time formatting for display.
//!
//! Domain values are always UTC (`docs/ja/develop/design/data/data-model.md`).
//! Conversion to the user's timezone happens here and nowhere else, and the
//! timezone is always an explicit argument rather than an ambient default.

use std::fmt::Write;
use std::str::FromStr;

use chrono::format::StrftimeItems;
use chrono::{DateTime, Datelike, Days, Local, LocalResult, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

pub const DEFAULT_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M";

/// Calendar fields shown by the date and time pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeParts {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
}

/// How to render a timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTimezone {
    /// Whatever the operating system reports.
    System,
    /// Always UTC. Used in tests and for diagnostics.
    Utc,
    Named(Tz),
}

impl DisplayTimezone {
    pub fn from_setting(value: &str) -> Self {
        match value.trim() {
            "" | "system" => Self::System,
            "UTC" | "GMT" => Self::Utc,
            value => Tz::from_str(value).map_or(Self::System, Self::Named),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeDisplaySettings {
    pub timezone: DisplayTimezone,
    pub format: String,
}

impl DateTimeDisplaySettings {
    pub fn new(timezone: &str, format: &str) -> Self {
        Self {
            timezone: DisplayTimezone::from_setting(timezone),
            format: format.to_string(),
        }
    }

    pub fn effective_format(&self) -> &str {
        if self.format.is_empty() {
            DEFAULT_DATETIME_FORMAT
        } else {
            &self.format
        }
    }
}

/// Formats an instant for display in the task list and detail pane.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use flequit_ui::adapters::datetime::{format_due, DateTimeDisplaySettings};
///
/// let due = Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 0).unwrap();
/// let display = DateTimeDisplaySettings::new("UTC", "");
/// assert_eq!(format_due(&due, &display), "2026-01-15 23:59");
/// ```
pub fn format_due(value: &DateTime<Utc>, settings: &DateTimeDisplaySettings) -> String {
    try_format(value, settings).unwrap_or_else(|| {
        let fallback = DateTimeDisplaySettings {
            timezone: settings.timezone,
            format: DEFAULT_DATETIME_FORMAT.to_string(),
        };
        try_format(value, &fallback).expect("the built-in datetime format is valid")
    })
}

pub fn try_format(value: &DateTime<Utc>, settings: &DateTimeDisplaySettings) -> Option<String> {
    let items = StrftimeItems::new(settings.effective_format());
    let mut output = String::new();
    let result = match settings.timezone {
        DisplayTimezone::Utc => write!(&mut output, "{}", value.format_with_items(items)),
        DisplayTimezone::System => write!(
            &mut output,
            "{}",
            value.with_timezone(&Local).format_with_items(items)
        ),
        DisplayTimezone::Named(timezone) => write!(
            &mut output,
            "{}",
            value.with_timezone(&timezone).format_with_items(items)
        ),
    };
    result.ok().map(|()| output)
}

/// Splits a UTC instant into the calendar fields of the display timezone.
pub fn to_display_parts(value: &DateTime<Utc>, timezone: DisplayTimezone) -> DateTimeParts {
    match timezone {
        DisplayTimezone::Utc => parts_from(value),
        DisplayTimezone::System => parts_from(&value.with_timezone(&Local)),
        DisplayTimezone::Named(timezone) => parts_from(&value.with_timezone(&timezone)),
    }
}

/// Converts picker calendar fields in the display timezone back to UTC.
pub fn from_display_parts(
    parts: DateTimeParts,
    timezone: DisplayTimezone,
) -> Option<DateTime<Utc>> {
    let month = u32::try_from(parts.month).ok()?;
    let day = u32::try_from(parts.day).ok()?;
    let hour = u32::try_from(parts.hour).ok()?;
    let minute = u32::try_from(parts.minute).ok()?;

    match timezone {
        DisplayTimezone::Utc => Utc
            .with_ymd_and_hms(parts.year, month, day, hour, minute, 0)
            .single(),
        DisplayTimezone::System => {
            resolve_local(Local.with_ymd_and_hms(parts.year, month, day, hour, minute, 0))
        }
        DisplayTimezone::Named(timezone) => {
            resolve_timezone(timezone.with_ymd_and_hms(parts.year, month, day, hour, minute, 0))
        }
    }
}

fn parts_from<Tz: TimeZone>(value: &DateTime<Tz>) -> DateTimeParts {
    DateTimeParts {
        year: value.year(),
        month: value.month() as i32,
        day: value.day() as i32,
        hour: value.hour() as i32,
        minute: value.minute() as i32,
    }
}

fn resolve_local(result: LocalResult<DateTime<Local>>) -> Option<DateTime<Utc>> {
    match result {
        LocalResult::Single(value) | LocalResult::Ambiguous(value, _) => {
            Some(value.with_timezone(&Utc))
        }
        LocalResult::None => None,
    }
}

fn resolve_timezone(result: LocalResult<DateTime<Tz>>) -> Option<DateTime<Utc>> {
    match result {
        LocalResult::Single(value) | LocalResult::Ambiguous(value, _) => {
            Some(value.with_timezone(&Utc))
        }
        LocalResult::None => None,
    }
}

/// Whether a due date has passed relative to `now`.
///
/// Completed tasks are never overdue; the caller passes `completed` so this
/// stays a single decision point.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use flequit_ui::adapters::datetime::is_overdue;
///
/// let now = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
/// let due = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
/// assert!(is_overdue(Some(&due), false, &now));
/// assert!(!is_overdue(Some(&due), true, &now));
/// assert!(!is_overdue(None, false, &now));
/// ```
pub fn is_overdue(due: Option<&DateTime<Utc>>, completed: bool, now: &DateTime<Utc>) -> bool {
    match due {
        Some(due) if !completed => due < now,
        _ => false,
    }
}

/// The last instant of the day `days_ahead` days after `now`, in display time.
///
/// Due-date filters are expressed in calendar days ("due today", "due within a
/// week"), so the boundary has to be the end of a local day rather than a fixed
/// number of hours from now.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use flequit_ui::adapters::datetime::{end_of_day_after, DisplayTimezone};
///
/// let now = Utc.with_ymd_and_hms(2026, 1, 15, 9, 0, 0).unwrap();
/// assert_eq!(
///     end_of_day_after(&now, 0, DisplayTimezone::Utc),
///     Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 59).unwrap()
/// );
/// ```
pub fn end_of_day_after(
    now: &DateTime<Utc>,
    days_ahead: u64,
    timezone: DisplayTimezone,
) -> DateTime<Utc> {
    match timezone {
        DisplayTimezone::Utc => end_of_day_in(&Utc, now.with_timezone(&Utc), days_ahead),
        DisplayTimezone::System => end_of_day_in(&Local, now.with_timezone(&Local), days_ahead),
        DisplayTimezone::Named(timezone) => {
            end_of_day_in(&timezone, now.with_timezone(&timezone), days_ahead)
        }
    }
}

/// Resolves 23:59:59 of `now + days_ahead` in `tz` back to UTC.
///
/// A daylight-saving jump can make that wall-clock time ambiguous or
/// non-existent; both cases fall back to an instant on the intended day rather
/// than failing, since a filter boundary must always produce an answer.
fn end_of_day_in<Tz: TimeZone>(tz: &Tz, now: DateTime<Tz>, days_ahead: u64) -> DateTime<Utc> {
    let Some(day) = now.date_naive().checked_add_days(Days::new(days_ahead)) else {
        // Only reachable for a horizon of millions of days; treat it as "no bound".
        return DateTime::<Utc>::MAX_UTC;
    };
    let local = day
        .and_hms_opt(23, 59, 59)
        .expect("23:59:59 is a valid wall-clock time");

    match tz.from_local_datetime(&local) {
        LocalResult::Single(value) | LocalResult::Ambiguous(_, value) => value.with_timezone(&Utc),
        LocalResult::None => now
            .offset()
            .fix()
            .from_local_datetime(&local)
            .single()
            .expect("a fixed offset always resolves a local time")
            .with_timezone(&Utc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn utc_formatting_is_stable() {
        let value = Utc.with_ymd_and_hms(2026, 12, 31, 9, 5, 0).unwrap();
        let display = DateTimeDisplaySettings::new("UTC", "");
        assert_eq!(format_due(&value, &display), "2026-12-31 09:05");
    }

    #[test]
    fn named_timezone_and_custom_format_are_applied() {
        let value = Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 0).unwrap();
        let display = DateTimeDisplaySettings::new("Asia/Tokyo", "%Y/%m/%d %H:%M");

        assert_eq!(format_due(&value, &display), "2026/01/16 08:59");
        assert_eq!(
            to_display_parts(&value, display.timezone),
            DateTimeParts {
                year: 2026,
                month: 1,
                day: 16,
                hour: 8,
                minute: 59,
            }
        );
    }

    #[test]
    fn invalid_custom_format_falls_back_for_due_labels() {
        let value = Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 0).unwrap();
        let display = DateTimeDisplaySettings::new("UTC", "%Q");

        assert!(try_format(&value, &display).is_none());
        assert_eq!(format_due(&value, &display), "2026-01-15 23:59");
    }

    #[test]
    fn a_due_date_in_the_future_is_not_overdue() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let due = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        assert!(!is_overdue(Some(&due), false, &now));
    }

    #[test]
    fn utc_parts_round_trip_without_losing_minutes() {
        let value = Utc.with_ymd_and_hms(2026, 8, 9, 14, 35, 0).unwrap();
        let parts = to_display_parts(&value, DisplayTimezone::Utc);

        assert_eq!(
            parts,
            DateTimeParts {
                year: 2026,
                month: 8,
                day: 9,
                hour: 14,
                minute: 35,
            }
        );
        assert_eq!(from_display_parts(parts, DisplayTimezone::Utc), Some(value));
    }

    #[test]
    fn the_end_of_today_keeps_the_calendar_day() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 0, 30, 0).unwrap();

        assert_eq!(
            end_of_day_after(&now, 0, DisplayTimezone::Utc),
            Utc.with_ymd_and_hms(2026, 3, 1, 23, 59, 59).unwrap()
        );
    }

    #[test]
    fn a_horizon_crosses_into_the_following_month() {
        let now = Utc.with_ymd_and_hms(2026, 1, 30, 12, 0, 0).unwrap();

        assert_eq!(
            end_of_day_after(&now, 3, DisplayTimezone::Utc),
            Utc.with_ymd_and_hms(2026, 2, 2, 23, 59, 59).unwrap()
        );
    }

    #[test]
    fn invalid_picker_fields_are_rejected() {
        let invalid = DateTimeParts {
            year: 2026,
            month: 2,
            day: 30,
            hour: 12,
            minute: 0,
        };

        assert_eq!(from_display_parts(invalid, DisplayTimezone::Utc), None);
    }
}
