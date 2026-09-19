//! The search box's state between keystrokes.
//!
//! Plain data with no Slint types, so the ViewModel can keep it in its shared
//! state and tests can drive it without a window.

use std::ops::Range;

use super::document::{QueryDocument, Segment};
use super::eval::Prepared;
use super::parser::parse;
use super::resolve::{NameIndex, Reference};
use super::sidebar::{self, Highlight, ItemKey, QueryEdit};
use super::suggestion::{self, ActiveToken, Suggestion, SuggestionTarget};
use super::vocabulary::Lang;

#[derive(Debug, Clone, Default)]
pub struct SearchSession {
    document: QueryDocument,
    /// Where the last edit ended; the word there is still being typed.
    active: Option<usize>,
    /// The ambiguous token whose candidates are shown.
    choosing: Option<usize>,
    /// The word the current suggestions would replace.
    suggesting: Option<ActiveToken>,
}

/// Everything the search box and the task list need after a change.
#[derive(Debug, Clone, Default)]
pub struct SearchView {
    pub text: String,
    pub prepared: Prepared,
    pub suggestions: Vec<Suggestion>,
    /// Non-empty while the disambiguation list is open.
    pub candidates: Vec<Suggestion>,
    /// `@` words that name nothing.
    pub unresolved: Vec<String>,
    /// `@` words still naming several targets.
    pub ambiguous: Vec<String>,
    /// Whether a stray `)` or `|` was skipped.
    pub syntax_issue: bool,
    pub highlights: Vec<(ItemKey, Highlight)>,
}

impl SearchSession {
    pub fn text(&self) -> String {
        self.document.text()
    }

    /// The language-neutral query kept in the settings file.
    pub fn canonical(&self) -> String {
        self.document.canonical()
    }

    pub fn is_empty(&self) -> bool {
        self.document.is_empty()
    }

