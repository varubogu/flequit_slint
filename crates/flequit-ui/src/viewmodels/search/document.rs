//! The search box's text together with what its `@` tokens were confirmed as.
//!
//! Slint's text field holds plain text only, so the document keeps the text as
//! a list of segments: free text as typed, and confirmed tokens carrying their
//! [`Reference`]. Each edit is diffed against the previous text so that tokens
//! outside the edited range keep their reference.

use std::ops::Range;

use super::lexer::{RawTerm, TokenKind, read_term, tokenize};
use super::normalize::syntax_char;
use super::resolve::{NameIndex, Reference, is_explicit};
use super::vocabulary::{Lang, RefKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Text as the user typed it: operators, free text, tags, and `@` words
    /// that are still being typed or that name nothing.
    Text(String),
    /// An `@` token confirmed as one target.
    Bound { text: String, reference: Reference },
    /// An `@` token that names several targets and has not been narrowed yet.
    Ambiguous {
        text: String,
        candidates: Vec<Reference>,
    },
    /// A confirmed token whose target was deleted. Kept, so that the condition
    /// does not silently loosen, and never re-bound until it is edited.
    Broken(String),
}

impl Segment {
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Broken(text) => text,
            Self::Bound { text, .. } | Self::Ambiguous { text, .. } => text,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryDocument {
    segments: Vec<Segment>,
}

impl QueryDocument {
    pub fn from_text(text: &str) -> Self {
        Self::from_segments(vec![Segment::Text(text.to_string())])
    }

    pub fn from_segments(segments: Vec<Segment>) -> Self {
        let mut document = Self { segments };
        document.tidy();
        document
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn text(&self) -> String {
        self.segments.iter().map(Segment::text).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| segment.text().trim().is_empty())
    }

    /// Byte ranges of every segment in [`Self::text`].
    pub fn spans(&self) -> Vec<Range<usize>> {
        let mut offset = 0;
        self.segments
            .iter()
            .map(|segment| {
                let start = offset;
                offset += segment.text().len();
                start..offset
            })
            .collect()
    }

    /// Replaces the text with what the field now holds.
    ///
    /// Returns where the edit ended in the new text, which stands in for the
    /// caret: Slint does not expose the caret position of a `LineEdit`.
    pub fn apply_edit(&mut self, new_text: &str) -> usize {
        let old_text = self.text();
        if old_text == new_text {
            return new_text.len();
        }

        let prefix = common_prefix(&old_text, new_text);
        let suffix = common_suffix(&old_text[prefix..], &new_text[prefix..]);
        let old_end = old_text.len() - suffix;
        let inserted = &new_text[prefix..new_text.len() - suffix];

        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut middle = String::new();
        for (segment, span) in self.segments.iter().zip(self.spans()) {
            if span.end <= prefix {
                before.push(segment.clone());
            } else if span.start >= old_end {
                after.push(segment.clone());
            } else {
                // Touched by the edit: whatever survives becomes plain text.
                // Only the first touched segment can start before the edit and
                // only the last can end after it, and nothing follows it yet.
                let text = segment.text();
                if span.start < prefix {
                    middle.push_str(&text[..prefix - span.start]);
                }
                if span.end > old_end {
                    after.push(Segment::Text(text[old_end - span.start..].to_string()));
                }
            }
        }
        middle.push_str(inserted);
        before.push(Segment::Text(middle));
        before.extend(after);

        self.segments = before;
        self.tidy();
        debug_assert_eq!(self.text(), new_text);
        prefix + inserted.len()
    }

