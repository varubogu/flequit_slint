//! Evaluates a parsed query against tasks and subtasks.
//!
//! Tasks and subtasks are evaluated as separate units against the whole query.
//! A subtask inherits where its parent lives, its parent's tags and its
//! parent's text, but not its due date, state or assignees: "a subtask due
//! today" has to find the subtask, not every subtask of a task due today.

use std::cell::OnceCell;
use std::ops::Range;

use chrono::{DateTime, Utc};

use super::normalize::fold;
use super::parser::{Expr, Parsed, Term};
use super::resolve::{NameIndex, Reference};
use super::vocabulary::{TextField, UnitStatus};
use crate::adapters::datetime::DisplayTimezone;

/// A task or a subtask, borrowed from the loaded tree.
#[derive(Debug, Clone, Copy)]
pub struct SearchUnit<'a> {
    pub project_id: &'a str,
    pub list_id: &'a str,
    pub title: &'a str,
    pub notes: Option<&'a str>,
    /// For a subtask: its parent task's title and notes.
    pub parent: Option<UnitText<'a>>,
    /// For a task: its subtasks' titles and notes, searched by `@subtask:`.
    pub children: &'a [UnitText<'a>],
    pub due: Option<&'a DateTime<Utc>>,
    pub status: UnitStatus,
    /// `UserId`s of the assignees.
    pub assignees: &'a [String],
    /// Tag names. A subtask's include its parent's.
    pub tags: &'a [&'a str],
}

#[derive(Debug, Clone, Copy)]
pub struct UnitText<'a> {
    pub title: &'a str,
    pub notes: Option<&'a str>,
}

/// When the query is evaluated.
#[derive(Debug, Clone, Copy)]
pub struct EvalContext {
    pub now: DateTime<Utc>,
    pub timezone: DisplayTimezone,
}

/// A query with every name looked up, ready to test units against.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    expr: Option<Node>,
    /// `@` words that name nothing, as typed.
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone)]
enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Leaf(Leaf),
}

#[derive(Debug, Clone)]
enum Leaf {
    Text(String),
    Tag(String),
    Field(TextField, String),
    AnyOf(Vec<Reference>),
    /// Contributes nothing: neither narrows nor widens the result.
    Ignore,
}

impl Prepared {
    /// Looks up the names left in `parsed`.
    ///
    /// A word being typed at `active` narrows by whatever it already names
    /// exactly and is otherwise ignored, so the list does not empty mid-word.
    pub fn new(parsed: &Parsed, index: &NameIndex, active: Option<usize>) -> Self {
        let mut unresolved = Vec::new();
        let expr = parsed
            .expr
            .as_ref()
            .map(|expr| prepare(expr, index, active, &mut unresolved));
        Self { expr, unresolved }
    }

    /// Whether the query has no condition at all.
    pub fn is_empty(&self) -> bool {
        self.expr.as_ref().is_none_or(Node::is_neutral)
    }

    /// Whether the unit satisfies the query. An empty query matches everything.
    pub fn matches(&self, unit: &SearchUnit<'_>, context: &EvalContext) -> bool {
        let folded = Folded::new(unit);
        self.expr
            .as_ref()
            .and_then(|expr| expr.evaluate(unit, &folded, context))
            .unwrap_or(true)
    }

    /// A query consisting of one reference, as a sidebar button would write it.
    pub fn single(reference: Reference) -> Self {
        Self {
            expr: Some(Node::Leaf(Leaf::AnyOf(vec![reference]))),
            unresolved: Vec::new(),
        }
    }
}

