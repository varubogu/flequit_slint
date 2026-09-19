//! Looks names up and writes references back as names.
//!
//! A [`Reference`] is what a confirmed `@` token points at. The search box
//! shows it as a short name (`@仕事`) while the internal query keeps the
//! reference itself (`@project:{id}`), so a rename or a language switch only
//! changes how it is shown.

use super::due::DueSpec;
use super::lexer::{AtWord, RawTerm, read_term};
use super::normalize::{fold, quote_if_needed};
use super::vocabulary::{Lang, RefKind, StatusKey};

/// The target of a confirmed `@` token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reference {
    Due(DueSpec),
    Status(StatusKey),
    Project(String),
    List(String),
    /// A user, by `UserId`: the handle may become editable.
    User(String),
}

impl Reference {
    pub fn kind(&self) -> RefKind {
        match self {
            Self::Due(_) => RefKind::Due,
            Self::Status(_) => RefKind::Status,
            Self::Project(_) => RefKind::Project,
            Self::List(_) => RefKind::List,
            Self::User(_) => RefKind::User,
        }
    }

    /// The token kept in the internal query, e.g. `@due:today`.
    pub fn canonical(&self) -> String {
        let kind = self.kind().key();
        match self {
            Self::Due(spec) => format!("@{kind}:{}", spec.key()),
            Self::Status(status) => format!("@{kind}:{}", status.key()),
            Self::Project(id) | Self::List(id) | Self::User(id) => format!("@{kind}:{{{id}}}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    /// Stored hex colour, empty when unset.
    pub color: String,
    pub list_count: usize,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub id: String,
    pub project_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEntry {
    pub id: String,
    pub display_name: String,
    pub handle: String,
}

/// Every name the search box can resolve, with folded copies for matching.
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    projects: Vec<(ProjectEntry, String)>,
    lists: Vec<(ListEntry, String)>,
    users: Vec<(UserEntry, String, String)>,
}

impl NameIndex {
    pub fn new(projects: Vec<ProjectEntry>, lists: Vec<ListEntry>, users: Vec<UserEntry>) -> Self {
        Self {
            projects: projects
                .into_iter()
                .map(|entry| {
                    let folded = fold(&entry.name);
                    (entry, folded)
                })
                .collect(),
            lists: lists
                .into_iter()
                .map(|entry| {
                    let folded = fold(&entry.name);
                    (entry, folded)
                })
                .collect(),
            users: users
                .into_iter()
                .map(|entry| {
                    let name = fold(&entry.display_name);
                    let handle = fold(&entry.handle);
                    (entry, name, handle)
                })
                .collect(),
        }
    }

    pub fn projects(&self) -> impl Iterator<Item = &ProjectEntry> {
        self.projects.iter().map(|(entry, _)| entry)
    }

    pub fn lists(&self) -> impl Iterator<Item = &ListEntry> {
        self.lists.iter().map(|(entry, _)| entry)
    }

    pub fn users(&self) -> impl Iterator<Item = &UserEntry> {
        self.users.iter().map(|(entry, ..)| entry)
    }

    pub fn project(&self, id: &str) -> Option<&ProjectEntry> {
        self.projects().find(|entry| entry.id == id)
    }

    pub fn list(&self, id: &str) -> Option<&ListEntry> {
        self.lists().find(|entry| entry.id == id)
    }

    pub fn user(&self, id: &str) -> Option<&UserEntry> {
        self.users().find(|entry| entry.id == id)
    }

    /// Whether the reference still points at something that exists.
    pub fn exists(&self, reference: &Reference) -> bool {
        match reference {
            Reference::Due(_) | Reference::Status(_) => true,
            Reference::Project(id) => self.project(id).is_some(),
            Reference::List(id) => self.list(id).is_some(),
            Reference::User(id) => self.user(id).is_some(),
        }
    }

    /// Everything an `@` word names, in the order candidates are offered.
    pub fn resolve(&self, word: &AtWord) -> Vec<Reference> {
        let mut found = Vec::new();
        if let Some(id) = &word.id {
            let reference = match word.kind {
                Some(RefKind::Project) => Reference::Project(id.clone()),
                Some(RefKind::List) => Reference::List(id.clone()),
                Some(RefKind::User) => Reference::User(id.clone()),
                _ => return found,
            };
            if self.exists(&reference) {
                found.push(reference);
            }
            return found;
        }

        let allows = |kind: RefKind| word.kind.is_none_or(|wanted| wanted == kind);
        if word.wildcard {
            self.resolve_pattern(word, &allows, &mut found);
            return found;
        }

        let whole = fold(&word.whole_name());
        if !word.quoted && word.parts.len() == 1 {
            if allows(RefKind::Due)
                && let Some(spec) = DueSpec::parse(&whole)
            {
                found.push(Reference::Due(spec));
            }
            if allows(RefKind::Status)
                && let Some(status) = StatusKey::parse(&whole)
            {
                found.push(Reference::Status(status));
            }
        }
        if allows(RefKind::Project) {
            found.extend(
                self.projects
                    .iter()
                    .filter(|(_, name)| *name == whole)
                    .map(|(entry, _)| Reference::Project(entry.id.clone())),
            );
        }
        if allows(RefKind::List) {
            found.extend(
                self.lists
                    .iter()
                    .filter(|(_, name)| *name == whole)
                    .map(|(entry, _)| Reference::List(entry.id.clone())),
            );
            if let [project, list @ ..] = word.parts.as_slice()
                && !list.is_empty()
            {
                let project = fold(project);
                let list = fold(&list.join("."));
                found.extend(self.qualified_lists(|name| *name == project, |name| *name == list));
            }
        }
        if allows(RefKind::User) {
            found.extend(
                self.users
                    .iter()
                    .filter(|(_, name, handle)| *name == whole || *handle == whole)
                    .map(|(entry, ..)| Reference::User(entry.id.clone())),
            );
        }

        dedup(found)
    }

    fn resolve_pattern(
        &self,
        word: &AtWord,
        allows: &dyn Fn(RefKind) -> bool,
        found: &mut Vec<Reference>,
    ) {
        let parts: Vec<String> = word.parts.iter().map(|part| fold(part)).collect();
        match parts.as_slice() {
            [pattern] => {
                if allows(RefKind::Project) {
                    found.extend(
                        self.projects
                            .iter()
                            .filter(|(_, name)| glob(pattern, name))
                            .map(|(entry, _)| Reference::Project(entry.id.clone())),
                    );
                }
                if allows(RefKind::List) {
                    found.extend(
                        self.lists
                            .iter()
                            .filter(|(_, name)| glob(pattern, name))
                            .map(|(entry, _)| Reference::List(entry.id.clone())),
                    );
                }
                if allows(RefKind::User) {
                    found.extend(
                        self.users
                            .iter()
                            .filter(|(_, name, handle)| {
                                glob(pattern, name) || glob(pattern, handle)
                            })
                            .map(|(entry, ..)| Reference::User(entry.id.clone())),
                    );
                }
            }
            [project, list @ ..] if allows(RefKind::List) => {
                let list = list.join(".");
                found.extend(
                    self.qualified_lists(|name| glob(project, name), |name| glob(&list, name)),
                );
            }
            _ => {}
        }
    }

    fn qualified_lists(
        &self,
        project_matches: impl Fn(&String) -> bool,
        list_matches: impl Fn(&String) -> bool,
    ) -> Vec<Reference> {
        self.lists
            .iter()
            .filter(|(entry, name)| {
                list_matches(name)
                    && self.projects.iter().any(|(project, folded)| {
                        project.id == entry.project_id && project_matches(folded)
                    })
            })
            .map(|(entry, _)| Reference::List(entry.id.clone()))
            .collect()
    }

    /// The token shown in the search box for a reference, or `None` when the
    /// target no longer exists.
    ///
    /// `explicit` keeps the `@<kind>:` prefix, for tokens the user typed that way.
    pub fn render(&self, reference: &Reference, lang: Lang, explicit: bool) -> Option<String> {
        let body = match reference {
            Reference::Due(spec) => spec.spell(lang),
            Reference::Status(status) => status.spell(lang).to_string(),
            Reference::Project(id) => quote_if_needed(&self.project(id)?.name),
            Reference::List(id) => {
                let list = self.list(id)?;
                let project = self.project(&list.project_id)?;
                format!(
                    "{}.{}",
                    quote_if_needed(&project.name),
                    quote_if_needed(&list.name)
                )
            }
            Reference::User(id) => {
                let user = self.user(id)?;
                let name = if user.display_name.is_empty() {
                    &user.handle
                } else {
                    &user.display_name
                };
                quote_if_needed(name)
            }
        };
        Some(if explicit {
            format!("@{}:{body}", reference.kind().spell(lang))
        } else {
            format!("@{body}")
        })
    }

    /// Whether `text` still names `reference`, i.e. the token needs no rewrite.
    pub fn describes(&self, text: &str, reference: &Reference) -> bool {
        match read_term(text) {
            RawTerm::At(word) => self.resolve(&word).contains(reference),
            _ => false,
        }
    }
}

/// Whether the `@` token was written with an explicit `@<kind>:` prefix.
pub fn is_explicit(text: &str) -> bool {
    matches!(read_term(text), RawTerm::At(AtWord { kind: Some(_), .. }))
}

fn dedup(references: Vec<Reference>) -> Vec<Reference> {
    let mut unique: Vec<Reference> = Vec::with_capacity(references.len());
    for reference in references {
        if !unique.contains(&reference) {
            unique.push(reference);
        }
    }
    unique
}

/// Matches a folded pattern where `*` stands for any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else {
        // No `*` at all: an exact match.
        return rest.is_empty();
    };
    for piece in middle {
        match rest.find(piece) {
            Some(index) => rest = &rest[index + piece.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::viewmodels::search::due::DueName;

    pub fn index() -> NameIndex {
        NameIndex::new(
            vec![
                project("p-work", "仕事"),
                project("p-work2", "仕事"),
                project("p-home", "Home"),
                project("p-today", "今日"),
                project("p-yamada", "山田"),
                project("p-ver", "v1.2"),
            ],
            vec![
                list("l-inbox", "p-home", "Inbox"),
                list("l-week", "p-work", "今週やる"),
                list("l-inbox2", "p-work", "Inbox"),
                list("l-bug", "p-ver", "バグ"),
            ],
            vec![UserEntry {
                id: "u-yamada".into(),
                display_name: "山田".into(),
                handle: "yamada".into(),
            }],
        )
    }

    fn project(id: &str, name: &str) -> ProjectEntry {
        ProjectEntry {
            id: id.into(),
            name: name.into(),
            color: String::new(),
            list_count: 0,
            archived: false,
        }
    }

    fn list(id: &str, project_id: &str, name: &str) -> ListEntry {
        ListEntry {
            id: id.into(),
            project_id: project_id.into(),
            name: name.into(),
        }
    }

    fn resolve(text: &str) -> Vec<Reference> {
        match read_term(text) {
            RawTerm::At(word) => index().resolve(&word),
            other => panic!("not an @ word: {other:?}"),
        }
    }

    #[test]
    fn a_name_can_fit_several_kinds_and_is_offered_in_order() {
        assert_eq!(
            resolve("@山田"),
            vec![
                Reference::Project("p-yamada".into()),
                Reference::User("u-yamada".into())
            ]
        );
        assert_eq!(
            resolve("@今日"),
            vec![
                Reference::Due(DueSpec::Named(DueName::Today)),
                Reference::Project("p-today".into())
            ]
        );
    }

    #[test]
    fn a_kind_prefix_narrows_the_lookup() {
        assert_eq!(
            resolve("@user:yamada"),
            vec![Reference::User("u-yamada".into())]
        );
        assert_eq!(
            resolve("@プロジェクト:今日"),
            vec![Reference::Project("p-today".into())]
        );
        assert_eq!(
            resolve(r#"@"今日""#),
            vec![Reference::Project("p-today".into())]
        );
    }

    #[test]
    fn same_named_projects_are_all_candidates() {
        assert_eq!(
            resolve("@仕事"),
            vec![
                Reference::Project("p-work".into()),
                Reference::Project("p-work2".into())
            ]
        );
    }

    #[test]
    fn a_dotted_name_is_tried_whole_before_it_is_split() {
        assert_eq!(resolve("@v1.2"), vec![Reference::Project("p-ver".into())]);
        assert_eq!(
            resolve("@仕事.inbox"),
            vec![Reference::List("l-inbox2".into())]
        );
        assert_eq!(
            resolve(r#"@"v1.2"."バグ""#),
            vec![Reference::List("l-bug".into())]
        );
    }

    #[test]
    fn names_are_matched_exactly_unless_a_wildcard_is_used() {
        assert!(resolve("@仕").is_empty());
        assert_eq!(
            resolve("@project:仕*"),
            vec![
                Reference::Project("p-work".into()),
                Reference::Project("p-work2".into())
            ]
        );
        assert_eq!(
            resolve("@list:*box"),
            vec![
                Reference::List("l-inbox".into()),
                Reference::List("l-inbox2".into())
            ]
        );
    }

    #[test]
    fn a_reference_renders_to_a_name_that_resolves_back_to_it() {
        let index = index();
        for reference in [
            Reference::Due(DueSpec::Named(DueName::Today)),
            Reference::Status(StatusKey::Open),
            Reference::Project("p-home".into()),
            Reference::List("l-week".into()),
            Reference::List("l-bug".into()),
            Reference::User("u-yamada".into()),
        ] {
            for lang in Lang::ALL {
                for explicit in [false, true] {
                    let text = index.render(&reference, lang, explicit).expect("rendered");
                    assert!(
                        index.describes(&text, &reference),
                        "{text} -> {reference:?}"
                    );
                }
            }
        }
        assert_eq!(
            index
                .render(&Reference::List("l-bug".into()), Lang::Ja, false)
                .as_deref(),
            Some(r#"@"v1.2".バグ"#)
        );
    }

    #[test]
    fn an_id_token_resolves_only_while_its_target_exists() {
        assert_eq!(
            resolve("@project:{p-home}"),
            vec![Reference::Project("p-home".into())]
        );
        assert!(resolve("@project:{gone}").is_empty());
    }

    #[test]
    fn a_glob_matches_runs_of_characters() {
        assert!(glob("a*c", "abc"));
        assert!(glob("*", ""));
        assert!(glob("a*", "a"));
        assert!(!glob("a*c", "abd"));
        assert!(glob("*b*", "abc"));
        assert!(!glob("abc", "abcd"));
    }
}
