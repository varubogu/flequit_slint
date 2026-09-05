//! Accordion state for the task list.
//!
//! Expansion is tracked here rather than inside `.slint` because the task
//! detail pane also expands rows (when navigating to a subtask), and because
//! the state is a future persistence target.

use std::collections::HashSet;

/// Which task rows are currently expanded.
#[derive(Debug, Default)]
pub struct TaskListUiViewModel {
    expanded: HashSet<String>,
}

impl TaskListUiViewModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `task_id`'s subtasks are shown.
    ///
    /// # Examples
    ///
    /// ```
    /// use flequit_ui::viewmodels::TaskListUiViewModel;
    ///
    /// let mut ui = TaskListUiViewModel::new();
    /// assert!(!ui.is_expanded("t1"));
    /// ui.expand("t1");
    /// assert!(ui.is_expanded("t1"));
    /// ```
    pub fn is_expanded(&self, task_id: &str) -> bool {
        self.expanded.contains(task_id)
    }

    /// Flips the expansion state and reports the new value.
    pub fn toggle(&mut self, task_id: &str) -> bool {
        if self.expanded.remove(task_id) {
            false
        } else {
            self.expanded.insert(task_id.to_string());
            true
        }
    }

    pub fn expand(&mut self, task_id: &str) {
        self.expanded.insert(task_id.to_string());
    }

    pub fn collapse(&mut self, task_id: &str) {
        self.expanded.remove(task_id);
    }

    /// Clears all expansion state.
    pub fn reset(&mut self) {
        self.expanded.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_reports_the_resulting_state() {
        let mut ui = TaskListUiViewModel::new();
        assert!(ui.toggle("t1"));
        assert!(ui.is_expanded("t1"));
        assert!(!ui.toggle("t1"));
        assert!(!ui.is_expanded("t1"));
    }

    #[test]
    fn expanding_twice_is_idempotent() {
        let mut ui = TaskListUiViewModel::new();
        ui.expand("t1");
        ui.expand("t1");
        ui.collapse("t1");
        assert!(!ui.is_expanded("t1"));
    }

    #[test]
    fn reset_clears_everything() {
        let mut ui = TaskListUiViewModel::new();
        ui.expand("a");
        ui.expand("b");
        ui.reset();
        assert!(!ui.is_expanded("a"));
        assert!(!ui.is_expanded("b"));
    }
}
