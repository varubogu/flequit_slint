//! The writes behind the project and task-list editor.
//!
//! Pure: the editor sends plain text, and these functions turn it into the
//! domain values a facade expects. Validation lives here too, so an empty name
//! is rejected in one place rather than in each callback.

use chrono::Utc;
use flequit_model::models::task_projects::project::{PartialProject, Project};
use flequit_model::models::task_projects::task_list::{PartialTaskList, TaskList};
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};

use crate::adapters::color::parse_hex;

/// Trims a name and rejects it when nothing is left.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::project_editor::validated_name;
///
/// assert_eq!(validated_name("  Work "), Some("Work".to_string()));
/// assert_eq!(validated_name("   "), None);
/// ```
pub fn validated_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Normalises a colour coming from the editor.
///
/// Anything that is not a hex colour becomes "no colour": the value is chosen
/// from a palette, so a different one means the UI and the domain disagree and
/// storing it would only spread the confusion.
pub fn validated_color(raw: &str) -> Option<String> {
    parse_hex(raw).map(|_| raw.trim().to_string())
}

/// A new project, ready to be created.
pub fn new_project(
    name: String,
    color: Option<String>,
    order_index: i32,
    user_id: UserId,
) -> Project {
    let now = Utc::now();
    Project {
        id: ProjectId::new(),
        name,
        description: None,
        color,
        order_index,
        is_archived: false,
        status: None,
        owner_id: Some(user_id),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

/// A new task list, ready to be created.
pub fn new_task_list(
    project_id: ProjectId,
    name: String,
    order_index: i32,
    user_id: UserId,
) -> TaskList {
    let now = Utc::now();
    TaskList {
        id: TaskListId::new(),
        project_id,
        name,
        description: None,
        color: None,
        order_index,
        is_archived: false,
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

/// The patch for renaming and recolouring a project.
///
/// The colour is always written, including when it was cleared, so that
/// choosing "no colour" is a change rather than a no-op.
pub fn project_patch(name: String, color: Option<String>) -> PartialProject {
    PartialProject {
        name: Some(name),
        color: Some(color),
        ..Default::default()
    }
}

/// The patch for archiving or unarchiving a project.
pub fn archive_patch(archived: bool) -> PartialProject {
    PartialProject {
        is_archived: Some(archived),
        ..Default::default()
    }
}

/// The patch for renaming a task list.
pub fn task_list_patch(name: String) -> PartialTaskList {
    PartialTaskList {
        name: Some(name),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_name_is_rejected() {
        for raw in ["", " ", "\t\n"] {
            assert_eq!(validated_name(raw), None);
        }
    }

    #[test]
    fn a_palette_colour_survives_validation() {
        assert_eq!(validated_color("#4c6ef5"), Some("#4c6ef5".to_string()));
        assert_eq!(validated_color("  #4c6ef5  "), Some("#4c6ef5".to_string()));
    }

    #[test]
    fn anything_that_is_not_a_colour_becomes_none() {
        for raw in ["", "blue", "#12345"] {
            assert_eq!(validated_color(raw), None);
        }
    }

    #[test]
    fn a_new_project_is_owned_by_its_author() {
        let user_id = UserId::new();

        let project = new_project("Work".to_string(), None, 2, user_id);

        assert_eq!(project.name, "Work");
        assert_eq!(project.order_index, 2);
        assert_eq!(project.owner_id, Some(user_id));
        assert_eq!(project.updated_by, user_id);
        assert!(!project.is_archived);
        assert!(!project.deleted);
    }

    #[test]
    fn a_new_task_list_belongs_to_its_project() {
        let project_id = ProjectId::new();
        let user_id = UserId::new();

        let list = new_task_list(project_id, "Inbox".to_string(), 1, user_id);

        assert_eq!(list.project_id, project_id);
        assert_eq!(list.name, "Inbox");
        assert_eq!(list.order_index, 1);
        assert_eq!(list.updated_by, user_id);
    }

    #[test]
    fn clearing_the_colour_is_written_as_a_change() {
        let patch = project_patch("Work".to_string(), None);

        assert_eq!(patch.name, Some("Work".to_string()));
        assert_eq!(patch.color, Some(None));
    }

    #[test]
    fn archiving_touches_only_the_archive_flag() {
        let patch = archive_patch(true);

        assert_eq!(patch.is_archived, Some(true));
        assert_eq!(patch.name, None);
    }
}
