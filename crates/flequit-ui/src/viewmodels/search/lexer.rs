//! Splits search text into tokens and reads each word as a search term.
//!
//! Every token keeps its byte range in the text it came from, so the caller can
//! replace a single token (a suggestion being picked, a name being bound to its
//! target) without re-serialising the rest of what the user typed.

use std::ops::Range;

use super::normalize::{fold, is_word_break, syntax_char};
use super::vocabulary::{RefKind, TextField};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    LParen,
    RParen,
    Or,
    And,
    Not,
    Word,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the text that was tokenised.
    pub span: Range<usize>,
}

/// Tokenises `text`. Never fails: anything unexpected becomes a word.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let single = |kind| Token {
            kind,
            span: start..start + ch.len_utf8(),
        };
        match syntax_char(ch) {
            '(' => {
                tokens.push(single(TokenKind::LParen));
                chars.next();
            }
            ')' => {
                tokens.push(single(TokenKind::RParen));
                chars.next();
            }
            '|' => {
                tokens.push(single(TokenKind::Or));
                chars.next();
            }
            '-' => {
                chars.next();
                // Only a prefix negates. A lone `-` is still being typed.
                if chars.peek().is_some_and(|(_, next)| !is_word_break(*next)) {
                    tokens.push(single(TokenKind::Not));
                }
            }
            _ => {
                let end = word_end(text, start);
                let kind = match &text[start..end] {
                    "AND" => TokenKind::And,
                    "OR" => TokenKind::Or,
                    "NOT" => TokenKind::Not,
                    _ => TokenKind::Word,
                };
                tokens.push(Token {
                    kind,
                    span: start..end,
                });
                while chars.peek().is_some_and(|(index, _)| *index < end) {
                    chars.next();
                }
            }
        }
    }
    tokens
}

/// Where the word starting at `start` ends. Breaks inside quotes do not count.
fn word_end(text: &str, start: usize) -> usize {
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in text[start..].char_indices() {
        let ch_is_quote = syntax_char(ch) == '"';
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch_is_quote {
                quoted = false;
            }
            continue;
        }
        if ch_is_quote {
            quoted = true;
        } else if is_word_break(ch) {
            return start + index;
        }
    }
    text.len()
}

/// What a single word means, before any name is looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawTerm {
    /// Free text, folded.
    Text(String),
    /// A tag prefix, folded.
    Tag(String),
    /// `@task:` and friends: a partial match on one text field, folded.
    Field(TextField, String),
    /// An `@` word naming a due date, a state, a project, a list or a user.
    At(AtWord),
    /// Nothing to search for (`@`, `#`, an empty quote).
    Ignore,
}

/// An `@` word, split into its parts but not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtWord {
    /// Set by `@<kind>:`.
    pub kind: Option<RefKind>,
    /// Name parts separated by unquoted dots, unquoted but not folded.
    pub parts: Vec<String>,
    /// Whether any part was quoted. A quoted name is never a keyword.
    pub quoted: bool,
    /// Whether an unquoted `*` makes this a pattern.
    pub wildcard: bool,
    /// `{…}` after a kind: an id written by the internal query.
    pub id: Option<String>,
}

impl AtWord {
    /// The parts joined back into one name, e.g. for a project called `v1.2`.
    pub fn whole_name(&self) -> String {
        self.parts.join(".")
    }
}

/// Reads one word token.
pub fn read_term(word: &str) -> RawTerm {
    let mut chars = word.chars();
    match chars.next().map(syntax_char) {
        Some('#') => {
            let tag = unquote(chars.as_str());
            if tag.is_empty() {
                RawTerm::Ignore
            } else {
                RawTerm::Tag(fold(&tag))
            }
        }
        Some('@') => read_at(chars.as_str()),
        Some(_) => {
            let text = unquote(word);
            if text.is_empty() {
                RawTerm::Ignore
            } else {
                RawTerm::Text(fold(&text))
            }
        }
        None => RawTerm::Ignore,
    }
}

fn read_at(rest: &str) -> RawTerm {
    if rest.is_empty() {
        return RawTerm::Ignore;
    }

    if let Some((prefix, value)) = split_kind(rest) {
        let prefix = fold(&prefix);
        if let Some(field) = TextField::parse(&prefix) {
            let value = unquote(value);
            return if value.is_empty() {
                RawTerm::Ignore
            } else {
                RawTerm::Field(field, fold(&value))
            };
        }
        if let Some(kind) = RefKind::parse(&prefix) {
            if value.is_empty() {
                return RawTerm::Ignore;
            }
            let mut word = split_name(value);
            word.kind = Some(kind);
            if !word.quoted && word.parts.len() == 1 {
                let part = &word.parts[0];
                if part.len() > 2 && part.starts_with('{') && part.ends_with('}') {
                    word.id = Some(part[1..part.len() - 1].to_string());
                }
            }
            return RawTerm::At(word);
        }
    }

    RawTerm::At(split_name(rest))
}

/// Splits `<kind>:<value>` at the first unquoted colon.
fn split_kind(rest: &str) -> Option<(String, &str)> {
    if rest.starts_with('"') {
        return None;
    }
    rest.char_indices()
        .find(|(_, ch)| syntax_char(*ch) == ':')
        .filter(|(index, _)| !rest[..*index].contains('"'))
        .map(|(index, ch)| (rest[..index].to_string(), &rest[index + ch.len_utf8()..]))
}

