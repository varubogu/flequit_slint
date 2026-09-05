//! Search candidates shown while the final query token begins with `@` or `#`.

use std::collections::HashSet;

/// The group a suggestion belongs to in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuggestionKind {
    Due,
    Project,
    TaskList,
    Account,
    Tag,
}

/// A token replacement plus the complete query produced by selecting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub query: String,
    pub replacement: String,
    pub kind: SuggestionKind,
}

/// Names that may be offered for the active query token.
pub struct SuggestionSources<'a> {
    pub projects: &'a [String],
    pub task_lists: &'a [String],
    pub tags: &'a [String],
    pub account: Option<&'a str>,
}

const DUE_TOKENS: &[&str] = &[
    "@overdue",
    "@期限切れ",
    "@today",
    "@今日",
    "@tomorrow",
    "@明日",
    "@3days",
    "@明後日",
    "@week",
    "@今週",
    "@month",
    "@今月",
    "@quarter",
    "@今期",
    "@year",
    "@今年",
    "@fiscalyear",
    "@今年度",
];

/// Returns candidates for the last whitespace-separated token in `query`.
///
/// `@` candidates use partial matching. Tag candidates use prefix matching,
/// mirroring the actual `#tag` search semantics.
pub fn suggestions(query: &str, sources: &SuggestionSources<'_>) -> Vec<Suggestion> {
    let (query_prefix, token) = active_token(query);
    if token.starts_with('#') {
        return tag_suggestions(query_prefix, token, sources.tags);
    }
    if !token.starts_with('@') {
        return Vec::new();
    }

    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for replacement in DUE_TOKENS {
        push_at_match(
            &mut found,
            &mut seen,
            query_prefix,
            token,
            replacement,
            replacement.trim_start_matches('@'),
            SuggestionKind::Due,
        );
    }

    for name in sorted_unique(sources.projects) {
        let replacement = format!("@project:{name}");
        push_at_match(
            &mut found,
            &mut seen,
            query_prefix,
            token,
            &replacement,
            name,
            SuggestionKind::Project,
        );
    }
    for name in sorted_unique(sources.task_lists) {
        let replacement = format!("@list:{name}");
        push_at_match(
            &mut found,
            &mut seen,
            query_prefix,
            token,
            &replacement,
            name,
            SuggestionKind::TaskList,
        );
    }
    if let Some(name) = sources.account.filter(|name| !name.trim().is_empty()) {
        let replacement = format!("@{name}");
        push_at_match(
            &mut found,
            &mut seen,
            query_prefix,
            token,
            &replacement,
            name,
            SuggestionKind::Account,
        );
    }

    found
}

fn active_token(query: &str) -> (&str, &str) {
    let Some((index, separator)) = query
        .char_indices()
        .rfind(|(_, value)| value.is_whitespace())
    else {
        return ("", query);
    };
    let start = index + separator.len_utf8();
    (&query[..start], &query[start..])
}

fn tag_suggestions(query_prefix: &str, token: &str, tags: &[String]) -> Vec<Suggestion> {
    let needle = token.trim_start_matches('#').to_lowercase();
    sorted_unique(tags)
        .into_iter()
        .filter(|name| name.to_lowercase().starts_with(&needle))
        .map(|name| {
            let replacement = format!("#{name}");
            Suggestion {
                query: format!("{query_prefix}{replacement}"),
                replacement,
                kind: SuggestionKind::Tag,
            }
        })
        .collect()
}

fn push_at_match(
    found: &mut Vec<Suggestion>,
    seen: &mut HashSet<String>,
    query_prefix: &str,
    token: &str,
    replacement: &str,
    name: &str,
    kind: SuggestionKind,
) {
    let needle = token.trim_start_matches('@').to_lowercase();
    let replacement_key = replacement.to_lowercase();
    let field_value = token.split_once(':').map(|(_, value)| value.to_lowercase());
    let matches = needle.is_empty()
        || replacement_key.contains(&needle)
        || name.to_lowercase().contains(&needle)
        || field_value.is_some_and(|value| name.to_lowercase().contains(&value));
    if matches && seen.insert(replacement_key) {
        found.push(Suggestion {
            query: format!("{query_prefix}{replacement}"),
            replacement: replacement.to_string(),
            kind,
        });
    }
}

fn sorted_unique(names: &[String]) -> Vec<&str> {
    let mut names: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSources {
        projects: Vec<String>,
        task_lists: Vec<String>,
        tags: Vec<String>,
    }

    impl TestSources {
        fn new() -> Self {
            Self {
                projects: vec!["Home".to_string(), "Work".to_string()],
                task_lists: vec!["Inbox".to_string(), "Later".to_string()],
                tags: vec!["shopping".to_string(), "work".to_string()],
            }
        }

        fn as_sources(&self) -> SuggestionSources<'_> {
            SuggestionSources {
                projects: &self.projects,
                task_lists: &self.task_lists,
                tags: &self.tags,
                account: Some("alice"),
            }
        }
    }

    #[test]
    fn ordinary_text_has_no_candidates() {
        let sources = TestSources::new();
        assert!(suggestions("milk", &sources.as_sources()).is_empty());
    }

    #[test]
    fn at_candidates_are_narrowed_by_partial_match() {
        let sources = TestSources::new();
        let found = suggestions("@ome", &sources.as_sources());
        assert!(found.iter().any(|item| item.replacement == "@project:Home"));
        assert!(!found.iter().any(|item| item.replacement == "@project:Work"));
    }

    #[test]
    fn bare_at_offers_every_at_candidate_kind() {
        let sources = TestSources::new();
        let kinds = suggestions("@", &sources.as_sources())
            .into_iter()
            .map(|item| item.kind)
            .collect::<HashSet<_>>();

        assert!(kinds.contains(&SuggestionKind::Due));
        assert!(kinds.contains(&SuggestionKind::Project));
        assert!(kinds.contains(&SuggestionKind::TaskList));
        assert!(kinds.contains(&SuggestionKind::Account));
    }

    #[test]
    fn field_aliases_match_their_values() {
        let sources = TestSources::new();
        let found = suggestions("@proj:ho", &sources.as_sources());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replacement, "@project:Home");
    }

    #[test]
    fn tags_use_prefix_matching() {
        let sources = TestSources::new();
        let found = suggestions("#wor", &sources.as_sources());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replacement, "#work");
        assert!(suggestions("#ork", &sources.as_sources()).is_empty());
    }

    #[test]
    fn selection_replaces_only_the_active_token() {
        let sources = TestSources::new();
        let found = suggestions("@today #sho", &sources.as_sources());
        assert_eq!(found[0].query, "@today #shopping");
    }
}
