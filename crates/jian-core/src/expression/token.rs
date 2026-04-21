//! Tokens produced by the lexer.

use super::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(f64),
    String(String),
    TemplateStart,        // `  opening backtick
    TemplateText(String), // literal part between ${} slices
    TemplateExprStart,    // ${
    TemplateExprEnd,      // } inside a template (context-sensitive)
    TemplateEnd,          // closing backtick
    True,
    False,
    Null,

    // Identifiers
    Ident(String),    // bare name: foo, bar, filter
    ScopeRef(String), // starts with $: $app, $state, $self, $item, $route, …

    // Punctuation
    LParen,
    RParen, // ( )
    LBracket,
    RBracket, // [ ]
    LBrace,
    RBrace, // { }
    Comma,
    Colon,
    Dot,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang, // !
    Eq,
    NotEq,
    EqStrict,
    NotEqStrict,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AndAnd,
    OrOr,
    NullishCoalesce,
    Question,

    Eof,
}
