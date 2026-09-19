//! What the sidebar does to the query, and what the query lights up in it.
//!
//! Sidebar items (due and state buttons, projects, lists, pinned tags) rewrite
//! the query instead of holding a selection of their own. Edits go through the
//! condition tree and write it back, so combining with an existing `a | b`
//! adds the parentheses precedence requires.

use super::document::{QueryDocument, Segment};
use super::normalize::fold;
use super::parser::{Expr, Parsed, Term, TermNode, parse};
use super::resolve::Reference;

/// A sidebar item, as far as the query is concerned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKey {
    Ref(Reference),
    /// A pinned tag, by folded name.
    Tag(String),
}

/// How a sidebar item changes the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryEdit {
    /// Plain click: show only this item, or clear when it is already all there is.
    Click,
    /// Primary modifier + click: AND it in, or take it out of the AND terms.
    ToggleAnd,
    /// Secondary modifier + click: OR it in, or take it out of the OR terms.
    ToggleOr,
    /// Context menu entries.
    Replace,
    And,
    Or,
    Exclude,
    Remove,
}

/// How strongly the query uses a sidebar item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Highlight {
    #[default]
    None,
    /// One of the top-level OR terms.
    Weak,
    /// One of the top-level AND terms, or the whole query.
    Strong,
    /// Negated at the top level.
    Excluded,
}

/// Applies `edit` for the item written as `item` (a bound token or `#tag`).
pub fn apply(
    document: &QueryDocument,
    key: &ItemKey,
    item: Segment,
    edit: QueryEdit,
) -> QueryDocument {
    let root = parse(document).expr;
    let single = || QueryDocument::from_segments(vec![item.clone()]);
    let term = || {
        Expr::Term(TermNode {
            term: Term::Ignore,
            span: 0..0,
            source: item.clone(),
        })
    };

    let next = match edit {
        QueryEdit::Click => {
            if root.as_ref().is_some_and(|expr| is_item(expr, key)) {
                None
            } else {
                return single();
            }
        }
        QueryEdit::Replace => return single(),
        QueryEdit::ToggleAnd if in_and_terms(root.as_ref(), key) => remove(root, key, Scope::And),
        QueryEdit::ToggleOr if in_or_terms(root.as_ref(), key) => remove(root, key, Scope::Or),
        QueryEdit::ToggleAnd | QueryEdit::And => Some(join_and(root, term())),
        QueryEdit::ToggleOr | QueryEdit::Or => Some(join_or(root, term())),
        QueryEdit::Exclude => Some(join_and(root, Expr::Not(Box::new(term())))),
        QueryEdit::Remove => remove(root, key, Scope::Any),
    };

    let mut segments = Vec::new();
    if let Some(expr) = &next {
        print(expr, &mut segments);
    }
    QueryDocument::from_segments(segments)
}

/// What each sidebar item looks like under the current query.
pub fn highlights(parsed: &Parsed) -> Vec<(ItemKey, Highlight)> {
    let mut found = Vec::new();
    let mut note = |expr: &Expr, level: Highlight| {
        let (expr, level) = match expr {
            Expr::Not(inner) if level == Highlight::Strong => (inner.as_ref(), Highlight::Excluded),
            other => (other, level),
        };
        if let Some(key) = key_of(expr) {
            found.push((key, level));
        }
    };
    match &parsed.expr {
        Some(Expr::And(items)) => items.iter().for_each(|item| note(item, Highlight::Strong)),
        Some(Expr::Or(items)) => items.iter().for_each(|item| note(item, Highlight::Weak)),
        Some(other) => note(other, Highlight::Strong),
        None => {}
    }
    found
}

/// The item a term stands for, if it is one the sidebar shows.
fn key_of(expr: &Expr) -> Option<ItemKey> {
    let Expr::Term(node) = expr else { return None };
    match &node.term {
        Term::Ref(reference) => Some(ItemKey::Ref(reference.clone())),
        Term::Tag(name) => Some(ItemKey::Tag(name.clone())),
        _ => None,
    }
}

