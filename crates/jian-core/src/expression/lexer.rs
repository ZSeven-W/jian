//! Hand-written lexer. Produces a Vec<Token> ending in `Eof`.
//!
//! Template literals (`Count: ${$state.count}`) use a small state machine:
//! inside backticks, text is consumed literally until `${`, at which point
//! we push `TemplateText + TemplateExprStart` and lex expression tokens until
//! the matching `}`, then resume template-text mode.

use super::diag::{Diagnostic, Span};
use super::token::{Token, TokenKind};

pub fn lex(src: &str) -> Result<Vec<Token>, Diagnostic> {
    Lexer::new(src).lex_all()
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.src.get(self.pos).copied()?;
        self.pos += 1;
        Some(c)
    }

    fn is_ident_start(c: u8) -> bool {
        c.is_ascii_alphabetic() || c == b'_'
    }
    fn is_ident_cont(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn lex_number(&mut self, start: usize) -> Result<Token, Diagnostic> {
        while let Some(c) = self.peek(0) {
            if c.is_ascii_digit() || c == b'.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        let n: f64 = text.parse().map_err(|_| {
            Diagnostic::lex(
                format!("invalid number `{}`", text),
                Span::new(start, self.pos),
            )
        })?;
        Ok(Token {
            kind: TokenKind::Number(n),
            span: Span::new(start, self.pos),
        })
    }

    fn lex_ident_or_keyword(&mut self, start: usize) -> Token {
        while let Some(c) = self.peek(0) {
            if Self::is_ident_cont(c) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        let kind = match text {
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            other => TokenKind::Ident(other.to_owned()),
        };
        Token {
            kind,
            span: Span::new(start, self.pos),
        }
    }

    fn lex_scope_ref(&mut self, start: usize) -> Result<Token, Diagnostic> {
        // Already consumed '$'
        while let Some(c) = self.peek(0) {
            if Self::is_ident_cont(c) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        if text.len() < 2 {
            return Err(Diagnostic::lex(
                "expected identifier after `$`",
                Span::new(start, self.pos),
            ));
        }
        Ok(Token {
            kind: TokenKind::ScopeRef(text.to_owned()),
            span: Span::new(start, self.pos),
        })
    }

    fn lex_string(&mut self, start: usize, quote: u8) -> Result<Token, Diagnostic> {
        // Already consumed opening quote at `start`.
        let mut buf = String::new();
        loop {
            let c = self
                .bump()
                .ok_or_else(|| Diagnostic::lex("unterminated string", Span::new(start, self.pos)))?;
            if c == quote {
                break;
            }
            if c == b'\\' {
                let nxt = self
                    .bump()
                    .ok_or_else(|| Diagnostic::lex("bad escape", Span::new(start, self.pos)))?;
                match nxt {
                    b'n' => buf.push('\n'),
                    b't' => buf.push('\t'),
                    b'r' => buf.push('\r'),
                    b'\\' => buf.push('\\'),
                    b'"' => buf.push('"'),
                    b'\'' => buf.push('\''),
                    b'`' => buf.push('`'),
                    _ => buf.push(nxt as char),
                }
            } else {
                buf.push(c as char);
            }
        }
        Ok(Token {
            kind: TokenKind::String(buf),
            span: Span::new(start, self.pos),
        })
    }

    fn lex_template(&mut self, start: usize) -> Result<Vec<Token>, Diagnostic> {
        // Consumed opening `
        let mut out = vec![Token {
            kind: TokenKind::TemplateStart,
            span: Span::new(start, start + 1),
        }];
        let mut text_buf = String::new();
        let mut text_start = self.pos;

        loop {
            let c = match self.peek(0) {
                None => {
                    return Err(Diagnostic::lex(
                        "unterminated template",
                        Span::new(start, self.pos),
                    ))
                }
                Some(c) => c,
            };
            if c == b'`' {
                if !text_buf.is_empty() {
                    out.push(Token {
                        kind: TokenKind::TemplateText(std::mem::take(&mut text_buf)),
                        span: Span::new(text_start, self.pos),
                    });
                }
                self.pos += 1;
                out.push(Token {
                    kind: TokenKind::TemplateEnd,
                    span: Span::new(self.pos - 1, self.pos),
                });
                return Ok(out);
            }
            if c == b'$' && self.peek(1) == Some(b'{') {
                if !text_buf.is_empty() {
                    out.push(Token {
                        kind: TokenKind::TemplateText(std::mem::take(&mut text_buf)),
                        span: Span::new(text_start, self.pos),
                    });
                }
                self.pos += 2;
                out.push(Token {
                    kind: TokenKind::TemplateExprStart,
                    span: Span::new(self.pos - 2, self.pos),
                });
                let mut depth = 1;
                while depth > 0 {
                    self.skip_whitespace();
                    let before = self.pos;
                    let tok = match self.peek(0) {
                        None => {
                            return Err(Diagnostic::lex(
                                "unterminated ${} in template",
                                Span::new(start, self.pos),
                            ))
                        }
                        Some(b'}') => {
                            self.pos += 1;
                            depth -= 1;
                            if depth == 0 {
                                out.push(Token {
                                    kind: TokenKind::TemplateExprEnd,
                                    span: Span::new(before, self.pos),
                                });
                                break;
                            }
                            Token {
                                kind: TokenKind::RBrace,
                                span: Span::new(before, self.pos),
                            }
                        }
                        Some(b'{') => {
                            self.pos += 1;
                            depth += 1;
                            Token {
                                kind: TokenKind::LBrace,
                                span: Span::new(before, self.pos),
                            }
                        }
                        Some(_) => self.lex_one()?,
                    };
                    out.push(tok);
                }
                text_start = self.pos;
                continue;
            }
            if c == b'\\' {
                self.pos += 1;
                let nxt = self.bump().ok_or_else(|| {
                    Diagnostic::lex("bad escape in template", Span::new(start, self.pos))
                })?;
                match nxt {
                    b'n' => text_buf.push('\n'),
                    b't' => text_buf.push('\t'),
                    b'\\' => text_buf.push('\\'),
                    b'`' => text_buf.push('`'),
                    b'$' => text_buf.push('$'),
                    _ => text_buf.push(nxt as char),
                }
                continue;
            }
            text_buf.push(c as char);
            self.pos += 1;
        }
    }

    /// Lex a single (non-template, non-string) token.
    fn lex_one(&mut self) -> Result<Token, Diagnostic> {
        self.skip_whitespace();
        let start = self.pos;
        let c = self.peek(0).ok_or_else(|| {
            Diagnostic::lex("unexpected end of input", Span::new(start, start))
        })?;

        if c.is_ascii_digit() {
            return self.lex_number(start);
        }
        if c == b'"' || c == b'\'' {
            self.pos += 1;
            return self.lex_string(start, c);
        }
        if c == b'$' {
            self.pos += 1;
            return self.lex_scope_ref(start);
        }
        if Self::is_ident_start(c) {
            return Ok(self.lex_ident_or_keyword(start));
        }

        self.pos += 1;
        let kind = match c {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b',' => TokenKind::Comma,
            b':' => TokenKind::Colon,
            b'.' => TokenKind::Dot,
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            b'%' => TokenKind::Percent,
            b'?' => {
                if self.peek(0) == Some(b'?') {
                    self.pos += 1;
                    TokenKind::NullishCoalesce
                } else {
                    TokenKind::Question
                }
            }
            b'!' => {
                if self.peek(0) == Some(b'=') {
                    self.pos += 1;
                    if self.peek(0) == Some(b'=') {
                        self.pos += 1;
                        TokenKind::NotEqStrict
                    } else {
                        TokenKind::NotEq
                    }
                } else {
                    TokenKind::Bang
                }
            }
            b'=' => {
                if self.peek(0) == Some(b'=') {
                    self.pos += 1;
                    if self.peek(0) == Some(b'=') {
                        self.pos += 1;
                        TokenKind::EqStrict
                    } else {
                        TokenKind::Eq
                    }
                } else {
                    return Err(Diagnostic::lex(
                        "single `=` is not valid in expressions",
                        Span::new(start, self.pos),
                    ));
                }
            }
            b'<' => {
                if self.peek(0) == Some(b'=') {
                    self.pos += 1;
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                if self.peek(0) == Some(b'=') {
                    self.pos += 1;
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'&' => {
                if self.peek(0) == Some(b'&') {
                    self.pos += 1;
                    TokenKind::AndAnd
                } else {
                    return Err(Diagnostic::lex(
                        "unexpected `&`",
                        Span::new(start, self.pos),
                    ));
                }
            }
            b'|' => {
                if self.peek(0) == Some(b'|') {
                    self.pos += 1;
                    TokenKind::OrOr
                } else {
                    return Err(Diagnostic::lex(
                        "unexpected `|`",
                        Span::new(start, self.pos),
                    ));
                }
            }
            _ => {
                return Err(Diagnostic::lex(
                    format!("unexpected character `{}`", c as char),
                    Span::new(start, self.pos),
                ))
            }
        };
        Ok(Token {
            kind,
            span: Span::new(start, self.pos),
        })
    }

    fn lex_all(&mut self) -> Result<Vec<Token>, Diagnostic> {
        let mut out = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.src.len() {
                break;
            }
            if self.peek(0) == Some(b'`') {
                let start = self.pos;
                self.pos += 1;
                let mut toks = self.lex_template(start)?;
                out.append(&mut toks);
                continue;
            }
            out.push(self.lex_one()?);
        }
        out.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.pos, self.pos),
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty_produces_eof() {
        assert_eq!(kinds(""), vec![TokenKind::Eof]);
    }

    #[test]
    fn integer_literal() {
        assert_eq!(kinds("42"), vec![TokenKind::Number(42.0), TokenKind::Eof]);
    }

    #[test]
    fn float_literal() {
        assert_eq!(kinds("3.14"), vec![TokenKind::Number(3.14), TokenKind::Eof]);
    }

    #[test]
    fn basic_arithmetic() {
        let k = kinds("1 + 2 * 3");
        assert_eq!(
            k,
            vec![
                TokenKind::Number(1.0),
                TokenKind::Plus,
                TokenKind::Number(2.0),
                TokenKind::Star,
                TokenKind::Number(3.0),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn ident_vs_keyword() {
        let k = kinds("true false null foo");
        assert_eq!(
            k,
            vec![
                TokenKind::True,
                TokenKind::False,
                TokenKind::Null,
                TokenKind::Ident("foo".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn scope_ref() {
        let k = kinds("$state.count");
        assert_eq!(
            k,
            vec![
                TokenKind::ScopeRef("$state".into()),
                TokenKind::Dot,
                TokenKind::Ident("count".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn string_with_escapes() {
        let k = kinds(r#""hello\n\"world\"""#);
        match &k[0] {
            TokenKind::String(s) => assert_eq!(s, "hello\n\"world\""),
            _ => panic!(),
        }
    }

    #[test]
    fn compound_operators() {
        let k = kinds("a == b !== c && d || e ?? f");
        assert!(k.iter().any(|t| matches!(t, TokenKind::Eq)));
        assert!(k.iter().any(|t| matches!(t, TokenKind::NotEqStrict)));
        assert!(k.iter().any(|t| matches!(t, TokenKind::AndAnd)));
        assert!(k.iter().any(|t| matches!(t, TokenKind::OrOr)));
        assert!(k.iter().any(|t| matches!(t, TokenKind::NullishCoalesce)));
    }

    #[test]
    fn template_simple() {
        let k = kinds("`hello`");
        assert_eq!(
            k,
            vec![
                TokenKind::TemplateStart,
                TokenKind::TemplateText("hello".into()),
                TokenKind::TemplateEnd,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn template_with_expr() {
        let k = kinds("`count=${$state.count}`");
        assert_eq!(
            k,
            vec![
                TokenKind::TemplateStart,
                TokenKind::TemplateText("count=".into()),
                TokenKind::TemplateExprStart,
                TokenKind::ScopeRef("$state".into()),
                TokenKind::Dot,
                TokenKind::Ident("count".into()),
                TokenKind::TemplateExprEnd,
                TokenKind::TemplateEnd,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn template_with_trailing_text() {
        let k = kinds("`pre ${x} post`");
        assert!(k
            .iter()
            .any(|t| matches!(t, TokenKind::TemplateText(s) if s == " post")));
    }

    #[test]
    fn error_on_unterminated_string() {
        assert!(lex(r#""hello"#).is_err());
    }

    #[test]
    fn error_on_unknown_char() {
        assert!(lex("@").is_err());
    }
}
