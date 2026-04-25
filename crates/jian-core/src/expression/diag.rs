//! Expression-layer diagnostics — errors & warnings with source spans.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn zero() -> Self {
        Self { start: 0, end: 0 }
    }
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    LexError,
    ParseError,
    CompileError,
    RuntimeWarning,
    UnknownIdentifier,
    TypeError,
    IndexOutOfBounds,
    DivisionByZero,
    UnknownFunction,
    ArityMismatch,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub kind: DiagKind,
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {}..{}: {}",
            self.kind, self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

impl Diagnostic {
    pub fn lex(msg: impl Into<String>, span: Span) -> Self {
        Self {
            kind: DiagKind::LexError,
            message: msg.into(),
            span,
        }
    }
    pub fn parse(msg: impl Into<String>, span: Span) -> Self {
        Self {
            kind: DiagKind::ParseError,
            message: msg.into(),
            span,
        }
    }
    /// Build a runtime-level warning at an unknown source location.
    /// Used by Action implementations that surface non-fatal advisory
    /// messages through `ctx.warn`.
    pub fn runtime_warning(msg: impl Into<String>) -> Self {
        Self {
            kind: DiagKind::RuntimeWarning,
            message: msg.into(),
            span: Span::zero(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_merge() {
        let a = Span::new(0, 3);
        let b = Span::new(5, 10);
        assert_eq!(a.merge(b), Span::new(0, 10));
    }
}
