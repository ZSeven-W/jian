//! Rustc-style source-excerpt + caret rendering for `LoadWarning`.
//!
//! Writes diagnostics like:
//!
//! ```text
//! warning: unknown field `mysteryField`
//!  ┌─ examples/clean.op:7:3
//!  │
//! 7 │   "mysteryField": 42,
//!  │   ^^^^^^^^^^^^^^
//! ```
//!
//! When stdout is a TTY the severity label, gutter, and caret are wrapped
//! in ANSI color escapes (16-color fallback — yellow `warning`, red
//! `error`). Anywhere `IsTerminal` reports false (piped to a file, captured
//! in CI, NDJSON consumer) the output is plain ASCII so log scrapers can
//! tail it without escape-sequence noise.
//!
//! ## Span derivation
//!
//! `LoadWarning` is path-based today; the source-byte offset where a
//! field key or expression appears is recovered heuristically by
//! searching `source` for the field's quoted form (`"mysteryField"`)
//! or the literal expression substring. Heuristic mismatches degrade
//! to a "first-line / no-caret" fallback rather than failing — a
//! best-effort excerpt is still strictly more useful than `path: $`
//! alone.

use jian_ops_schema::error::LoadWarning;
use std::io::IsTerminal;

const C_RESET: &str = "\x1b[0m";
const C_BOLD: &str = "\x1b[1m";
const C_YELLOW: &str = "\x1b[33m";
const C_RED: &str = "\x1b[31m";
const C_BLUE: &str = "\x1b[34m";
const C_DIM: &str = "\x1b[2m";

/// 1-based source location with a byte underline length. `(0, 0, 0)` means
/// "no span recovered" — the renderer drops the excerpt and prints the
/// title line + path only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

impl Span {
    pub const NONE: Self = Self {
        line: 0,
        col: 0,
        len: 0,
    };
    pub fn is_some(&self) -> bool {
        self.line > 0
    }
}

/// Style probe: whether to emit ANSI colors. Cached on the type so tests
/// can override without poking environment.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    pub fn auto() -> Self {
        Self {
            color: std::io::stdout().is_terminal(),
        }
    }
    /// Force the no-color path. Currently used by tests so colour
    /// escapes don't leak into the snapshot strings.
    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false }
    }
}

/// Render every warning into `buf`. Caller decides where `buf` goes
/// (stdout / a string for tests). `path_label` is the user-facing file
/// path (e.g. `examples/counter.op`).
pub fn render_warnings(
    buf: &mut String,
    source: &str,
    path_label: &str,
    warnings: &[LoadWarning],
    style: Style,
) {
    for w in warnings {
        render_warning(buf, source, path_label, w, style);
    }
}

/// Render a single warning. Public for callers that want to interleave
/// with their own framing (e.g. a numbered list).
pub fn render_warning(
    buf: &mut String,
    source: &str,
    path_label: &str,
    w: &LoadWarning,
    style: Style,
) {
    let (severity, message, span) = describe(source, w);
    push_diagnostic(buf, source, path_label, severity, &message, span, style);
}