fn prepare(
    expr: &Expr,
    index: &NameIndex,
    active: Option<usize>,
    unresolved: &mut Vec<String>,
) -> Node {
    match expr {
        Expr::And(items) => Node::And(
            items
                .iter()
                .map(|item| prepare(item, index, active, unresolved))
                .collect(),
        ),
        Expr::Or(items) => Node::Or(
            items
                .iter()
                .map(|item| prepare(item, index, active, unresolved))
                .collect(),
        ),
        Expr::Not(inner) => Node::Not(Box::new(prepare(inner, index, active, unresolved))),
        Expr::Term(node) => Node::Leaf(match &node.term {
            Term::Text(value) => Leaf::Text(value.clone()),
            Term::Tag(value) => Leaf::Tag(value.clone()),
            Term::Field(field, value) => Leaf::Field(*field, value.clone()),
            Term::Ref(reference) => Leaf::AnyOf(vec![reference.clone()]),
            Term::AnyOf(candidates) => Leaf::AnyOf(candidates.clone()),
            Term::Ignore => Leaf::Ignore,
            Term::Name(word) => {
                let candidates = index.resolve(word);
                if candidates.is_empty() {
                    if !is_active(&node.span, active) {
                        unresolved.push(node.source.text().to_string());
                    }
                    Leaf::Ignore
                } else {
                    Leaf::AnyOf(candidates)
                }
            }
        }),
    }
}

fn is_active(span: &Range<usize>, active: Option<usize>) -> bool {
    active.is_some_and(|at| span.start <= at && at <= span.end)
}

/// The unit's text fields, folded on first use.
struct Folded<'a> {
    unit: &'a SearchUnit<'a>,
    own: OnceCell<Vec<String>>,
    parent: OnceCell<Vec<String>>,
    children_titles: OnceCell<Vec<String>>,
    children_notes: OnceCell<Vec<String>>,
    tags: OnceCell<Vec<String>>,
}

impl<'a> Folded<'a> {
    fn new(unit: &'a SearchUnit<'a>) -> Self {
        Self {
            unit,
            own: OnceCell::new(),
            parent: OnceCell::new(),
            children_titles: OnceCell::new(),
            children_notes: OnceCell::new(),
            tags: OnceCell::new(),
        }
    }

    /// Title then notes of the unit itself.
    fn own(&self) -> &[String] {
        self.own
            .get_or_init(|| text_of(self.unit.title, self.unit.notes))
    }

    /// Title then notes of the parent task, empty for a task.
    fn parent(&self) -> &[String] {
        self.parent.get_or_init(|| {
            self.unit
                .parent
                .map(|parent| text_of(parent.title, parent.notes))
                .unwrap_or_default()
        })
    }

    fn children_titles(&self) -> &[String] {
        self.children_titles.get_or_init(|| {
            self.unit
                .children
                .iter()
                .map(|child| fold(child.title))
                .collect()
        })
    }

    fn children_notes(&self) -> &[String] {
        self.children_notes.get_or_init(|| {
            self.unit
                .children
                .iter()
                .filter_map(|child| child.notes.map(fold))
                .collect()
        })
    }

    fn tags(&self) -> &[String] {
        self.tags
            .get_or_init(|| self.unit.tags.iter().map(|tag| fold(tag)).collect())
    }
}

fn text_of(title: &str, notes: Option<&str>) -> Vec<String> {
    let mut values = vec![fold(title)];
    values.extend(notes.map(fold));
    values
}

fn contains(haystacks: &[String], needle: &str) -> bool {
    haystacks.iter().any(|value| value.contains(needle))
}

impl Node {
    /// Whether this node can never narrow anything.
    fn is_neutral(&self) -> bool {
        match self {
            Self::And(items) | Self::Or(items) => items.iter().all(Self::is_neutral),
            Self::Not(inner) => inner.is_neutral(),
            Self::Leaf(leaf) => matches!(leaf, Leaf::Ignore),
        }
    }