/// The key a pinned tag is compared by.
pub fn tag_key(name: &str) -> ItemKey {
    ItemKey::Tag(fold(name))
}

fn is_item(expr: &Expr, key: &ItemKey) -> bool {
    key_of(expr).as_ref() == Some(key)
}

fn is_negated_item(expr: &Expr, key: &ItemKey) -> bool {
    matches!(expr, Expr::Not(inner) if is_item(inner, key))
}

fn in_and_terms(root: Option<&Expr>, key: &ItemKey) -> bool {
    match root {
        Some(Expr::And(items)) => items.iter().any(|item| is_item(item, key)),
        Some(other) => is_item(other, key),
        None => false,
    }
}

fn in_or_terms(root: Option<&Expr>, key: &ItemKey) -> bool {
    match root {
        Some(Expr::Or(items)) => items.iter().any(|item| is_item(item, key)),
        Some(other) => is_item(other, key),
        None => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    And,
    Or,
    Any,
}

fn remove(root: Option<Expr>, key: &ItemKey, scope: Scope) -> Option<Expr> {
    let and = scope != Scope::Or;
    let or = scope != Scope::And;
    match root? {
        expr if is_item(&expr, key) => None,
        expr if and && is_negated_item(&expr, key) => None,
        Expr::And(items) if and => combine(
            items
                .into_iter()
                .filter(|item| !is_item(item, key) && !is_negated_item(item, key))
                .collect(),
            Expr::And,
        ),
        Expr::Or(items) if or => combine(
            items
                .into_iter()
                .filter(|item| !is_item(item, key))
                .collect(),
            Expr::Or,
        ),
        other => Some(other),
    }
}

fn join_and(root: Option<Expr>, addition: Expr) -> Expr {
    match root {
        None => addition,
        Some(Expr::And(mut items)) => {
            items.push(addition);
            Expr::And(items)
        }
        Some(other) => Expr::And(vec![other, addition]),
    }
}

fn join_or(root: Option<Expr>, addition: Expr) -> Expr {
    match root {
        None => addition,
        Some(Expr::Or(mut items)) => {
            items.push(addition);
            Expr::Or(items)
        }
        Some(other) => Expr::Or(vec![other, addition]),
    }
}

fn combine(mut items: Vec<Expr>, group: fn(Vec<Expr>) -> Expr) -> Option<Expr> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(group(items)),
    }
}

/// Writes a tree back as segments, with parentheses only where needed.
fn print(expr: &Expr, out: &mut Vec<Segment>) {
    let text = |out: &mut Vec<Segment>, value: &str| out.push(Segment::Text(value.to_string()));
    match expr {
        Expr::And(items) | Expr::Or(items) => {
            let (separator, strength) = match expr {
                Expr::And(_) => (" ", 2),
                _ => (" | ", 1),
            };
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    text(out, separator);
                }
                print_operand(item, strength, out);
            }
        }
        Expr::Not(inner) => {
            text(out, "-");
            print_operand(inner, 3, out);
        }
        Expr::Term(node) => out.push(node.source.clone()),
    }
}

