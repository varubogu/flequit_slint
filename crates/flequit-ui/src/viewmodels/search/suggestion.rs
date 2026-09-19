//! Candidates offered for the `@` or `#` word being typed, and for an `@` word
//! that named several targets.

use std::ops::Range;

use super::document::{QueryDocument, Segment};
use super::due::{DueName, DueSpec};
use super::lexer::{TokenKind, tokenize};
use super::normalize::{fold, syntax_char};
use super::resolve::{NameIndex, Reference};
use super::vocabulary::{Lang, RefKind, StatusKey};

/// The most candidates shown at once; the list narrows as the user types.
const LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Due,
    Status,
    Project,
    TaskList,
    User,
    Tag,
}

/// What picking a suggestion puts into the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionTarget {
    Ref(Reference),
    Tag(String),
}

/// One candidate, ready to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The token as it would appear in the search box, e.g. `@仕事.今週やる`.
    pub label: String,
    pub kind: SuggestionKind,
    /// A due or state key, a list's project, or a user's handle; empty otherwise.
    pub detail: String,
    /// The project colour for projects and lists, empty when unset.
    pub color: String,
    /// A project's number of lists.
    pub count: usize,
    pub archived: bool,
    pub target: SuggestionTarget,
}

/// The `@` or `#` word the user is typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveToken {
    pub span: Range<usize>,
    pub text: String,
}

impl ActiveToken {
    /// Whether the word names its kind, e.g. `@user:yam`.
    pub fn kind(&self) -> Option<RefKind> {
        split_kind(self.text.get(1..).unwrap_or_default()).map(|(kind, _)| kind)
    }
}

/// Finds the `@` / `#` word in free text that the edit at `active` ended in.
pub fn active_token(document: &QueryDocument, active: usize) -> Option<ActiveToken> {
    document
        .segments()
        .iter()
        .zip(document.spans())
        .filter_map(|(segment, span)| match segment {
            Segment::Text(text) => Some((text, span)),
            _ => None,
        })
        .find_map(|(text, span)| {
            tokenize(text).into_iter().find_map(|token| {
                let global = span.start + token.span.start..span.start + token.span.end;
                let word = &text[token.span.clone()];
                let starts = word.chars().next().map(syntax_char);
                (token.kind == TokenKind::Word
                    && global.start < active
                    && active <= global.end
                    && matches!(starts, Some('@' | '#')))
                .then(|| ActiveToken {
                    span: global,
                    text: word.to_string(),
                })
            })
        })
}

/// Candidates for the word being typed.
pub fn suggestions(
    token: &ActiveToken,
    index: &NameIndex,
    tags: &[String],
    lang: Lang,
) -> Vec<Suggestion> {
    let mut chars = token.text.chars();
    match chars.next().map(syntax_char) {
        Some('#') => tag_suggestions(chars.as_str(), tags),
        Some('@') => at_suggestions(chars.as_str(), index, lang),
        _ => Vec::new(),
    }
}

/// Candidates for an ambiguous token, in the order they were resolved.
pub fn candidates(references: &[Reference], index: &NameIndex, lang: Lang) -> Vec<Suggestion> {
    references
        .iter()
        .filter_map(|reference| describe(reference, index, lang))
        .collect()
}

fn tag_suggestions(typed: &str, tags: &[String]) -> Vec<Suggestion> {
    let needle = fold(typed);
    let mut names: Vec<&String> = tags.iter().collect();
    names.sort_by_key(|name| fold(name));
    names.dedup_by_key(|name| fold(name));
    names
        .into_iter()
        .filter(|name| fold(name).starts_with(&needle))
        .take(LIMIT)
        .map(|name| Suggestion {
            label: format!("#{name}"),
            kind: SuggestionKind::Tag,
            detail: String::new(),
            color: String::new(),
            count: 0,
            archived: false,
            target: SuggestionTarget::Tag(name.clone()),
        })
        .collect()
}

