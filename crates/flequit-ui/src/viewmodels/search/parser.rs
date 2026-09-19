//! Builds the condition tree of a [`QueryDocument`].
//!
//! ```text
//! query    = [ or_expr ] ;
//! or_expr  = and_expr , { ( "|" | "OR" ) , and_expr } ;
//! and_expr = unary , { [ "AND" ] , unary } ;
//! unary    = ( "-" | "NOT" ) , unary | primary ;
//! primary  = "(" , or_expr , [ ")" ] | term ;
//! ```
//!
//! The parser never fails. The text is usually half typed, so a missing `)` is
//! assumed at the end and stray operators are skipped; each repair is reported
//! so the search box can say that part of the query was ignored.

use std::ops::Range;

use super::document::{QueryDocument, Segment};
use super::lexer::{AtWord, RawTerm, TokenKind, read_term, tokenize};
use super::normalize::{is_word_break, syntax_char};
use super::resolve::Reference;
use super::vocabulary::TextField;

/// One condition before any name has been looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Text(String),
    Tag(String),
    Field(TextField, String),
    /// An `@` word still in the text: resolved by name when evaluated.
    Name(AtWord),
    /// A confirmed token.
    Ref(Reference),
    /// An ambiguous token: any of the candidates.
    AnyOf(Vec<Reference>),
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermNode {
    pub term: Term,
    /// Byte range in the document text.
    pub span: Range<usize>,
    /// The segment the term was read from, reused when the tree is written back.
    pub source: Segment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Term(TermNode),
}

impl Expr {
    /// Binding strength, for writing the tree back with the fewest parentheses.
    pub(super) fn precedence(&self) -> u8 {
        match self {
            Self::Or(_) => 1,
            Self::And(_) => 2,
            Self::Not(_) => 3,
            Self::Term(_) => 4,
        }
    }
}

/// A part of the query that was skipped while parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxIssue {
    /// A `)` without a matching `(`.
    StrayParen,
    /// An `|` with nothing on one side.
    StrayOr,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub expr: Option<Expr>,
    pub issues: Vec<SyntaxIssue>,
}

struct DocToken {
    kind: TokenKind,
    span: Range<usize>,
    term: Option<(Term, Segment)>,
}

/// Parses the document's text into a condition tree.
pub fn parse(document: &QueryDocument) -> Parsed {
    let tokens = document_tokens(document);
    let mut parser = Parser {
        tokens,
        position: 0,
        depth: 0,
        issues: Vec::new(),
    };
    let expr = parser.parse_or();
    Parsed {
        expr,
        issues: parser.issues,
    }
}

fn document_tokens(document: &QueryDocument) -> Vec<DocToken> {
    let mut tokens = Vec::new();
    let segments = document.segments();
    for (position, (segment, span)) in segments.iter().zip(document.spans()).enumerate() {
        let whole = |term: Term| DocToken {
            kind: TokenKind::Word,
            span: span.clone(),
            term: Some((term, segment.clone())),
        };
        match segment {
            Segment::Text(text) => {
                for token in tokenize(text) {
                    let global = span.start + token.span.start..span.start + token.span.end;
                    let term = (token.kind == TokenKind::Word).then(|| {
                        let raw = &text[token.span.clone()];
                        let term = match read_term(raw) {
                            RawTerm::Text(value) => Term::Text(value),
                            RawTerm::Tag(value) => Term::Tag(value),
                            RawTerm::Field(field, value) => Term::Field(field, value),
                            RawTerm::At(word) => Term::Name(word),
                            RawTerm::Ignore => Term::Ignore,
                        };
                        (term, Segment::Text(raw.to_string()))
                    });
                    tokens.push(DocToken {
                        kind: token.kind,
                        span: global,
                        term,
                    });
                }
                // `-@仕事` where `@仕事` is confirmed: the minus ends this text
                // segment, so the lexer took it for a lone `-` still being typed.
                let next_is_token = segments
                    .get(position + 1)
                    .is_some_and(|next| !matches!(next, Segment::Text(_)));
                if next_is_token && ends_with_prefix_minus(text) {
                    let end = span.end;
                    tokens.push(DocToken {
                        kind: TokenKind::Not,
                        span: end - 1..end,
                        term: None,
                    });
                }
            }
            Segment::Bound { reference, .. } => tokens.push(whole(Term::Ref(reference.clone()))),
            Segment::Ambiguous { candidates, .. } => {
                tokens.push(whole(Term::AnyOf(candidates.clone())));
            }
            Segment::Broken(_) => tokens.push(whole(Term::Ignore)),
        }
    }
    tokens
}