    /// `None` when the node holds no condition, so it neither narrows an AND
    /// nor satisfies an OR on its own.
    fn evaluate(
        &self,
        unit: &SearchUnit<'_>,
        folded: &Folded<'_>,
        context: &EvalContext,
    ) -> Option<bool> {
        match self {
            Self::And(items) => {
                let mut result = None;
                for item in items {
                    match item.evaluate(unit, folded, context) {
                        Some(false) => return Some(false),
                        Some(true) => result = Some(true),
                        None => {}
                    }
                }
                result
            }
            Self::Or(items) => {
                let mut result = None;
                for item in items {
                    match item.evaluate(unit, folded, context) {
                        Some(true) => return Some(true),
                        Some(false) => result = Some(false),
                        None => {}
                    }
                }
                result
            }
            Self::Not(inner) => inner.evaluate(unit, folded, context).map(|value| !value),
            Self::Leaf(leaf) => leaf.evaluate(unit, folded, context),
        }
    }
}

impl Leaf {
    fn evaluate(
        &self,
        unit: &SearchUnit<'_>,
        folded: &Folded<'_>,
        context: &EvalContext,
    ) -> Option<bool> {
        Some(match self {
            Self::Ignore => return None,
            Self::Text(needle) => {
                contains(folded.own(), needle) || contains(folded.parent(), needle)
            }
            Self::Tag(prefix) => folded
                .tags()
                .iter()
                .any(|tag| tag.starts_with(prefix.as_str())),
            Self::Field(field, needle) => {
                let is_subtask = unit.parent.is_some();
                let values: &[String] = match (field, is_subtask) {
                    (TextField::Task, false) => &folded.own()[..1],
                    (TextField::Note, false) => &folded.own()[1..],
                    (TextField::Task, true) => folded.parent().get(..1).unwrap_or_default(),
                    (TextField::Note, true) => folded.parent().get(1..).unwrap_or_default(),
                    (TextField::Subtask, false) => folded.children_titles(),
                    (TextField::SubtaskNote, false) => folded.children_notes(),
                    (TextField::Subtask, true) => &folded.own()[..1],
                    (TextField::SubtaskNote, true) => &folded.own()[1..],
                };
                contains(values, needle)
            }
            Self::AnyOf(references) => references
                .iter()
                .any(|reference| reference_matches(reference, unit, context)),
        })
    }
}

