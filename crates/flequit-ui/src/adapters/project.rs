//! `ProjectTree` → Slint UI types.

use flequit_model::models::task_projects::project::ProjectTree;
use slint::{Brush, ModelRc, SharedString, VecModel};

use super::color::parse_hex;
use crate::bindings::{ProjectItem, TaskListItem};

/// Number of leading characters shown when the sidebar is collapsed and the
/// project has no icon.
const SHORT_LABEL_LEN: usize = 2;

/// Builds the fallback label shown for a project in the collapsed sidebar.
///
/// Uses characters rather than bytes so multi-byte names are not split.
///
/// # Examples
///
/// ```
/// use flequit_ui::adapters::project::short_label;
///
/// assert_eq!(short_label("My Tasks"), "My");
/// assert_eq!(short_label("プロジェクト"), "プロ");
/// assert_eq!(short_label(""), "?");
/// ```
pub fn short_label(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "?".to_string();
    }
    trimmed.chars().take(SHORT_LABEL_LEN).collect()
}

/// Converts a project and its task lists for the sidebar.
///
/// Task counts come from the tree that was already loaded, so no extra query is
/// needed to render them.
pub fn to_project_item(project: &ProjectTree, expanded: bool) -> ProjectItem {
    let task_lists: Vec<TaskListItem> = project
        .task_lists
        .iter()
        .filter(|list| !list.deleted && !list.is_archived)
        .map(|list| TaskListItem {
            id: SharedString::from(list.id.as_str()),
            project_id: SharedString::from(list.project_id.as_str()),
            name: SharedString::from(list.name.clone()),
            task_count: list
                .tasks
                .iter()
                .filter(|task| !task.deleted && !task.is_archived)
                .count() as i32,
        })
        .collect();

    // An unparseable colour is treated as unset rather than as black: the
    // value comes from storage and may predate the current palette.
    let parsed = project.color.as_deref().and_then(parse_hex);
    let color = parsed.and(project.color.clone()).unwrap_or_default();

    ProjectItem {
        id: SharedString::from(project.id.as_str()),
        name: SharedString::from(project.name.clone()),
        short_label: SharedString::from(short_label(&project.name)),
        has_color: parsed.is_some(),
        color: SharedString::from(color),
        color_brush: parsed.map(Brush::from).unwrap_or_default(),
        is_archived: project.is_archived,
        task_lists: ModelRc::new(VecModel::from(task_lists)),
        expanded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_label_ignores_surrounding_whitespace() {
        assert_eq!(short_label("  Work  "), "Wo");
    }

    #[test]
    fn short_label_handles_names_shorter_than_the_limit() {
        assert_eq!(short_label("A"), "A");
    }
}
