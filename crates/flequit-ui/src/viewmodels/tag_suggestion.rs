//! Candidates offered under the detail pane's "add a tag" field.
//!
//! Pure: the pane sends what is in the field — including text the IME is still
//! composing — and this picks which of the project's tags to list.
//!
//! Matching is fuzzy rather than prefix- or suffix-bound. The query and each
//! name are folded first (NFKC, lowercase, katakana to hiragana), so `WORK`
//! finds `work`, and `かいぎ` typed mid-composition finds `カイギ`. A name
//! matches when the query's characters appear in it in order; closer matches
//! are listed first:
//!
//! 1. the whole name,
//! 2. the start of the name,
//! 3. a contiguous run, earlier first,
//! 4. scattered characters, tighter first.
//!
//! Ties keep the order the tags were given in, which is the project's own.

use super::search::fold;

/// Suggestions shown when the setting is missing or out of range.
pub const DEFAULT_LIMIT: usize = 5;

/// The most suggestions the setting may ask for.
pub const MAX_LIMIT: usize = 20;

/// Folds text so that case, width and kana script do not matter.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::tag_suggestion::fold_for_match;
///
/// assert_eq!(fold_for_match("ＷｏＲＫ"), "work");
/// assert_eq!(fold_for_match("カイギ"), "かいぎ");
/// assert_eq!(fold_for_match("ｶｲｷﾞ"), "かいぎ");
/// ```
pub fn fold_for_match(text: &str) -> String {
    fold(text).chars().map(katakana_to_hiragana).collect()
}

/// Picks up to `limit` items whose name fits `query`, best match first.
///
/// An empty query (or one that is only `#`) fits every item, so focusing the
/// empty field lists the first few tags in their usual order.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::tag_suggestion::suggest;
///
/// let tags = ["homework", "work", "network", "wide-orbit"];
/// let picked = suggest(&tags, |tag| tag, "WORK", 3);
/// assert_eq!(picked, [&"work", &"network", &"homework"]);
/// ```
pub fn suggest<'a, T>(
    items: &'a [T],
    name: impl Fn(&T) -> &str,
    query: &str,
    limit: usize,
) -> Vec<&'a T> {
    let query = fold_for_match(query.trim());
    let query = query.strip_prefix('#').unwrap_or(&query);
    let query: Vec<char> = query.chars().collect();

    let mut ranked: Vec<(Rank, usize, &T)> = items
        .iter()
        .enumerate()
        .filter_map(|(position, item)| {
            let name: Vec<char> = fold_for_match(name(item)).chars().collect();
            rank(&name, &query).map(|rank| (rank, position, item))
        })
        .collect();
    ranked.sort_by_key(|(rank, position, _)| (*rank, *position));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, item)| item)
        .collect()
}

/// How closely a name matched; smaller is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Exact,
    Prefix,
    /// Where the run starts, in characters.
    Contiguous(usize),
    /// Characters skipped between the first and last matched one.
    Scattered(usize),
}

fn rank(name: &[char], query: &[char]) -> Option<Rank> {
    if query.is_empty() {
        return Some(Rank::Exact);
    }
    if name == query {
        return Some(Rank::Exact);
    }
    if name.starts_with(query) {
        return Some(Rank::Prefix);
    }
    if let Some(start) = name.windows(query.len()).position(|window| window == query) {
        return Some(Rank::Contiguous(start));
    }
    tightest_subsequence(name, query).map(Rank::Scattered)
}

/// The fewest skipped characters over every in-order placement of `query`.
///
/// Tried from each place the first character occurs, taking the earliest
/// match for the rest: that is the tightest placement starting there.
fn tightest_subsequence(name: &[char], query: &[char]) -> Option<usize> {
    let (first, rest) = query.split_first()?;
    name.iter()
        .enumerate()
        .filter(|(_, ch)| *ch == first)
        .filter_map(|(start, _)| {
            let mut end = start;
            for wanted in rest {
                end += 1 + name.get(end + 1..)?.iter().position(|ch| ch == wanted)?;
            }
            Some(end + 1 - start - query.len())
        })
        .min()
}