    /// Confirms every `@` word that is not being typed at `active`.
    ///
    /// A word naming exactly one target becomes bound, one naming several
    /// becomes ambiguous; a word naming nothing, or a `*` pattern, stays text.
    /// Returns where the first newly ambiguous token starts.
    pub fn commit(&mut self, index: &NameIndex, active: Option<usize>) -> Option<usize> {
        let mut replacements = Vec::new();
        for (segment, span) in self.segments.iter().zip(self.spans()) {
            let Segment::Text(text) = segment else {
                continue;
            };
            for token in tokenize(text) {
                if token.kind != TokenKind::Word {
                    continue;
                }
                let global = span.start + token.span.start..span.start + token.span.end;
                if active.is_some_and(|at| global.start <= at && at <= global.end) {
                    continue;
                }
                let word_text = &text[token.span.clone()];
                let RawTerm::At(word) = read_term(word_text) else {
                    continue;
                };
                if word.wildcard {
                    continue;
                }
                let mut candidates = index.resolve(&word);
                let replacement = match candidates.len() {
                    0 => continue,
                    1 => Segment::Bound {
                        text: word_text.to_string(),
                        reference: candidates.remove(0),
                    },
                    _ => Segment::Ambiguous {
                        text: word_text.to_string(),
                        candidates,
                    },
                };
                replacements.push((global, replacement));
            }
        }

        let first_ambiguous = replacements
            .iter()
            .find(|(_, segment)| matches!(segment, Segment::Ambiguous { .. }))
            .map(|(span, _)| span.start);
        for (span, segment) in replacements.into_iter().rev() {
            self.replace_range(span, segment);
        }
        first_ambiguous
    }

    /// Puts `segment` in place of the text in `range`.
    ///
    /// Segments wholly inside the range are dropped; text segments that it cuts
    /// through keep their parts outside it.
    pub fn replace_range(&mut self, range: Range<usize>, segment: Segment) {
        let mut next = Vec::new();
        let mut placed = false;
        for (current, span) in self.segments.iter().zip(self.spans()) {
            if span.end <= range.start && span.start != range.start {
                next.push(current.clone());
                continue;
            }
            if span.start >= range.end && span.end != range.end {
                if !placed {
                    next.push(segment.clone());
                    placed = true;
                }
                next.push(current.clone());
                continue;
            }
            if let Segment::Text(text) = current {
                if span.start < range.start {
                    next.push(Segment::Text(text[..range.start - span.start].to_string()));
                }
                if !placed {
                    next.push(segment.clone());
                    placed = true;
                }
                if span.end > range.end {
                    next.push(Segment::Text(text[range.end - span.start..].to_string()));
                }
            } else if !placed {
                next.push(segment.clone());
                placed = true;
            }
        }
        if !placed {
            next.push(segment);
        }
        self.segments = next;
        self.tidy();
    }

    /// The ambiguous token starting at `start`, if it is still there.
    pub fn ambiguous_at(&self, start: usize) -> Option<&[Reference]> {
        self.segments
            .iter()
            .zip(self.spans())
            .find_map(|(segment, span)| match segment {
                Segment::Ambiguous { candidates, .. } if span.start == start => {
                    Some(candidates.as_slice())
                }
                _ => None,
            })
    }

    /// Where the first ambiguous token starts.
    pub fn first_ambiguous(&self) -> Option<usize> {
        self.segments
            .iter()
            .zip(self.spans())
            .find(|(segment, _)| matches!(segment, Segment::Ambiguous { .. }))
            .map(|(_, span)| span.start)
    }

    /// Narrows the ambiguous token at `start` to one of its candidates.
    pub fn choose(&mut self, start: usize, reference: Reference) -> bool {
        let spans = self.spans();
        for (segment, span) in self.segments.iter_mut().zip(spans) {
            if span.start != start {
                continue;
            }
            if let Segment::Ambiguous { text, candidates } = segment
                && candidates.contains(&reference)
            {
                *segment = Segment::Bound {
                    text: std::mem::take(text),
                    reference,
                };
                return true;
            }
        }
        false
    }

