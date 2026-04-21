//! Recursive-descent parser following the grammar in spec 02 §2.2.

use super::ast::{BinaryOp, Expr, ExprKind, TemplatePart, UnaryOp};
use super::diag::Diagnostic;
use super::lexer::lex;
use super::token::{Token, TokenKind};

pub fn parse(src: &str) -> Result<Expr, Diagnostic> {
    let tokens = lex(src)?;
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    let e = p.parse_expr()?;
    p.expect(TokenKind::Eof, "expected end of expression")?;
    Ok(e)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn eat(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: TokenKind, msg: &str) -> Result<Token, Diagnostic> {
        let t = self.peek().clone();
        if std::mem::discriminant(&t.kind) == std::mem::discriminant(&expected) {
            self.pos += 1;
            Ok(t)
        } else {
            Err(Diagnostic::parse(
                format!("{} (found `{:?}`)", msg, t.kind),
                t.span,
            ))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, Diagnostic> {
        let cond = self.parse_logical_or()?;
        if matches!(self.peek().kind, TokenKind::Question) {
            self.eat();
            let then_branch = self.parse_expr()?;
            self.expect(TokenKind::Colon, "expected `:` in ternary")?;
            let else_branch = self.parse_expr()?;
            let span = cond.span.merge(else_branch.span);
            return Ok(Expr {
                kind: ExprKind::Ternary(
                    Box::new(cond),
                    Box::new(then_branch),
                    Box::new(else_branch),
                ),
                span,
            });
        }
        Ok(cond)
    }

    fn parse_logical_or(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_logical_and()?;
        while matches!(self.peek().kind, TokenKind::OrOr) {
            self.eat();
            let right = self.parse_logical_and()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(BinaryOp::Or, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_nullish()?;
        while matches!(self.peek().kind, TokenKind::AndAnd) {
            self.eat();
            let right = self.parse_nullish()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(BinaryOp::And, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn parse_nullish(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_equality()?;
        while matches!(self.peek().kind, TokenKind::NullishCoalesce) {
            self.eat();
            let right = self.parse_equality()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(BinaryOp::Nullish, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                TokenKind::EqStrict => BinaryOp::EqStrict,
                TokenKind::NotEqStrict => BinaryOp::NotEqStrict,
                _ => return Ok(left),
            };
            self.eat();
            let right = self.parse_comparison()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => return Ok(left),
            };
            self.eat();
            let right = self.parse_additive()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn parse_additive(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_mult()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.eat();
            let right = self.parse_mult()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn parse_mult(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => return Ok(left),
            };
            self.eat();
            let right = self.parse_unary()?;
            let span = left.span.merge(right.span);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        match self.peek().kind {
            TokenKind::Bang => {
                let t = self.eat();
                let rhs = self.parse_unary()?;
                let span = t.span.merge(rhs.span);
                Ok(Expr {
                    kind: ExprKind::Unary(UnaryOp::Not, Box::new(rhs)),
                    span,
                })
            }
            TokenKind::Minus => {
                let t = self.eat();
                let rhs = self.parse_unary()?;
                let span = t.span.merge(rhs.span);
                Ok(Expr {
                    kind: ExprKind::Unary(UnaryOp::Neg, Box::new(rhs)),
                    span,
                })
            }
            TokenKind::Plus => {
                let t = self.eat();
                let rhs = self.parse_unary()?;
                let span = t.span.merge(rhs.span);
                Ok(Expr {
                    kind: ExprKind::Unary(UnaryOp::Pos, Box::new(rhs)),
                    span,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.eat();
                    let name_tok = self.eat();
                    let name = match name_tok.kind {
                        TokenKind::Ident(ref s) => s.clone(),
                        _ => {
                            return Err(Diagnostic::parse(
                                "expected identifier after `.`",
                                name_tok.span,
                            ))
                        }
                    };
                    let span = e.span.merge(name_tok.span);
                    e = Expr {
                        kind: ExprKind::Member(Box::new(e), name),
                        span,
                    };
                }
                TokenKind::LBracket => {
                    self.eat();
                    let idx = self.parse_expr()?;
                    let rb = self.expect(TokenKind::RBracket, "expected `]`")?;
                    let span = e.span.merge(rb.span);
                    e = Expr {
                        kind: ExprKind::Index(Box::new(e), Box::new(idx)),
                        span,
                    };
                }
                TokenKind::LParen => {
                    self.eat();
                    let mut args = Vec::new();
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        args.push(self.parse_expr()?);
                        while matches!(self.peek().kind, TokenKind::Comma) {
                            self.eat();
                            args.push(self.parse_expr()?);
                        }
                    }
                    let rp = self.expect(TokenKind::RParen, "expected `)`")?;
                    let span = e.span.merge(rp.span);
                    e = Expr {
                        kind: ExprKind::Call(Box::new(e), args),
                        span,
                    };
                }
                _ => return Ok(e),
            }
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Number(n) => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::Number(n),
                    span: tok.span,
                })
            }
            TokenKind::String(ref s) => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::String(s.clone()),
                    span: tok.span,
                })
            }
            TokenKind::True => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span: tok.span,
                })
            }
            TokenKind::False => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span: tok.span,
                })
            }
            TokenKind::Null => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::Null,
                    span: tok.span,
                })
            }
            TokenKind::Ident(ref name) => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::Identifier(name.clone()),
                    span: tok.span,
                })
            }
            TokenKind::ScopeRef(ref s) => {
                self.eat();
                Ok(Expr {
                    kind: ExprKind::ScopeRef(s.clone(), Vec::new()),
                    span: tok.span,
                })
            }
            TokenKind::LParen => {
                self.eat();
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen, "expected `)`")?;
                Ok(e)
            }
            TokenKind::LBracket => {
                let start = tok.span;
                self.eat();
                let mut items = Vec::new();
                if !matches!(self.peek().kind, TokenKind::RBracket) {
                    items.push(self.parse_expr()?);
                    while matches!(self.peek().kind, TokenKind::Comma) {
                        self.eat();
                        if matches!(self.peek().kind, TokenKind::RBracket) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                }
                let rb = self.expect(TokenKind::RBracket, "expected `]`")?;
                Ok(Expr {
                    kind: ExprKind::Array(items),
                    span: start.merge(rb.span),
                })
            }
            TokenKind::LBrace => {
                let start = tok.span;
                self.eat();
                let mut entries = Vec::new();
                if !matches!(self.peek().kind, TokenKind::RBrace) {
                    loop {
                        let key_tok = self.eat();
                        let key = match &key_tok.kind {
                            TokenKind::Ident(s) => s.clone(),
                            TokenKind::String(s) => s.clone(),
                            _ => {
                                return Err(Diagnostic::parse(
                                    "expected object key",
                                    key_tok.span,
                                ))
                            }
                        };
                        self.expect(TokenKind::Colon, "expected `:` in object literal")?;
                        let v = self.parse_expr()?;
                        entries.push((key, v));
                        if matches!(self.peek().kind, TokenKind::Comma) {
                            self.eat();
                            continue;
                        }
                        break;
                    }
                }
                let rb = self.expect(TokenKind::RBrace, "expected `}`")?;
                Ok(Expr {
                    kind: ExprKind::Object(entries),
                    span: start.merge(rb.span),
                })
            }
            TokenKind::TemplateStart => self.parse_template(),
            ref other => Err(Diagnostic::parse(
                format!("unexpected token `{:?}` at start of expression", other),
                tok.span,
            )),
        }
    }

    fn parse_template(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.eat().span; // TemplateStart
        let mut parts = Vec::new();
        loop {
            let tok = self.eat();
            match tok.kind {
                TokenKind::TemplateText(s) => parts.push(TemplatePart::Text(s)),
                TokenKind::TemplateExprStart => {
                    let inner = self.parse_expr()?;
                    self.expect(
                        TokenKind::TemplateExprEnd,
                        "expected `}` closing template expression",
                    )?;
                    parts.push(TemplatePart::Expr(inner));
                }
                TokenKind::TemplateEnd => {
                    return Ok(Expr {
                        kind: ExprKind::Template(parts),
                        span: start.merge(tok.span),
                    });
                }
                _ => {
                    return Err(Diagnostic::parse(
                        format!("unexpected token in template: {:?}", tok.kind),
                        tok.span,
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Expr {
        parse(s).unwrap()
    }

    #[test]
    fn number_literal() {
        let e = p("42");
        assert!(matches!(e.kind, ExprKind::Number(n) if n == 42.0));
    }

    #[test]
    fn add_left_assoc() {
        let e = p("1 + 2 + 3");
        match e.kind {
            ExprKind::Binary(BinaryOp::Add, l, r) => {
                assert!(matches!(&r.kind, ExprKind::Number(n) if *n == 3.0));
                assert!(matches!(&l.kind, ExprKind::Binary(BinaryOp::Add, _, _)));
            }
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn mult_tighter_than_add() {
        let e = p("1 + 2 * 3");
        match e.kind {
            ExprKind::Binary(BinaryOp::Add, l, r) => {
                assert!(matches!(&l.kind, ExprKind::Number(n) if *n == 1.0));
                assert!(matches!(&r.kind, ExprKind::Binary(BinaryOp::Mul, _, _)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ternary_right_assoc() {
        let e = p("a ? b : c ? d : e");
        match e.kind {
            ExprKind::Ternary(_, _, else_) => {
                assert!(matches!(else_.kind, ExprKind::Ternary(_, _, _)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn member_chain() {
        let e = p("$app.user.name");
        match &e.kind {
            ExprKind::Member(inner, name) => {
                assert_eq!(name, "name");
                match &inner.kind {
                    ExprKind::Member(inner2, name2) => {
                        assert_eq!(name2, "user");
                        assert!(
                            matches!(inner2.kind, ExprKind::ScopeRef(ref s, _) if s == "$app")
                        );
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn index_chain() {
        let e = p("$app.items[0].id");
        assert!(matches!(e.kind, ExprKind::Member(_, _)));
    }

    #[test]
    fn call_with_args() {
        let e = p("len($state.items)");
        match e.kind {
            ExprKind::Call(fn_, args) => {
                assert!(matches!(fn_.kind, ExprKind::Identifier(ref s) if s == "len"));
                assert_eq!(args.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn array_literal() {
        let e = p("[1, 2, 3]");
        assert!(matches!(e.kind, ExprKind::Array(ref a) if a.len() == 3));
    }

    #[test]
    fn object_literal() {
        let e = p(r#"{name: "Alice", age: 30}"#);
        match e.kind {
            ExprKind::Object(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].0, "name");
                assert_eq!(entries[1].0, "age");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn unary_negation() {
        let e = p("-5");
        assert!(matches!(e.kind, ExprKind::Unary(UnaryOp::Neg, _)));
    }

    #[test]
    fn template_with_expr() {
        let e = p("`count=${1+1}`");
        match e.kind {
            ExprKind::Template(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], TemplatePart::Text(s) if s == "count="));
                assert!(matches!(&parts[1], TemplatePart::Expr(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn error_on_unclosed_paren() {
        assert!(parse("(1 + 2").is_err());
    }
}