/// Splits a name at unquoted dots and removes the quotes.
fn split_name(text: &str) -> AtWord {
    let mut parts = vec![String::new()];
    let mut quoted = false;
    let mut any_quoted = false;
    let mut wildcard = false;
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        let symbol = syntax_char(ch);
        if quoted {
            match ch {
                '\\' => {
                    if let Some(next) = chars.next() {
                        push_char(&mut parts, next);
                    }
                }
                _ if symbol == '"' => quoted = false,
                _ => push_char(&mut parts, ch),
            }
            continue;
        }
        match symbol {
            '"' => {
                quoted = true;
                any_quoted = true;
            }
            '.' => parts.push(String::new()),
            '*' => {
                wildcard = true;
                push_char(&mut parts, '*');
            }
            _ => push_char(&mut parts, ch),
        }
    }

    // `@仕事.` is a project followed by a list still being typed.
    if parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }

    AtWord {
        kind: None,
        parts,
        quoted: any_quoted,
        wildcard,
        id: None,
    }
}

fn push_char(parts: &mut [String], ch: char) {
    if let Some(last) = parts.last_mut() {
        last.push(ch);
    }
}

/// Removes surrounding quotes and escapes, if the text is quoted.
fn unquote(text: &str) -> String {
    let mut chars = text.chars();
    if chars.next().map(syntax_char) != Some('"') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if syntax_char(ch) == '"' {
            break;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(TokenKind, &str)> {
        tokenize(text)
            .into_iter()
            .map(|token| (token.kind, &text[token.span]))
            .collect()
    }

    #[test]
    fn operators_groups_and_words_are_told_apart() {
        assert_eq!(
            kinds("(@今日 | @期限切れ) -#保留 AND 牛乳 OR x NOT y"),
            vec![
                (TokenKind::LParen, "("),
                (TokenKind::Word, "@今日"),
                (TokenKind::Or, "|"),
                (TokenKind::Word, "@期限切れ"),
                (TokenKind::RParen, ")"),
                (TokenKind::Not, "-"),
                (TokenKind::Word, "#保留"),
                (TokenKind::And, "AND"),
                (TokenKind::Word, "牛乳"),
                (TokenKind::Or, "OR"),
                (TokenKind::Word, "x"),
                (TokenKind::Not, "NOT"),
                (TokenKind::Word, "y"),
            ]
        );
    }

    #[test]
    fn full_width_symbols_and_spaces_work_like_ascii() {
        assert_eq!(
            kinds("（＠今日｜＃仕事）　牛乳"),
            vec![
                (TokenKind::LParen, "（"),
                (TokenKind::Word, "＠今日"),
                (TokenKind::Or, "｜"),
                (TokenKind::Word, "＃仕事"),
                (TokenKind::RParen, "）"),
                (TokenKind::Word, "牛乳"),
            ]
        );
    }

    #[test]
    fn quotes_keep_spaces_and_symbols_inside_one_word() {
        assert_eq!(
            kinds(r#"@"My (old) Project".Inbox "a | b""#),
            vec![
                (TokenKind::Word, r#"@"My (old) Project".Inbox"#),
                (TokenKind::Word, r#""a | b""#),
            ]
        );
    }

    #[test]
    fn a_lone_or_trailing_minus_is_dropped_and_lowercase_words_are_text() {
        assert_eq!(kinds("- a -"), vec![(TokenKind::Word, "a")]);
        assert_eq!(
            kinds("a or b"),
            vec![
                (TokenKind::Word, "a"),
                (TokenKind::Word, "or"),
                (TokenKind::Word, "b")
            ]
        );
    }

    #[test]
    fn at_words_split_into_kind_parts_and_flags() {
        let RawTerm::At(word) = read_term(r#"@プロジェクト:"v1.2""#) else {
            panic!("an @ word");
        };
        assert_eq!(word.kind, Some(RefKind::Project));
        assert_eq!(word.parts, vec!["v1.2".to_string()]);
        assert!(word.quoted);

        let RawTerm::At(word) = read_term("@仕事.今週やる") else {
            panic!("an @ word");
        };
        assert_eq!(word.kind, None);
        assert_eq!(word.parts, vec!["仕事".to_string(), "今週やる".to_string()]);

        let RawTerm::At(word) = read_term("@project:*仕*") else {
            panic!("an @ word");
        };
        assert!(word.wildcard);

        let RawTerm::At(word) = read_term("@list:{abc}") else {
            panic!("an @ word");
        };
        assert_eq!(word.id.as_deref(), Some("abc"));
    }

    #[test]
    fn fields_tags_and_text_are_folded() {
        assert_eq!(
            read_term("@note:ＭＩＬＫ"),
            RawTerm::Field(TextField::Note, "milk".into())
        );
        assert_eq!(read_term("#Shop"), RawTerm::Tag("shop".into()));
        assert_eq!(
            read_term(r#""Two Words""#),
            RawTerm::Text("two words".into())
        );
        assert_eq!(read_term("@"), RawTerm::Ignore);
        assert_eq!(read_term("#"), RawTerm::Ignore);
        assert_eq!(read_term("@user:"), RawTerm::Ignore);
    }
}