fn at_suggestions(typed: &str, index: &NameIndex, lang: Lang) -> Vec<Suggestion> {
    let (kind, value) = match split_kind(typed) {
        Some((kind, value)) => (Some(kind), value),
        None => (None, typed),
    };
    let allows = |wanted: RefKind| kind.is_none_or(|kind| kind == wanted);
    let value: String = value.chars().filter(|ch| syntax_char(*ch) != '"').collect();
    let parts: Vec<String> = value.split(|ch| syntax_char(ch) == '.').map(fold).collect();

    let mut found = Vec::new();
    if let [project, list] = parts.as_slice() {
        if allows(RefKind::List) {
            found.extend(
                index
                    .lists()
                    .filter(|entry| {
                        fold(&entry.name).contains(list.as_str())
                            && index
                                .project(&entry.project_id)
                                .is_some_and(|parent| fold(&parent.name) == *project)
                    })
                    .map(|entry| Reference::List(entry.id.clone())),
            );
        }
    } else {
        let needle = parts.concat();
        if allows(RefKind::Due) {
            let mut specs: Vec<DueSpec> = DueName::ALL
                .iter()
                .map(|name| DueSpec::Named(*name))
                .collect();
            specs.insert(3, DueSpec::Days(3));
            if let Some(spec) = DueSpec::parse(&needle)
                && !specs.contains(&spec)
            {
                specs.insert(0, spec);
            }
            found.extend(
                specs
                    .into_iter()
                    .filter(|spec| {
                        spec.spellings()
                            .iter()
                            .any(|word| fold(word).contains(&needle))
                    })
                    .map(Reference::Due),
            );
        }
        if allows(RefKind::Status) {
            found.extend(
                StatusKey::ALL
                    .into_iter()
                    .filter(|status| status.spellings().any(|word| fold(word).contains(&needle)))
                    .map(Reference::Status),
            );
        }
        if allows(RefKind::Project) {
            found.extend(
                index
                    .projects()
                    .filter(|entry| fold(&entry.name).contains(&needle))
                    .map(|entry| Reference::Project(entry.id.clone())),
            );
        }
        if allows(RefKind::List) {
            found.extend(
                index
                    .lists()
                    .filter(|entry| fold(&entry.name).contains(&needle))
                    .map(|entry| Reference::List(entry.id.clone())),
            );
        }
        if allows(RefKind::User) {
            found.extend(
                index
                    .users()
                    .filter(|entry| {
                        fold(&entry.display_name).contains(&needle)
                            || fold(&entry.handle).contains(&needle)
                    })
                    .map(|entry| Reference::User(entry.id.clone())),
            );
        }
    }

    found
        .iter()
        .take(LIMIT)
        .filter_map(|reference| describe(reference, index, lang))
        .collect()
}

fn describe(reference: &Reference, index: &NameIndex, lang: Lang) -> Option<Suggestion> {
    let label = index.render(reference, lang, false)?;
    let mut suggestion = Suggestion {
        label,
        kind: SuggestionKind::Due,
        detail: String::new(),
        color: String::new(),
        count: 0,
        archived: false,
        target: SuggestionTarget::Ref(reference.clone()),
    };
    match reference {
        Reference::Due(spec) => suggestion.detail = spec.key(),
        Reference::Status(status) => {
            suggestion.kind = SuggestionKind::Status;
            suggestion.detail = status.key().to_string();
        }
        Reference::Project(id) => {
            let project = index.project(id)?;
            suggestion.kind = SuggestionKind::Project;
            suggestion.color = project.color.clone();
            suggestion.count = project.list_count;
            suggestion.archived = project.archived;
        }
        Reference::List(id) => {
            let list = index.list(id)?;
            let project = index.project(&list.project_id)?;
            suggestion.kind = SuggestionKind::TaskList;
            suggestion.detail = project.name.clone();
            suggestion.color = project.color.clone();
            suggestion.archived = project.archived;
        }
        Reference::User(id) => {
            suggestion.kind = SuggestionKind::User;
            suggestion.detail = format!("@{}", index.user(id)?.handle);
        }
    }
    Some(suggestion)
}

/// Splits `<kind>:<value>` when the prefix names a kind.
fn split_kind(typed: &str) -> Option<(RefKind, &str)> {
    let (index, ch) = typed
        .char_indices()
        .find(|(_, ch)| syntax_char(*ch) == ':')?;
    RefKind::parse(&fold(&typed[..index])).map(|kind| (kind, &typed[index + ch.len_utf8()..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodels::search::resolve::tests::index;

    fn labels(typed: &str) -> Vec<String> {
        let token = ActiveToken {
            span: 0..typed.len(),
            text: typed.into(),
        };
        suggestions(
            &token,
            &index(),
            &["Shopping".into(), "shop".into(), "Work".into()],
            Lang::Ja,
        )
        .into_iter()
        .map(|suggestion| suggestion.label)
        .collect()
    }

    #[test]
    fn at_offers_every_kind_narrowed_by_partial_match() {
        // Due keywords first, then the project called 今日, then the list.
        assert_eq!(
            labels("@今"),
            [
                "@今日",
                "@今週",
                "@今月",
                "@今期",
                "@今年",
                "@今年度",
                "@今日",
                "@仕事.今週やる"
            ]
        );
        assert_eq!(labels("@yama"), vec!["@山田".to_string()]);
        assert_eq!(labels("@未完"), vec!["@未完了".to_string()]);
    }

    #[test]
    fn a_kind_prefix_or_a_dot_narrows_what_is_offered() {
        assert_eq!(labels("@user:"), vec!["@山田".to_string()]);
        assert_eq!(labels("@Home."), vec!["@Home.Inbox".to_string()]);
        assert_eq!(labels("@仕事.in"), vec!["@仕事.Inbox".to_string()]);
    }

    #[test]
    fn a_count_being_typed_is_offered_as_a_due_keyword() {
        assert_eq!(labels("@10分").first().map(String::as_str), Some("@10分"));
    }

    #[test]
    fn tags_are_matched_by_prefix_and_listed_once() {
        assert_eq!(
            labels("#sh"),
            vec!["#shop".to_string(), "#Shopping".to_string()]
        );
    }

    #[test]
    fn the_word_ending_at_the_edit_is_the_active_one() {
        let document = QueryDocument::from_text("milk @to #x");
        assert_eq!(
            active_token(&document, 8).map(|token| token.text),
            Some("@to".to_string())
        );
        assert_eq!(active_token(&document, 4), None);
        assert_eq!(
            active_token(&document, 11).map(|token| token.text),
            Some("#x".into())
        );
    }
}
