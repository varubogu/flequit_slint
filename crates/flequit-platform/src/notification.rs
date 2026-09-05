//! Local notifications, used by the reminder feature.
//!
//! Mobile platforms require an explicit permission grant before the first
//! notification, and may schedule notifications through the OS rather than the
//! process — do not assume the app is running when one fires.

use chrono::{DateTime, Utc};

/// A notification to display to the user.
#[derive(Debug, Clone)]
pub struct NotificationRequest {
    pub title: String,
    pub body: String,
    /// Domain identifier (e.g. a task id) echoed back when the user taps the
    /// notification, so the app can navigate to the right place.
    pub payload: Option<String>,
}

/// Handle to a delivered or scheduled notification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotificationId(pub String);

impl NotificationId {
    pub fn scheduled(payload: &str, scheduled_at: &DateTime<Utc>) -> Self {
        Self(format!("{payload}@{}", scheduled_at.to_rfc3339()))
    }
}

/// Result of asking the OS for notification permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    /// The user has not been asked yet.
    NotDetermined,
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::NotificationId;

    #[test]
    fn scheduled_notification_id_is_stable_and_timestamp_specific() {
        let first = Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 9, 7, 12, 5, 0).unwrap();

        assert_eq!(
            NotificationId::scheduled("task-1", &first),
            NotificationId::scheduled("task-1", &first)
        );
        assert_ne!(
            NotificationId::scheduled("task-1", &first),
            NotificationId::scheduled("task-1", &second)
        );
    }
}
