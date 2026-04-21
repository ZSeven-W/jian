//! AST produced by the parser. Enum-of-nodes; each node carries a span for
//! diagnostics.

use super::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    // Literals
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Expr>),
    Object(Vec<(String, Expr)>),

    // References
    Identifier(String), // bare name — resolves to builtin function or $self.<name>
    ScopeRef(String, Vec<AccessPath>), // $app, $app.user, $app.items[0].id, etc.
    Template(Vec<TemplatePart>),

    // Unary & binary
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),

    // Ternary
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),

    // Postfix
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessPath {
    Field(String),
    Index(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    EqStrict,
    NotEqStrict,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    Nullish,
}

impl Expr {
    pub fn lit_number(n: f64, span: Span) -> Self {
        Expr {
            kind: ExprKind::Number(n),
            span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_number() {
        let e = Expr::lit_number(42.0, Span::new(0, 2));
        assert!(matches!(e.kind, ExprKind::Number(n) if n == 42.0));
    }
}
