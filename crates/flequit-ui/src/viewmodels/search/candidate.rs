//! The task shape the search matcher works on.
//!
//! Kept separate from the domain model so the matching rules can be tested with
//! literals instead of fully populated `TaskTree` values.

use chrono::{DateTime, Utc};

/// A task offered to [`super::SearchQuery::matches`].
///
/// Borrowed rather than converted: filtering runs on every keystroke, and the
/// caller already holds the loaded tree.
#[derive(Debug, Clone, Copy)]
pub struct TaskCandidate<'a> {
    pub project_name: &'a str,
    pub list_name: &'a str,
    pub title: &'a str,
    pub notes: Option<&'a str>,
    pub due: Option<&'a DateTime<Utc>>,
    pub completed: bool,
    pub subtasks: &'a [SubTaskCandidate<'a>],
    /// Resolved tag names. Empty while the tag cache is still loading.
    pub tags: &'a [&'a str],
}

#[derive(Debug, Clone, Copy)]
pub struct SubTaskCandidate<'a> {
    pub title: &'a str,
    pub notes: Option<&'a str>,
}

/// A candidate with every searchable field lowercased once.
///
/// Folding per token instead would repeat the work for every condition in the
/// query, on every task, on every keystroke.
pub(super) struct FoldedCandidate {
    pub(super) project_name: String,
    pub(super) list_name: String,
    pub(super) title: String,
    pub(super) tags: Vec<String>,
    notes: Option<String>,
    subtasks: Vec<(String, Option<String>)>,
}

impl FoldedCandidate {
    pub(super) fn notes(&self) -> Vec<&str> {
        self.notes.as_deref().into_iter().collect()
    }

    pub(super) fn subtask_titles(&self) -> Vec<&str> {
        self.subtasks
            .iter()
            .map(|(title, _)| title.as_str())
            .collect()
    }

    pub(super) fn subtask_notes(&self) -> Vec<&str> {
        self.subtasks
            .iter()
            .filter_map(|(_, notes)| notes.as_deref())
            .collect()
    }

    /// The fields a bare keyword searches: the task and its subtasks, both the
    /// title and the notes of each.
    pub(super) fn free_text(&self) -> Vec<&str> {
        let mut values = vec![self.title.as_str()];
        values.extend(self.notes());
        values.extend(self.subtask_titles());
        values.extend(self.subtask_notes());
        values
    }
}

impl From<&TaskCandidate<'_>> for FoldedCandidate {
    fn from(candidate: &TaskCandidate<'_>) -> Self {
        Self {
            project_name: candidate.project_name.to_lowercase(),
            list_name: candidate.list_name.to_lowercase(),
            title: candidate.title.to_lowercase(),
            notes: candidate.notes.map(str::to_lowercase),
            subtasks: candidate
                .subtasks
                .iter()
                .map(|sub| (sub.title.to_lowercase(), sub.notes.map(str::to_lowercase)))
                .collect(),
            tags: candidate
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect(),
        }
    }
}