fn reference_matches(reference: &Reference, unit: &SearchUnit<'_>, context: &EvalContext) -> bool {
    match reference {
        Reference::Due(spec) => spec.keyword().matches(
            unit.due,
            unit.status == UnitStatus::Completed,
            &context.now,
            context.timezone,
        ),
        Reference::Status(status) => status.matches(unit.status),
        Reference::Project(id) => unit.project_id == id,
        Reference::List(id) => unit.list_id == id,
        Reference::User(id) => unit.assignees.iter().any(|assignee| assignee == id),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::viewmodels::search::document::QueryDocument;
    use crate::viewmodels::search::parser::parse;
    use crate::viewmodels::search::resolve::tests::index;

    fn context() -> EvalContext {
        EvalContext {
            now: Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap(),
            timezone: DisplayTimezone::Utc,
        }
    }

    fn due_tomorrow() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, 15, 0, 0).unwrap()
    }

    const CHILDREN: &[UnitText<'static>] = &[UnitText {
        title: "Check the fridge",
        notes: Some("bottom shelf"),
    }];

    fn task<'a>(due: Option<&'a DateTime<Utc>>, assignees: &'a [String]) -> SearchUnit<'a> {
        SearchUnit {
            project_id: "p-home",
            list_id: "l-inbox",
            title: "Buy milk",
            notes: Some("Semi-skimmed"),
            parent: None,
            children: CHILDREN,
            due,
            status: UnitStatus::NotStarted,
            assignees,
            tags: &["Shopping"],
        }
    }

    fn subtask<'a>(due: Option<&'a DateTime<Utc>>, assignees: &'a [String]) -> SearchUnit<'a> {
        SearchUnit {
            project_id: "p-home",
            list_id: "l-inbox",
            title: "Check the fridge",
            notes: Some("bottom shelf"),
            parent: Some(UnitText {
                title: "Buy milk",
                notes: Some("Semi-skimmed"),
            }),
            children: &[],
            due,
            status: UnitStatus::Completed,
            assignees,
            tags: &["Shopping"],
        }
    }

    fn matches(query: &str, unit: &SearchUnit<'_>) -> bool {
        let document = QueryDocument::from_text(query);
        Prepared::new(&parse(&document), &index(), None).matches(unit, &context())
    }

    #[test]
    fn an_empty_or_half_typed_query_matches_everything() {
        let unit = task(None, &[]);
        for query in ["", "  ", "@", "#", "-", "@unknown", "(", "@to"] {
            assert!(matches(query, &unit), "{query:?}");
        }
    }

    #[test]
    fn free_text_is_folded_and_reaches_the_notes() {
        let unit = task(None, &[]);
        assert!(matches("MILK", &unit));
        assert!(matches("ＳＫＩＭ", &unit));
        assert!(!matches("bread", &unit));
    }

    #[test]
    fn and_or_not_and_groups_combine() {
        let due = due_tomorrow();
        let unit = task(Some(&due), &[]);
        assert!(matches("@tomorrow milk", &unit));
        assert!(!matches("@today milk", &unit));
        assert!(matches("@today | milk", &unit));
        assert!(!matches("-milk", &unit));
        assert!(matches("(@today | @明日) -#work", &unit));
        assert!(!matches("(@today | bread) milk", &unit));
    }

    #[test]
    fn an_unresolved_name_does_not_widen_an_or() {
        let unit = task(None, &[]);
        assert!(!matches("@nobody | bread", &unit));
    }

    #[test]
    fn places_states_and_assignees_narrow_by_reference() {
        let assignees = ["u-yamada".to_string()];
        let unit = task(None, &assignees);
        assert!(matches("@Home", &unit));
        assert!(matches("@Home.Inbox", &unit));
        assert!(matches("@user:yamada", &unit));
        assert!(matches("@未完了", &unit));
        assert!(!matches("@完了", &unit));
        assert!(!matches("@仕事", &unit));
    }

    #[test]
    fn fields_search_their_own_text() {
        let unit = task(None, &[]);
        assert!(matches("@task:milk", &unit));
        assert!(!matches("@note:milk", &unit));
        assert!(matches("@subtask:fridge", &unit));
        assert!(matches("@subtasknote:shelf", &unit));
        assert!(
            !matches("fridge", &unit),
            "a task no longer reaches into its subtasks"
        );
    }

    #[test]
    fn a_subtask_inherits_place_tags_and_text_but_not_due_state_or_assignees() {
        let parent_due = due_tomorrow();
        let parent = task(Some(&parent_due), &[]);
        let child = subtask(None, &[]);

        assert!(matches("@Home #shop milk fridge", &child));
        assert!(matches("@task:milk @subtask:fridge", &child));
        assert!(matches("@tomorrow", &parent));
        assert!(!matches("@tomorrow", &child));
        assert!(matches("@完了", &child));
        assert!(!matches("@完了", &parent));
    }

    #[test]
    fn all_conditions_must_hold_within_one_unit() {
        let due = due_tomorrow();
        let assignees = ["u-yamada".to_string()];
        let parent = task(Some(&due), &[]);
        let child = subtask(None, &assignees);

        assert!(!matches("@user:yamada @tomorrow", &parent));
        assert!(!matches("@user:yamada @tomorrow", &child));
    }

    #[test]
    fn a_wildcard_matches_every_target_it_names() {
        let unit = task(None, &[]);
        assert!(matches("@project:h*", &unit));
        assert!(
            !matches("@project:仕*", &unit),
            "names only the 仕事 projects"
        );

        // A pattern naming nothing is ignored and reported, like any `@` word
        // naming nothing.
        let document = QueryDocument::from_text("@project:x* milk");
        let prepared = Prepared::new(&parse(&document), &index(), None);
        assert!(prepared.matches(&unit, &context()));
        assert_eq!(prepared.unresolved, vec!["@project:x*".to_string()]);
    }
}