    /// Replaces the whole query with a stored one.
    pub fn restore(&mut self, canonical: &str, index: &NameIndex, lang: Lang) {
        *self = Self {
            document: QueryDocument::from_canonical(canonical, index, lang),
            ..Self::default()
        };
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Takes in what the text field now says.
    ///
    /// Words the edit moved away from are confirmed; if one of them names
    /// several targets, its candidates open.
    pub fn edit(&mut self, text: &str, index: &NameIndex) {
        let active = self.document.apply_edit(text);
        self.active = Some(active);
        if let Some(start) = self.document.commit(index, Some(active)) {
            self.choosing = Some(start);
        }
    }

    /// Confirms the word being typed too (Enter).
    pub fn commit(&mut self, index: &NameIndex) {
        self.active = None;
        self.suggesting = None;
        if let Some(start) = self.document.commit(index, None) {
            self.choosing = Some(start);
        }
    }

    /// Puts the picked suggestion in place of the word being typed.
    pub fn choose_suggestion(
        &mut self,
        position: usize,
        index: &NameIndex,
        tags: &[String],
        lang: Lang,
    ) -> bool {
        let Some(token) = self.suggesting.take() else {
            return false;
        };
        let Some(picked) = suggestion::suggestions(&token, index, tags, lang)
            .into_iter()
            .nth(position)
        else {
            return false;
        };
        let segment = match picked.target {
            SuggestionTarget::Ref(reference) => {
                let explicit = token.kind().is_some();
                let Some(text) = index.render(&reference, lang, explicit) else {
                    return false;
                };
                Segment::Bound { text, reference }
            }
            SuggestionTarget::Tag(name) => Segment::Text(format!("#{name}")),
        };
        self.replace(token.span, segment);
        true
    }

    /// Narrows the open ambiguous token to its `position`-th candidate.
    pub fn choose_candidate(&mut self, position: usize) -> bool {
        let Some(start) = self.choosing.take() else {
            return false;
        };
        let Some(reference) = self
            .document
            .ambiguous_at(start)
            .and_then(|candidates| candidates.get(position))
            .cloned()
        else {
            return false;
        };
        self.document.choose(start, reference)
    }

    pub fn dismiss_candidates(&mut self) {
        self.choosing = None;
    }

    /// Opens the candidates of the first token that is still ambiguous.
    pub fn reopen_candidates(&mut self) {
        self.choosing = self.document.first_ambiguous();
    }

    /// Rewrites the query for a sidebar item.
    pub fn apply_item(&mut self, key: &ItemKey, item: Segment, edit: QueryEdit, index: &NameIndex) {
        self.commit(index);
        self.choosing = None;
        self.document = sidebar::apply(&self.document, key, item, edit);
    }

    /// Follows renames, deletions and, with `relocalize`, a language switch.
    pub fn refresh(&mut self, index: &NameIndex, lang: Lang, relocalize: bool) {
        self.document.refresh(index, lang, relocalize);
        self.suggesting = None;
    }

    /// Computes what to show. Also remembers which word the suggestions are for.
    pub fn view(&mut self, index: &NameIndex, tags: &[String], lang: Lang) -> SearchView {
        let parsed = parse(&self.document);
        let prepared = Prepared::new(&parsed, index, self.active);

        self.suggesting = self
            .active
            .and_then(|active| suggestion::active_token(&self.document, active));
        let suggestions = self
            .suggesting
            .as_ref()
            .map(|token| suggestion::suggestions(token, index, tags, lang))
            .unwrap_or_default();

        let candidates = match self
            .choosing
            .and_then(|start| self.document.ambiguous_at(start))
        {
            Some(references) => suggestion::candidates(references, index, lang),
            None => {
                self.choosing = None;
                Vec::new()
            }
        };

        let ambiguous = self
            .document
            .segments()
            .iter()
            .filter_map(|segment| match segment {
                Segment::Ambiguous { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let mut unresolved = prepared.unresolved.clone();
        unresolved.extend(
            self.document
                .segments()
                .iter()
                .filter_map(|segment| match segment {
                    Segment::Broken(text) => Some(text.clone()),
                    _ => None,
                }),
        );

        SearchView {
            text: self.document.text(),
            highlights: sidebar::highlights(&parsed),
            syntax_issue: !parsed.issues.is_empty(),
            prepared,
            suggestions,
            candidates,
            unresolved,
            ambiguous,
        }
    }

    /// The query with its names looked up, for filtering the task list.
    pub fn prepared(&self, index: &NameIndex) -> Prepared {
        Prepared::new(&parse(&self.document), index, self.active)
    }

    /// How the query uses each sidebar item.
    pub fn highlights(&self) -> Vec<(ItemKey, Highlight)> {
        sidebar::highlights(&parse(&self.document))
    }

    /// The references the query narrows by with AND, e.g. its one list.
    pub fn strong_references(&self) -> Vec<Reference> {
        self.highlights()
            .into_iter()
            .filter_map(|(key, highlight)| match (key, highlight) {
                (ItemKey::Ref(reference), Highlight::Strong) => Some(reference),
                _ => None,
            })
            .collect()
    }

    fn replace(&mut self, span: Range<usize>, segment: Segment) {
        self.document.replace_range(span, segment);
        self.active = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodels::search::resolve::tests::index;

    fn typed(session: &mut SearchSession, text: &str) {
        // A text field reports every intermediate state.
        for end in text.char_indices().map(|(index, ch)| index + ch.len_utf8()) {
            session.edit(&text[..end], &index());
        }
    }

    #[test]
    fn typing_past_an_ambiguous_word_opens_its_candidates() {
        let mut session = SearchSession::default();
        typed(&mut session, "@山田 ");
        let view = session.view(&index(), &[], Lang::Ja);
        assert_eq!(view.candidates.len(), 2);
        assert_eq!(view.ambiguous, vec!["@山田".to_string()]);

        assert!(session.choose_candidate(1));
        assert_eq!(session.canonical(), "@user:{u-yamada} ");
        assert!(session.view(&index(), &[], Lang::Ja).candidates.is_empty());
    }

    #[test]
    fn dismissing_leaves_the_word_ambiguous_until_it_is_reopened() {
        let mut session = SearchSession::default();
        typed(&mut session, "@山田");
        session.commit(&index());
        session.dismiss_candidates();
        assert!(session.view(&index(), &[], Lang::Ja).candidates.is_empty());
        session.reopen_candidates();
        assert_eq!(session.view(&index(), &[], Lang::Ja).candidates.len(), 2);
    }

    #[test]
    fn a_picked_suggestion_is_confirmed_in_place() {
        let mut session = SearchSession::default();
        typed(&mut session, "milk @hom");
        let view = session.view(&index(), &[], Lang::En);
        assert_eq!(view.suggestions[0].label, "@Home");

        assert!(session.choose_suggestion(0, &index(), &[], Lang::En));
        assert_eq!(session.text(), "milk @Home");
        assert_eq!(session.canonical(), "milk @project:{p-home}");
    }

    #[test]
    fn the_word_being_typed_narrows_only_once_it_names_something() {
        let mut session = SearchSession::default();
        typed(&mut session, "@Hom");
        let view = session.view(&index(), &[], Lang::En);
        assert!(view.prepared.is_empty());
        assert!(view.unresolved.is_empty());

        typed(&mut session, "@Home");
        assert!(!session.view(&index(), &[], Lang::En).prepared.is_empty());
    }

    #[test]
    fn an_unknown_word_is_reported_once_it_is_left_behind() {
        let mut session = SearchSession::default();
        typed(&mut session, "@nobody x");
        assert_eq!(
            session.view(&index(), &[], Lang::En).unresolved,
            vec!["@nobody".to_string()]
        );
    }

    #[test]
    fn a_sidebar_item_confirms_what_was_typed_first() {
        let mut session = SearchSession::default();
        typed(&mut session, "@Home");
        let reference = Reference::Project("p-work".into());
        let item = Segment::Bound {
            text: "@仕事".into(),
            reference: reference.clone(),
        };
        session.apply_item(
            &ItemKey::Ref(reference),
            item,
            QueryEdit::ToggleOr,
            &index(),
        );
        assert_eq!(session.canonical(), "@project:{p-home} | @project:{p-work}");
    }
}
