//! Spellings of the search keywords in every supported language.
//!
//! Kept in Rust rather than in the `.po` catalogues: the parser has to accept
//! every language's spelling at once, whatever the UI language is, and a
//! catalogue only maps one key to the current language's text. The due-date
//! words live next to their matcher in [`super::due`].
//!
//! Operators (`AND`, `OR`, `NOT`, `|`, `-`) are deliberately not translated: a
//! translated operator would collide with free text ("または" in a task title).

use crate::viewmodels::settings::supported;

/// A language the search vocabulary has spellings for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Ja,
}

impl Lang {
    pub const ALL: [Self; 2] = [Self::En, Self::Ja];

    /// The vocabulary language for a UI locale tag, English when unknown.
    pub fn from_locale(locale: &str) -> Self {
        match supported(locale) {
            Some("ja") => Self::Ja,
            _ => Self::En,
        }
    }
}

/// A task state the search box can filter on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusKey {
    Completed,
    /// Anything still to be done: neither completed nor cancelled.
    Open,
    NotStarted,
    InProgress,
    Waiting,
    Cancelled,
}

/// The state of one task or subtask, as far as the search box cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitStatus {
    NotStarted,
    InProgress,
    Waiting,
    Completed,
    Cancelled,
}

const STATUSES: &[(StatusKey, &str, &[&str], &[&str])] = &[
    (
        StatusKey::Completed,
        "completed",
        &["done", "completed"],
        &["完了", "完了済み"],
    ),
    (StatusKey::Open, "open", &["open", "todo"], &["未完了"]),
    (
        StatusKey::NotStarted,
        "not-started",
        &["not-started"],
        &["未着手"],
    ),
    (
        StatusKey::InProgress,
        "in-progress",
        &["in-progress", "doing"],
        &["実行中", "進行中"],
    ),
    (StatusKey::Waiting, "waiting", &["waiting"], &["待機中"]),
    (
        StatusKey::Cancelled,
        "cancelled",
        &["cancelled", "canceled"],
        &["中止"],
    ),
];

impl StatusKey {
    pub const ALL: [Self; 6] = [
        Self::Open,
        Self::Completed,
        Self::NotStarted,
        Self::InProgress,
        Self::Waiting,
        Self::Cancelled,
    ];

    fn entry(
        self,
    ) -> &'static (
        StatusKey,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    ) {
        STATUSES
            .iter()
            .find(|(key, ..)| *key == self)
            .expect("every status has spellings")
    }

    /// Parses a folded word in any language, or the internal key.
    pub fn parse(word: &str) -> Option<Self> {
        STATUSES
            .iter()
            .find(|(_, key, en, ja)| *key == word || en.contains(&word) || ja.contains(&word))
            .map(|(status, ..)| *status)
    }

    /// The language-neutral spelling kept in the internal query.
    pub fn key(self) -> &'static str {
        self.entry().1
    }

    pub fn spell(self, lang: Lang) -> &'static str {
        let (_, _, en, ja) = self.entry();
        match lang {
            Lang::En => en[0],
            Lang::Ja => ja[0],
        }
    }

    pub fn spellings(self) -> impl Iterator<Item = &'static str> {
        let (_, _, en, ja) = self.entry();
        en.iter().chain(ja.iter()).copied()
    }

    pub fn matches(self, status: UnitStatus) -> bool {
        match self {
            Self::Completed => status == UnitStatus::Completed,
            Self::Open => !matches!(status, UnitStatus::Completed | UnitStatus::Cancelled),
            Self::NotStarted => status == UnitStatus::NotStarted,
            Self::InProgress => status == UnitStatus::InProgress,
            Self::Waiting => status == UnitStatus::Waiting,
            Self::Cancelled => status == UnitStatus::Cancelled,
        }
    }
}

/// The kind of thing an `@` token can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    Due,
    Status,
    Project,
    List,
    User,
}

const KINDS: &[(RefKind, &str, &[&str], &[&str])] = &[
    (RefKind::Due, "due", &["due"], &["期限"]),
    (RefKind::Status, "status", &["status"], &["状態"]),
    (
        RefKind::Project,
        "project",
        &["project", "proj"],
        &["プロジェクト"],
    ),
    (RefKind::List, "list", &["list"], &["リスト"]),
    (RefKind::User, "user", &["user"], &["ユーザー", "担当"]),
];

impl RefKind {
    /// The order candidates are offered in when a name fits several kinds.
    pub const ALL: [Self; 5] = [
        Self::Due,
        Self::Status,
        Self::Project,
        Self::List,
        Self::User,
    ];

    fn entry(
        self,
    ) -> &'static (
        RefKind,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    ) {
        KINDS
            .iter()
            .find(|(kind, ..)| *kind == self)
            .expect("every kind has spellings")
    }

    /// Parses the folded word before the `:` of `@<kind>:<name>`.
    pub fn parse(word: &str) -> Option<Self> {
        KINDS
            .iter()
            .find(|(_, _, en, ja)| en.contains(&word) || ja.contains(&word))
            .map(|(kind, ..)| *kind)
    }

    /// The language-neutral spelling kept in the internal query.
    pub fn key(self) -> &'static str {
        self.entry().1
    }

    pub fn spell(self, lang: Lang) -> &'static str {
        let (_, _, en, ja) = self.entry();
        match lang {
            Lang::En => en[0],
            Lang::Ja => ja[0],
        }
    }
}

/// A text field an `@<field>:<value>` token searches by partial match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextField {
    Task,
    Note,
    Subtask,
    SubtaskNote,
}

impl TextField {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "task" => Some(Self::Task),
            "note" => Some(Self::Note),
            "subtask" => Some(Self::Subtask),
            "subtasknote" => Some(Self::SubtaskNote),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_and_kind_has_a_spelling_in_every_language() {
        for lang in Lang::ALL {
            for status in StatusKey::ALL {
                let word = status.spell(lang);
                assert_eq!(StatusKey::parse(word), Some(status), "{lang:?} {status:?}");
                assert_eq!(StatusKey::parse(status.key()), Some(status));
            }
            for kind in RefKind::ALL {
                let word = kind.spell(lang);
                assert_eq!(RefKind::parse(word), Some(kind), "{lang:?} {kind:?}");
            }
        }
    }

    #[test]
    fn open_excludes_finished_and_cancelled_work() {
        assert!(StatusKey::Open.matches(UnitStatus::Waiting));
        assert!(!StatusKey::Open.matches(UnitStatus::Completed));
        assert!(!StatusKey::Open.matches(UnitStatus::Cancelled));
    }

    #[test]
    fn the_ui_locale_picks_the_vocabulary() {
        assert_eq!(Lang::from_locale("ja"), Lang::Ja);
        assert_eq!(Lang::from_locale("en"), Lang::En);
        assert_eq!(Lang::from_locale("de"), Lang::En);
    }
}
