//! Local notifications, used by the reminder feature.
//!
//! Mobile platforms require an explicit permission grant before the first
//! notification, and may schedule notifications through the OS rather than the
//! process — do not assume the app is running when one fires.

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationId(pub String);

/// Result of asking the OS for notification permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    /// The user has not been asked yet.
    NotDetermined,
}