fn print_operand(expr: &Expr, strength: u8, out: &mut Vec<Segment>) {
    if expr.precedence() < strength {
        out.push(Segment::Text("(".into()));
        print(expr, out);
        out.push(Segment::Text(")".into()));
    } else {
        print(expr, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodels::search::due::{DueName, DueSpec};

    fn due(name: DueName, text: &str) -> (ItemKey, Segment) {
        let reference = Reference::Due(DueSpec::Named(name));
        (
            ItemKey::Ref(reference.clone()),
            Segment::Bound {
                text: text.into(),
                reference,
            },
        )
    }

    fn work() -> (ItemKey, Segment) {
        let reference = Reference::Project("p-work".into());
        (
            ItemKey::Ref(reference.clone()),
            Segment::Bound {
                text: "@仕事".into(),
                reference,
            },
        )
    }

    fn run(start: QueryDocument, item: &(ItemKey, Segment), edit: QueryEdit) -> QueryDocument {
        apply(&start, &item.0, item.1.clone(), edit)
    }

    fn today_or_overdue() -> QueryDocument {
        let today = due(DueName::Today, "@今日");
        let overdue = due(DueName::Overdue, "@期限切れ");
        run(
            QueryDocument::from_segments(vec![today.1]),
            &overdue,
            QueryEdit::ToggleOr,
        )
    }

    #[test]
    fn click_replaces_and_a_second_click_clears() {
        let item = work();
        let once = run(QueryDocument::from_text("milk"), &item, QueryEdit::Click);
        assert_eq!(once.text(), "@仕事");
        assert!(run(once, &item, QueryEdit::Click).is_empty());
    }

    #[test]
    fn adding_with_and_groups_an_existing_or() {
        let query = today_or_overdue();
        assert_eq!(query.text(), "@今日 | @期限切れ");
        assert_eq!(
            run(query.clone(), &work(), QueryEdit::ToggleAnd).text(),
            "(@今日 | @期限切れ) @仕事"
        );
        assert_eq!(
            run(query.clone(), &work(), QueryEdit::ToggleOr).text(),
            "@今日 | @期限切れ | @仕事"
        );
        assert_eq!(
            run(query, &work(), QueryEdit::Exclude).text(),
            "(@今日 | @期限切れ) -@仕事"
        );
    }

    #[test]
    fn toggling_takes_a_term_back_out() {
        let with_work = run(
            QueryDocument::from_text("milk"),
            &work(),
            QueryEdit::ToggleAnd,
        );
        assert_eq!(with_work.text(), "milk @仕事");
        assert_eq!(run(with_work, &work(), QueryEdit::ToggleAnd).text(), "milk");

        let today = due(DueName::Today, "@今日");
        let either = today_or_overdue();
        assert_eq!(run(either, &today, QueryEdit::ToggleOr).text(), "@期限切れ");
    }

    #[test]
    fn remove_finds_the_item_in_and_or_and_negated_terms() {
        let excluded = run(
            QueryDocument::from_text("milk"),
            &work(),
            QueryEdit::Exclude,
        );
        assert_eq!(excluded.text(), "milk -@仕事");
        assert_eq!(run(excluded, &work(), QueryEdit::Remove).text(), "milk");
    }

    #[test]
    fn a_confirmed_token_survives_the_rewrite() {
        let query = run(QueryDocument::from_text("a | b"), &work(), QueryEdit::And);
        assert!(
            query
                .segments()
                .iter()
                .any(|segment| matches!(segment, Segment::Bound { .. }))
        );
    }

    #[test]
    fn highlights_follow_the_top_level_shape() {
        let (today_key, _) = due(DueName::Today, "@今日");
        let (work_key, _) = work();

        let and = run(QueryDocument::from_text("milk"), &work(), QueryEdit::And);
        let and = run(and, &due(DueName::Today, "@今日"), QueryEdit::Exclude);
        let found = highlights(&parse(&and));
        assert!(found.contains(&(work_key.clone(), Highlight::Strong)));
        assert!(found.contains(&(today_key.clone(), Highlight::Excluded)));

        let or = today_or_overdue();
        assert!(highlights(&parse(&or)).contains(&(today_key.clone(), Highlight::Weak)));

        // "(@今日 | @期限切れ) @仕事": the grouped terms are not lit.
        let grouped = run(today_or_overdue(), &work(), QueryEdit::And);
        let found = highlights(&parse(&grouped));
        assert_eq!(found, vec![(work_key, Highlight::Strong)]);
        assert!(!found.iter().any(|(key, _)| *key == today_key));
    }

    #[test]
    fn a_pinned_tag_is_matched_by_name() {
        let tag = (tag_key("Shop"), Segment::Text("#Shop".into()));
        let query = run(QueryDocument::from_text("milk"), &tag, QueryEdit::ToggleAnd);
        assert_eq!(query.text(), "milk #Shop");
        assert_eq!(
            highlights(&parse(&query)).last(),
            Some(&(tag_key("shop"), Highlight::Strong))
        );
        assert_eq!(run(query, &tag, QueryEdit::ToggleAnd).text(), "milk");
    }
}