    /// Brings confirmed tokens up to date with the loaded data.
    ///
    /// A token whose text no longer names its target (a rename) is rewritten
    /// from the target; one whose target is gone becomes broken. With
    /// `relocalize`, keyword tokens are rewritten in `lang` whatever they say.
    pub fn refresh(&mut self, index: &NameIndex, lang: Lang, relocalize: bool) {
        for segment in &mut self.segments {
            match segment {
                Segment::Bound { text, reference } => {
                    if !index.exists(reference) {
                        *segment = Segment::Broken(std::mem::take(text));
                        continue;
                    }
                    let explicit = is_explicit(text);
                    let keyword = matches!(reference.kind(), RefKind::Due | RefKind::Status);
                    let stale = !index.describes(text, reference);
                    if (stale || (relocalize && (keyword || explicit)))
                        && let Some(rendered) = index.render(reference, lang, explicit)
                    {
                        *text = rendered;
                    }
                }
                Segment::Ambiguous { text, candidates } => {
                    candidates.retain(|candidate| index.exists(candidate));
                    match candidates.len() {
                        0 => *segment = Segment::Broken(std::mem::take(text)),
                        1 => {
                            *segment = Segment::Bound {
                                text: std::mem::take(text),
                                reference: candidates.remove(0),
                            }
                        }
                        _ => {}
                    }
                }
                Segment::Text(_) | Segment::Broken(_) => {}
            }
        }
        self.tidy();
    }

    /// The internal query: confirmed tokens as references, the rest as typed.
    pub fn canonical(&self) -> String {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Bound { reference, .. } => reference.canonical(),
                other => other.text().to_string(),
            })
            .collect()
    }

    /// Rebuilds a document from [`Self::canonical`] output.
    pub fn from_canonical(canonical: &str, index: &NameIndex, lang: Lang) -> Self {
        let mut document = Self::from_text(canonical);
        document.commit(index, None);
        for segment in &mut document.segments {
            if let Segment::Bound { text, reference } = segment
                && let Some(rendered) = index.render(reference, lang, false)
            {
                *text = rendered;
            }
        }
        document.tidy();
        document
    }

    /// Merges neighbouring text and unbinds tokens that an edit glued to a word.
    fn tidy(&mut self) {
        self.segments.retain(|segment| !segment.text().is_empty());

        // `@仕事` followed directly by `x` reads as the word `@仕事x`.
        let text = self.text();
        let spans = self.spans();
        for (segment, span) in self.segments.iter_mut().zip(spans) {
            if matches!(segment, Segment::Text(_)) {
                continue;
            }
            let before = text[..span.start].chars().next_back();
            let after = text[span.end..].chars().next();
            let open_before = before
                .is_none_or(|ch| ch.is_whitespace() || matches!(syntax_char(ch), '(' | '|' | '-'));
            let open_after =
                after.is_none_or(|ch| ch.is_whitespace() || matches!(syntax_char(ch), ')' | '|'));
            if !(open_before && open_after) {
                *segment = Segment::Text(segment.text().to_string());
            }
        }

        let mut merged: Vec<Segment> = Vec::with_capacity(self.segments.len());
        for segment in self.segments.drain(..) {
            if let (Some(Segment::Text(last)), Segment::Text(text)) = (merged.last_mut(), &segment)
            {
                last.push_str(text);
                continue;
            }
            merged.push(segment);
        }
        self.segments = merged;
    }
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, left), right)| left != right)
        .map_or_else(|| a.len().min(b.len()), |((index, _), _)| index)
}

fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodels::search::due::{DueName, DueSpec};
    use crate::viewmodels::search::resolve::tests::index;

    fn bound(text: &str, reference: Reference) -> Segment {
        Segment::Bound {
            text: text.into(),
            reference,
        }
    }

    fn today() -> Reference {
        Reference::Due(DueSpec::Named(DueName::Today))
    }

    #[test]
    fn a_finished_word_is_confirmed_and_the_word_being_typed_is_not() {
        let index = index();
        let mut document = QueryDocument::from_text("@Home @今週");
        let active = document.text().len();
        document.commit(&index, Some(active));

        assert_eq!(
            document.segments(),
            [
                bound("@Home", Reference::Project("p-home".into())),
                Segment::Text(" @今週".into()),
            ]
        );
    }

    #[test]
    fn a_word_naming_several_targets_becomes_ambiguous() {
        let index = index();
        let mut document = QueryDocument::from_text("x @山田");
        let first = document.commit(&index, None);

        assert_eq!(first, Some(2));
        assert_eq!(
            document.ambiguous_at(2),
            Some(
                [
                    Reference::Project("p-yamada".into()),
                    Reference::User("u-yamada".into())
                ]
                .as_slice()
            )
        );
        assert!(document.choose(2, Reference::User("u-yamada".into())));
        assert_eq!(
            document.segments()[1],
            bound("@山田", Reference::User("u-yamada".into()))
        );
    }

    #[test]
    fn edits_elsewhere_keep_a_token_confirmed() {
        let mut document = QueryDocument::from_segments(vec![
            bound("@今日", today()),
            Segment::Text(" milk".into()),
        ]);

        let active = document.apply_edit("@今日 milks");
        assert_eq!(active, "@今日 milks".len());
        assert_eq!(document.segments()[0], bound("@今日", today()));

        document.apply_edit("x @今日 milks");
        assert_eq!(document.segments()[1], bound("@今日", today()));
    }

    #[test]
    fn editing_inside_or_against_a_token_unbinds_it() {
        let mut document = QueryDocument::from_segments(vec![bound("@今日", today())]);
        document.apply_edit("@今日x");
        assert_eq!(document.segments(), [Segment::Text("@今日x".into())]);

        let mut document =
            QueryDocument::from_segments(vec![bound("@今日", today()), Segment::Text(" x".into())]);
        document.apply_edit("@今日x");
        assert_eq!(document.segments(), [Segment::Text("@今日x".into())]);

        let mut document = QueryDocument::from_segments(vec![bound("@今日", today())]);
        document.apply_edit("@今");
        assert_eq!(document.segments(), [Segment::Text("@今".into())]);
    }

    #[test]
    fn a_token_can_be_negated_or_grouped_without_unbinding() {
        // Typed one character at a time, as a text field reports it.
        let mut document = QueryDocument::from_segments(vec![bound("@今日", today())]);
        for step in ["(@今日", "(-@今日", "(-@今日)"] {
            document.apply_edit(step);
        }
        assert_eq!(document.segments()[1], bound("@今日", today()));
    }

    #[test]
    fn the_internal_query_round_trips_and_is_shown_in_the_ui_language() {
        let index = index();
        let document = QueryDocument::from_segments(vec![
            Segment::Text("(".into()),
            bound("@today", today()),
            Segment::Text(" | ".into()),
            bound("@Home", Reference::Project("p-home".into())),
            Segment::Text(") -#保留 牛乳".into()),
        ]);
        let canonical = document.canonical();
        assert_eq!(canonical, "(@due:today | @project:{p-home}) -#保留 牛乳");

        let restored = QueryDocument::from_canonical(&canonical, &index, Lang::Ja);
        assert_eq!(restored.text(), "(@今日 | @Home) -#保留 牛乳");
        assert_eq!(restored.canonical(), canonical);
    }

    #[test]
    fn a_rename_rewrites_the_token_and_a_deletion_breaks_it() {
        let mut document = QueryDocument::from_segments(vec![bound(
            "@OldName",
            Reference::Project("p-home".into()),
        )]);
        document.refresh(&index(), Lang::En, false);
        assert_eq!(document.text(), "@Home");

        let mut document =
            QueryDocument::from_segments(vec![bound("@Gone", Reference::Project("p-gone".into()))]);
        document.refresh(&index(), Lang::En, false);
        assert_eq!(document.segments(), [Segment::Broken("@Gone".into())]);
    }

    #[test]
    fn switching_language_rewrites_keywords_but_keeps_names_as_typed() {
        let mut document = QueryDocument::from_segments(vec![
            bound("@today", today()),
            Segment::Text(" ".into()),
            bound("@home", Reference::Project("p-home".into())),
        ]);
        document.refresh(&index(), Lang::Ja, true);
        assert_eq!(document.text(), "@今日 @home");
    }

    #[test]
    fn replacing_a_range_splits_the_text_around_it() {
        let mut document = QueryDocument::from_text("a @to b");
        document.replace_range(2..5, bound("@today", today()));
        assert_eq!(
            document.segments(),
            [
                Segment::Text("a ".into()),
                bound("@today", today()),
                Segment::Text(" b".into()),
            ]
        );
    }
}