/// Maps a katakana letter to its hiragana counterpart; leaves anything else.
///
/// Only the range with a direct counterpart (ァ..ヶ) moves; the long-vowel mark
/// and the letters with no hiragana form are shared or kept as they are.
fn katakana_to_hiragana(ch: char) -> char {
    match ch {
        'ァ'..='ヶ' => char::from_u32(ch as u32 - 0x60).unwrap_or(ch),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(tags: &'a [&'a str], query: &str, limit: usize) -> Vec<&'a str> {
        suggest(tags, |tag| tag, query, limit)
            .into_iter()
            .copied()
            .collect()
    }

    #[test]
    fn case_and_width_are_ignored() {
        let tags = ["Work", "ｗｏｒｋｓｈｏｐ", "home"];

        assert_eq!(names(&tags, "WORK", 5), ["Work", "ｗｏｒｋｓｈｏｐ"]);
        assert_eq!(names(&tags, "ｗｏ", 5), ["Work", "ｗｏｒｋｓｈｏｐ"]);
    }

    #[test]
    fn hiragana_and_katakana_find_each_other() {
        let tags = ["カイギ", "かいもの", "ｶｲｼｬ"];

        assert_eq!(names(&tags, "かい", 5), ["カイギ", "かいもの", "ｶｲｼｬ"]);
        assert_eq!(names(&tags, "モノ", 5), ["かいもの"]);
    }

    #[test]
    fn a_partly_composed_word_already_narrows_the_list() {
        // What the IME shows while "会議" is being typed: kana first, then the
        // converted kanji. Each stage is a query of its own.
        let tags = ["会議", "会議室", "かいぎ準備", "開発"];

        assert_eq!(names(&tags, "か", 5), ["かいぎ準備"]);
        assert_eq!(names(&tags, "かいぎ", 5), ["かいぎ準備"]);
        assert_eq!(names(&tags, "会", 5), ["会議", "会議室"]);
        assert_eq!(names(&tags, "会議", 5), ["会議", "会議室"]);
    }

    #[test]
    fn matches_anywhere_not_only_at_either_end() {
        let tags = ["homework", "work", "network", "worker", "unrelated"];

        // Exact, then prefix, then by where the run starts.
        assert_eq!(
            names(&tags, "work", 5),
            ["work", "worker", "network", "homework"]
        );
    }

    #[test]
    fn scattered_characters_match_after_contiguous_ones() {
        let tags = ["w-o-r-k", "wreck", "work", "walk"];

        assert_eq!(names(&tags, "wrk", 5), ["work", "wreck", "w-o-r-k"]);
    }

    #[test]
    fn the_tightest_placement_counts() {
        // The first "a" gives a loose placement; the later one is tighter.
        let tags = ["axxxxbxaxb", "axxxb"];

        assert_eq!(names(&tags, "ab", 5), ["axxxxbxaxb", "axxxb"]);
        assert_eq!(tightest_subsequence(&['a', 'x', 'b'], &['a', 'b']), Some(1));
        assert_eq!(tightest_subsequence(&['b', 'a'], &['a', 'b']), None);
    }

    #[test]
    fn the_limit_caps_the_list_and_an_empty_query_keeps_the_order() {
        let tags = ["c", "a", "b", "d"];

        assert_eq!(names(&tags, "", 3), ["c", "a", "b"]);
        assert_eq!(names(&tags, "  #", 2), ["c", "a"]);
        assert!(names(&tags, "", 0).is_empty());
    }

    #[test]
    fn a_leading_hash_is_ignored() {
        let tags = ["home", "work"];

        assert_eq!(names(&tags, "#wo", 5), ["work"]);
        assert_eq!(names(&tags, "＃wo", 5), ["work"]);
    }

    #[test]
    fn nothing_is_offered_when_no_name_fits() {
        let tags = ["home", "work"];

        assert!(names(&tags, "xyz", 5).is_empty());
    }
}