/// Whether `text` ends in a `-` that starts a word rather than ending one.
fn ends_with_prefix_minus(text: &str) -> bool {
    let mut chars = text.chars().rev();
    if chars.next().map(syntax_char) != Some('-') {
        return false;
    }
    chars
        .next()
        .is_none_or(|before| is_word_break(before) || syntax_char(before) == '-')
}

struct Parser {
    tokens: Vec<DocToken>,
    position: usize,
    depth: usize,
    issues: Vec<SyntaxIssue>,
}

impl Parser {
    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.position).map(|token| &token.kind)
    }

    fn parse_or(&mut self) -> Option<Expr> {
        let mut items = Vec::new();
        let mut expecting_operand = true;
        loop {
            match self.peek() {
                None => break,
                Some(TokenKind::RParen) => {
                    if self.depth > 0 {
                        break;
                    }
                    self.position += 1;
                    self.issues.push(SyntaxIssue::StrayParen);
                }
                Some(TokenKind::Or) => {
                    self.position += 1;
                    if expecting_operand {
                        self.issues.push(SyntaxIssue::StrayOr);
                    }
                    expecting_operand = true;
                }
                Some(_) => {
                    if let Some(expr) = self.parse_and() {
                        items.push(expr);
                    }
                    expecting_operand = false;
                }
            }
        }
        if expecting_operand && !items.is_empty() {
            self.issues.push(SyntaxIssue::StrayOr);
        }
        combine(items, Expr::Or)
    }

    fn parse_and(&mut self) -> Option<Expr> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(TokenKind::Or | TokenKind::RParen) => break,
                Some(TokenKind::And) => self.position += 1,
                Some(_) => {
                    if let Some(expr) = self.parse_unary() {
                        items.push(expr);
                    }
                }
            }
        }
        combine(items, Expr::And)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        match self.peek()? {
            TokenKind::Not => {
                self.position += 1;
                match self.peek() {
                    None | Some(TokenKind::Or | TokenKind::RParen | TokenKind::And) => None,
                    Some(_) => self.parse_unary().map(|inner| Expr::Not(Box::new(inner))),
                }
            }
            TokenKind::LParen => {
                self.position += 1;
                self.depth += 1;
                let inner = self.parse_or();
                self.depth -= 1;
                if self.peek() == Some(&TokenKind::RParen) {
                    self.position += 1;
                }
                inner
            }
            _ => {
                let token = &mut self.tokens[self.position];
                self.position += 1;
                let (term, source) = token.term.take()?;
                Some(Expr::Term(TermNode {
                    term,
                    span: token.span.clone(),
                    source,
                }))
            }
        }
    }
}

fn combine(mut items: Vec<Expr>, group: fn(Vec<Expr>) -> Expr) -> Option<Expr> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(group(items)),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Renders a tree compactly for assertions: `&(a,|(b,c))`, `!x`.
    pub fn shape(expr: &Expr) -> String {
        match expr {
            Expr::And(items) => format!("&({})", join(items)),
            Expr::Or(items) => format!("|({})", join(items)),
            Expr::Not(inner) => format!("!{}", shape(inner)),
            Expr::Term(node) => node.source.text().to_string(),
        }
    }

    fn join(items: &[Expr]) -> String {
        items.iter().map(shape).collect::<Vec<_>>().join(",")
    }

    fn parsed(text: &str) -> (String, Vec<SyntaxIssue>) {
        let parsed = parse(&QueryDocument::from_text(text));
        (
            parsed.expr.as_ref().map(shape).unwrap_or_default(),
            parsed.issues,
        )
    }

    #[test]
    fn not_binds_tighter_than_and_which_binds_tighter_than_or() {
        assert_eq!(parsed("a | b c").0, "|(a,&(b,c))");
        assert_eq!(parsed("-a b").0, "&(!a,b)");
        assert_eq!(parsed("(a | b) c").0, "&(|(a,b),c)");
        assert_eq!(parsed("a AND b OR NOT c").0, "|(&(a,b),!c)");
    }

    #[test]
    fn half_typed_queries_are_repaired_and_reported() {
        assert_eq!(parsed("(a | b"), ("|(a,b)".into(), vec![]));
        assert_eq!(parsed("a |"), ("a".into(), vec![SyntaxIssue::StrayOr]));
        assert_eq!(parsed("| a"), ("a".into(), vec![SyntaxIssue::StrayOr]));
        assert_eq!(
            parsed("a | | b"),
            ("|(a,b)".into(), vec![SyntaxIssue::StrayOr])
        );
        assert_eq!(parsed(")a"), ("a".into(), vec![SyntaxIssue::StrayParen]));
        assert_eq!(parsed("() a"), ("a".into(), vec![]));
        assert_eq!(parsed("- a"), ("a".into(), vec![]));
        assert_eq!(parsed(""), (String::new(), vec![]));
    }
}
