//! Date and time formatting for display.
//!
//! Domain values are always UTC (`docs/ja/develop/design/data/data-model.md`).
//! Conversion to the user's timezone happens here and nowhere else, and the
//! timezone is always an explicit argument rather than an ambient default.

use chrono::{DateTime, Local, Utc};

/// How to render a timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTimezone {
    /// Whatever the operating system reports.
    System,
    /// Always UTC. Used in tests and for diagnostics.
    Utc,
}

/// Formats an instant for display in the task list and detail pane.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use flequit_ui::adapters::datetime::{format_due, DisplayTimezone};
///
/// let due = Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 0).unwrap();
/// assert_eq!(format_due(&due, DisplayTimezone::Utc), "2026-01-15 23:59");
/// ```
pub fn format_due(value: &DateTime<Utc>, timezone: DisplayTimezone) -> String {
    match timezone {
        DisplayTimezone::Utc => value.format("%Y-%m-%d %H:%M").to_string(),
        DisplayTimezone::System => value
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn utc_formatting_is_stable() {
        let value = Utc.with_ymd_and_hms(2026, 12, 31, 9, 5, 0).unwrap();
        assert_eq!(format_due(&value, DisplayTimezone::Utc), "2026-12-31 09:05");
    }

    #[test]
    fn a_due_date_in_the_future_is_not_overdue() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let due = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        assert!(!is_overdue(Some(&due), false, &now));
    }
}
