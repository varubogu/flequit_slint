//! Parsing and evaluation of the search box query.
//!
//! The syntax is specified in `docs/ja/develop/design/ui/page/main/main.md`:
//! `@keyword` narrows by due date or by an attribute (`@list:inbox`), `#name`
//! narrows by tag, and anything else is matched as free text. Tokens are
//! separated by whitespace and combined with AND, so each one narrows further.
//!
//! Everything here is pure so the rules can be tested without a window.

mod candidate;
pub mod due;
mod suggestion;

use chrono::{DateTime, Utc};

use crate::adapters::datetime::DisplayTimezone;
use candidate::FoldedCandidate;
pub use candidate::{SubTaskCandidate, TaskCandidate};
pub use due::DueKeyword;
pub use suggestion::{SuggestionKind, SuggestionSources, suggestions};

/// A parsed query. Every field holds lowercased tokens that must all match.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    due: Vec<DueKeyword>,
    project: Vec<String>,
    list: Vec<String>,
    task: Vec<String>,
    note: Vec<String>,
    subtask: Vec<String>,
    subtask_note: Vec<String>,
    tags: Vec<String>,
    text: Vec<String>,
}

impl SearchQuery {
    /// Splits the search text into conditions.
    ///
    /// Unrecognised keywords are dropped rather than treated as free text: the
    /// user is typing `@to` on the way to `@today`, and matching that literally
    /// would empty the list mid-word. `@name` is also reserved for account names
    /// in the specification, which are not resolvable yet.
    pub fn parse(input: &str) -> Self {
        let mut query = Self::default();
        for token in input.split_whitespace() {
            query.push_token(&token.to_lowercase());
        }
        query
    }

    /// Whether no condition was given, in which case everything matches.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn push_token(&mut self, token: &str) {
        if let Some(tag) = token.strip_prefix('#') {
            if !tag.is_empty() {
                self.tags.push(tag.to_string());
            }
            return;
        }

        let Some(keyword) = token.strip_prefix('@') else {
            self.text.push(token.to_string());
            return;
        };

        if let Some((field, value)) = keyword.split_once(':') {
            if let Some(target) = self.field(field)
                && !value.is_empty()
            {
                target.push(value.to_string());
            }
            return;
        }

        if let Some(due) = DueKeyword::parse(keyword) {
            self.due.push(due);
        }
    }

    /// The condition list an `@field:value` token writes to.
    fn field(&mut self, name: &str) -> Option<&mut Vec<String>> {
        match name {
            "proj" | "project" => Some(&mut self.project),
            "list" => Some(&mut self.list),
            "task" => Some(&mut self.task),
            "note" => Some(&mut self.note),
            "subtask" => Some(&mut self.subtask),
            "subtasknote" => Some(&mut self.subtask_note),
            _ => None,
        }
    }

    /// Whether a task satisfies every condition in the query.
    pub fn matches(
        &self,
        candidate: &TaskCandidate<'_>,
        now: &DateTime<Utc>,
        timezone: DisplayTimezone,
    ) -> bool {
        if self.is_empty() {
            return true;
        }

        let due_matches = self
            .due
            .iter()
            .all(|keyword| keyword.matches(candidate.due, candidate.completed, now, timezone));
        if !due_matches {
            return false;
        }

        let folded = FoldedCandidate::from(candidate);
        // Tags are matched by prefix, mirroring how the suggestion list narrows
        // as the name is typed.
        self.tags
            .iter()
            .all(|tag| folded.tags.iter().any(|name| name.starts_with(tag)))
            && all_contained(&self.project, &[folded.project_name.as_str()])
            && all_contained(&self.list, &[folded.list_name.as_str()])
            && all_contained(&self.task, &[folded.title.as_str()])
            && all_contained(&self.note, &folded.notes())
            && all_contained(&self.subtask, &folded.subtask_titles())
            && all_contained(&self.subtask_note, &folded.subtask_notes())
            && all_contained(&self.text, &folded.free_text())
    }
}

/// Whether every needle appears in at least one of the haystacks.
fn all_contained(needles: &[String], haystacks: &[&str]) -> bool {
    needles.iter().all(|needle| {
        haystacks
            .iter()
            .any(|value| value.contains(needle.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap()
    }

    /// A task with a subtask, a tag and a due date two days out.
    fn candidate<'a>(
        subtasks: &'a [SubTaskCandidate<'a>],
        tags: &'a [&'a str],
    ) -> TaskCandidate<'a> {
        TaskCandidate {
            project_name: "Home",
            list_name: "Inbox",
            title: "Buy milk",
            notes: Some("Semi-skimmed"),
            due: None,
            completed: false,
            subtasks,
            tags,
        }
    }

    fn matches(query: &str, candidate: &TaskCandidate<'_>) -> bool {
        SearchQuery::parse(query).matches(candidate, &now(), DisplayTimezone::Utc)
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches("", &candidate(&[], &[])));
        assert!(matches("   ", &candidate(&[], &[])));
    }

    #[test]
    fn free_text_matches_the_title_and_notes_case_insensitively() {
        let task = candidate(&[], &[]);

        assert!(matches("MILK", &task));
        assert!(matches("skimmed", &task));
        assert!(!matches("bread", &task));
    }

    #[test]
    fn free_text_also_reaches_into_subtasks() {
        let subtasks = [SubTaskCandidate {
            title: "Check the fridge",
            notes: Some("bottom shelf"),
        }];
        let task = candidate(&subtasks, &[]);

        assert!(matches("fridge", &task));
        assert!(matches("shelf", &task));
    }

    #[test]
    fn attribute_keywords_narrow_by_their_own_field() {
        let subtasks = [SubTaskCandidate {
            title: "Check the fridge",
            notes: Some("bottom shelf"),
        }];
        let task = candidate(&subtasks, &[]);

        assert!(matches("@project:home", &task));
        assert!(matches("@proj:home @list:inbox", &task));
        assert!(matches("@task:milk", &task));
        assert!(matches("@note:semi", &task));
        assert!(matches("@subtask:fridge", &task));
        assert!(matches("@subtasknote:shelf", &task));

        // The value has to be in that field, not merely somewhere in the task.
        assert!(!matches("@list:home", &task));
        assert!(!matches("@note:fridge", &task));
    }

    #[test]
    fn conditions_are_combined_with_and() {
        let task = candidate(&[], &[]);

        assert!(matches("@list:inbox milk", &task));
        assert!(!matches("@list:inbox bread", &task));
    }

    #[test]
    fn a_tag_is_matched_by_prefix() {
        let task = candidate(&[], &["Shopping"]);

        assert!(matches("#shop", &task));
        assert!(matches("#shopping", &task));
        assert!(!matches("#work", &task));
        // A lone "#" is still being typed and must not hide everything.
        assert!(matches("#", &task));
    }

    #[test]
    fn a_due_keyword_filters_on_the_due_date() {
        let due = Utc.with_ymd_and_hms(2026, 3, 2, 15, 0, 0).unwrap();
        let task = TaskCandidate {
            due: Some(&due),
            ..candidate(&[], &[])
        };

        assert!(matches("@tomorrow", &task));
        assert!(!matches("@today", &task));
        assert!(matches("@week milk", &task));
    }

    #[test]
    fn a_half_typed_keyword_does_not_hide_the_list() {
        let task = candidate(&[], &[]);

        // Neither an unfinished due keyword nor a reserved one filters anything.
        assert!(matches("@", &task));
        assert!(matches("@to", &task));
        assert!(matches("@someone", &task));
        assert!(matches("@unknown:value", &task));
    }
}
