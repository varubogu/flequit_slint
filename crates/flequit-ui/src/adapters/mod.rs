//! Conversion between domain types and the Slint UI types.
//!
//! Every function here is pure: no I/O, no facade calls, no globals. That makes
//! the display rules (date formatting, status labels, optional resolution)
//! testable without starting the UI or a database.

pub mod color;
pub mod datetime;
pub mod project;
pub mod recurrence;
pub mod tag;
pub mod task;

pub use project::to_project_item;
pub use recurrence::to_recurrence_state;
pub use tag::{to_bookmarked_tag_item, to_tag_item};
pub use task::{to_subtask_item, to_task_item};
