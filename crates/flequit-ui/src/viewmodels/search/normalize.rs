//! Character-level normalisation shared by the lexer and the matcher.

use unicode_normalization::UnicodeNormalization;

/// Folds text for comparison: NFKC, then lowercase.
///
/// NFKC turns full-width letters and digits into their ASCII forms and
/// half-width katakana into full-width, so `ＡＢＣ`, `abc` and `ABC` compare
/// equal and a name typed with the IME still on finds its target.
pub fn fold(text: &str) -> String {
    text.nfkc().collect::<String>().to_lowercase()
}

/// Maps the full-width forms of the syntax characters to ASCII.
///
/// Applied to single characters while tokenising, so the byte offsets of the
/// original text stay valid.
pub fn syntax_char(ch: char) -> char {
    match ch {
        '＠' => '@',
        '＃' => '#',
        '（' => '(',
        '）' => ')',
        '｜' => '|',
        '＂' | '“' | '”' => '"',
        '．' => '.',
        '：' => ':',
        '－' => '-',
        '＊' => '*',
        other => other,
    }
}

/// Whether `ch` ends a word: whitespace (including the ideographic space) or
/// one of the grouping and OR symbols.
pub fn is_word_break(ch: char) -> bool {
    ch.is_whitespace() || matches!(syntax_char(ch), '(' | ')' | '|')
}

/// Quotes a name when writing it into the search box would split or change it.
pub fn quote_if_needed(name: &str) -> String {
    let needs_quotes = name.is_empty()
        || matches!(name, "AND" | "OR" | "NOT")
        || name.starts_with('-')
        || name.starts_with('{')
        || name.chars().any(|ch| {
            is_word_break(ch) || matches!(syntax_char(ch), '@' | '#' | '"' | '.' | ':' | '*')
        });
    if needs_quotes {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_and_case_fold_together() {
        assert_eq!(fold("ＷＯＲＫ"), "work");
        assert_eq!(fold("ｼｺﾞﾄ"), "シゴト");
    }

    #[test]
    fn a_plain_name_is_left_alone_and_a_tricky_one_is_quoted() {
        assert_eq!(quote_if_needed("仕事"), "仕事");
        assert_eq!(quote_if_needed("My Project"), "\"My Project\"");
        assert_eq!(quote_if_needed("v1.2"), "\"v1.2\"");
        assert_eq!(quote_if_needed("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_if_needed("OR"), "\"OR\"");
    }
}