fn describe(source: &str, w: &LoadWarning) -> (Severity, String, Option<Span>) {
    match w {
        LoadWarning::UnknownField { path: _, field } => {
            let span = locate_quoted_key(source, field);
            (Severity::Warning, format!("unknown field `{}`", field), span)
        }
        LoadWarning::FutureFormatVersion {
            found,
            supported_max,
        } => {
            let span = locate_quoted_key(source, "formatVersion")
                .or_else(|| locate_quoted_key(source, "version"));
            (
                Severity::Warning,
                format!(
                    "formatVersion `{}` is newer than supported (`{}`); behaviour may be undefined",
                    found, supported_max
                ),
                span,
            )
        }
        LoadWarning::LogicModulesSkipped { reason } => (
            Severity::Warning,
            format!("`logicModules` skipped: {}", reason),
            locate_quoted_key(source, "logicModules"),
        ),
        LoadWarning::InvalidExpression { path, expr, reason } => {
            let span = locate_substring(source, expr);
            (
                Severity::Warning,
                format!("invalid expression at `{}`: `{}` — {}", path, expr, reason),
                span,
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
    fn color(self) -> &'static str {
        match self {
            Self::Warning => C_YELLOW,
            Self::Error => C_RED,
        }
    }
}

/// Render a free-form diagnostic (also used by `check`'s semantic-error
/// path). When `span` is `Span::NONE` the excerpt is omitted and the
/// `--> path` line still anchors the report.
pub fn push_diagnostic(
    buf: &mut String,
    source: &str,
    path_label: &str,
    severity: Severity,
    message: &str,
    span: Option<Span>,
    style: Style,
) {
    let span = span.filter(|s| s.is_some());
    let cstart = if style.color { severity.color() } else { "" };
    let cend = if style.color { C_RESET } else { "" };
    let cbold = if style.color { C_BOLD } else { "" };
    let cdim = if style.color { C_DIM } else { "" };
    let cblue = if style.color { C_BLUE } else { "" };

    // Title line: `warning: <message>`.
    buf.push_str(cstart);
    buf.push_str(cbold);
    buf.push_str(severity.label());
    buf.push_str(cend);
    buf.push_str(cbold);
    buf.push_str(": ");
    buf.push_str(message);
    buf.push_str(cend);
    buf.push('\n');

    if let Some(span) = span {
        // `<gutter>┌─ <path>:<line>:<col>` location anchor.
        let gutter = " ".repeat(line_number_width(span.line));
        buf.push_str(&gutter);
        buf.push_str(cblue);
        buf.push_str(" ┌─ ");
        buf.push_str(cend);
        buf.push_str(path_label);
        buf.push(':');
        buf.push_str(&span.line.to_string());
        buf.push(':');
        buf.push_str(&span.col.to_string());
        buf.push('\n');

        // Empty separator line.
        buf.push_str(&gutter);
        buf.push_str(cblue);
        buf.push_str(" │");
        buf.push_str(cend);
        buf.push('\n');

        // Source excerpt with line number on left.
        if let Some(line_text) = nth_line(source, span.line) {
            buf.push_str(cblue);
            buf.push_str(&format!("{} │ ", span.line));
            buf.push_str(cend);
            buf.push_str(line_text);
            buf.push('\n');

            // Caret line — pad to col, draw `^` * len (≥1).
            buf.push_str(&gutter);
            buf.push_str(cblue);
            buf.push_str(" │ ");
            buf.push_str(cend);
            buf.push_str(&" ".repeat(span.col.saturating_sub(1)));
            buf.push_str(cstart);
            buf.push_str(&"^".repeat(span.len.max(1)));
            buf.push_str(cend);
            buf.push('\n');
        }

        // Trailing dim line — visually closes the block.
        buf.push_str(&gutter);
        buf.push_str(cdim);
        buf.push_str(" │");
        buf.push_str(cend);
        buf.push('\n');
    } else {
        // No span: still print the `--> path` anchor without a line/col.
        buf.push_str(cblue);
        buf.push_str(" --> ");
        buf.push_str(cend);
        buf.push_str(path_label);
        buf.push('\n');
    }
}

fn line_number_width(line: usize) -> usize {
    if line == 0 {
        1
    } else {
        let mut n = line;
        let mut w = 0;
        while n > 0 {
            n /= 10;
            w += 1;
        }
        w
    }
}

/// Return the 1-based `line`-th line of `source` without its trailing
/// `\n`. `None` when `line == 0` or beyond the file's last line.
fn nth_line(source: &str, line: usize) -> Option<&str> {
    if line == 0 {
        return None;
    }
    source.split('\n').nth(line - 1).map(|l| l.trim_end_matches('\r'))
}

/// Find the byte offset of `"<key>"` in `source`. Returns `Span::NONE`
/// when the key isn't in the source as a quoted JSON key (e.g. the user
/// wrote a different key style, or the warning's field name is empty).
pub fn locate_quoted_key(source: &str, key: &str) -> Option<Span> {
    if key.is_empty() {
        return None;
    }
    // Match the quoted form (`"foo"`) so a substring like `bar.foo` in a
    // string value doesn't trigger. The `+1` skips the opening quote so
    // the caret lands on the key's first character, not the quote.
    let needle = format!("\"{}\"", key);
    let idx = source.find(&needle)?;
    let span_start = idx + 1;
    let span_len = key.len();
    let (line, col) = byte_offset_to_line_col(source, span_start);
    Some(Span {
        line,
        col,
        len: span_len,
    })
}

/// Find the byte offset of an arbitrary substring (used for
/// `InvalidExpression`).
pub fn locate_substring(source: &str, needle: &str) -> Option<Span> {
    if needle.is_empty() {
        return None;
    }
    let idx = source.find(needle)?;
    let (line, col) = byte_offset_to_line_col(source, idx);
    Some(Span {
        line,
        col,
        len: needle.chars().count().max(1),
    })
}

/// Map a byte offset into `source` to a 1-based `(line, col)` pair where
/// `col` counts unicode scalar values (not bytes), so a pre-caret prefix
/// of `中文` produces `col = 3` rather than `col = 7`.
fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1usize;
    let mut last_newline_byte = 0usize;
    for (idx, ch) in source.char_indices() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            last_newline_byte = idx + 1;
        }
    }
    let col_slice = &source[last_newline_byte..offset];
    let col = col_slice.chars().count() + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        // Two known fields then an unknown one on the *third* JSON line
        // (file line 4 because of the `{` on line 1).
        "{\n  \"id\": \"x\",\n  \"version\": \"1.0\",\n  \"mysteryField\": 42\n}\n".to_owned()
    }

    #[test]
    fn nth_line_indexes_one_based() {
        let s = "alpha\nbeta\ngamma";
        assert_eq!(nth_line(s, 1), Some("alpha"));
        assert_eq!(nth_line(s, 2), Some("beta"));
        assert_eq!(nth_line(s, 3), Some("gamma"));
        assert_eq!(nth_line(s, 0), None);
        assert_eq!(nth_line(s, 4), None);
    }

    #[test]
    fn nth_line_strips_carriage_returns() {
        let s = "alpha\r\nbeta\r\n";
        assert_eq!(nth_line(s, 1), Some("alpha"));
        assert_eq!(nth_line(s, 2), Some("beta"));
    }

    #[test]
    fn byte_offset_to_line_col_handles_multibyte() {
        let s = "中文\nabc";
        // 中 = 3 bytes, 文 = 3 bytes, \n = 1 byte → 'a' at byte 7, 'b' at byte 8.
        let (line, col) = byte_offset_to_line_col(s, 8);
        assert_eq!(line, 2);
        assert_eq!(col, 2);
        // First-line column is character-, not byte-counted.
        let (line2, col2) = byte_offset_to_line_col(s, 3);
        assert_eq!(line2, 1);
        assert_eq!(col2, 2, "col counts unicode scalar values, not bytes");
    }

    #[test]
    fn locate_quoted_key_finds_field() {
        let src = fixture();
        let span = locate_quoted_key(&src, "mysteryField").expect("found");
        assert_eq!(span.line, 4);
        // `  "` prefix → col 4 (after the two spaces and opening quote).
        assert_eq!(span.col, 4);
        assert_eq!(span.len, "mysteryField".len());
    }

    #[test]
    fn locate_quoted_key_returns_none_when_missing() {
        assert!(locate_quoted_key(&fixture(), "nope").is_none());
        assert!(locate_quoted_key(&fixture(), "").is_none());
    }

    #[test]
    fn render_unknown_field_produces_caret() {
        let src = fixture();
        let mut buf = String::new();
        render_warning(
            &mut buf,
            &src,
            "fixture.op",
            &LoadWarning::UnknownField {
                path: "$".into(),
                field: "mysteryField".into(),
            },
            Style::plain(),
        );
        assert!(buf.starts_with("warning: unknown field `mysteryField`"));
        assert!(buf.contains("fixture.op:4:4"));
        assert!(buf.contains("\"mysteryField\": 42"));
        assert!(buf.contains("^^^^^^^^^^^^"));
    }

    #[test]
    fn render_without_span_falls_back_to_arrow_anchor() {
        let mut buf = String::new();
        render_warning(
            &mut buf,
            "{}",
            "tiny.op",
            &LoadWarning::UnknownField {
                path: "$".into(),
                field: "absent".into(),
            },
            Style::plain(),
        );
        assert!(buf.starts_with("warning: unknown field `absent`"));
        assert!(buf.contains(" --> tiny.op"));
        assert!(!buf.contains('^'));
    }

    #[test]
    fn render_invalid_expression_locates_snippet() {
        let src = "  bind: ${1 + }\n".to_owned();
        let mut buf = String::new();
        render_warning(
            &mut buf,
            &src,
            "expr.op",
            &LoadWarning::InvalidExpression {
                path: "$.bind".into(),
                expr: "${1 + }".into(),
                reason: "trailing operator".into(),
            },
            Style::plain(),
        );
        assert!(buf.contains("invalid expression at `$.bind`"));
        assert!(buf.contains("expr.op:1:9"));
        assert!(buf.contains("^^^^^^^"));
    }

    #[test]
    fn color_style_emits_ansi_escapes() {
        let src = fixture();
        let mut buf = String::new();
        render_warning(
            &mut buf,
            &src,
            "color.op",
            &LoadWarning::UnknownField {
                path: "$".into(),
                field: "mysteryField".into(),
            },
            Style { color: true },
        );
        assert!(buf.contains("\x1b[33m"), "expected yellow on warning label");
        assert!(buf.contains("\x1b[0m"), "expected ANSI reset");
    }
}
