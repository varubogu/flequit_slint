//! Conversion between domain types and the Slint UI types.
//!
//! Every function here is pure: no I/O, no facade calls, no globals. That makes
//! the display rules (date formatting, status labels, optional resolution)
//! testable without starting the UI or a database.

pub mod datetime;
pub mod project;
pub mod task;

pub use project::to_project_item;
pub use task::{to_subtask_item, to_task_item};
