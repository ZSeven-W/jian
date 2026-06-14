//! CJK-aware greedy line breaking for multi-line text widgets.
//!
//! Pure logic (no fonts / pixels): callers pass a column budget where a
//! wide (East-Asian) glyph counts as 2 columns and everything else as 1
//! — the same width model the static caret approximation uses. Mirrors
//! `pen-renderer/paint-utils.ts::wrapLine`:
//! - wide codepoints break between any two characters,
//! - Latin runs break only at word (whitespace) boundaries,
//! - explicit `\n` is preserved, including empty lines.

/// East-Asian "wide" codepoint ranges (CJK, kana, Hangul, full-width).
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK radicals, Kangxi, CJK symbols/punct
        | 0x3041..=0x33FF // Hiragana, Katakana, CJK symbols
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compat ideographs
        | 0xFE30..=0xFE4F // CJK compat forms
        | 0xFF00..=0xFF60 // full-width forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD) // CJK Ext B+
}

/// Column width of a character (wide = 2, otherwise 1).
pub fn char_cols(c: char) -> usize {
    if is_wide(c) {
        2
    } else {
        1
    }
}

/// Greedy-wrap `text` to `max_cols` columns per line. Always returns at
/// least one (possibly empty) line; preserves blank lines from `\n`.
pub fn wrap_lines(text: &str, max_cols: usize) -> Vec<String> {
    let max_cols = max_cols.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        wrap_segment(raw, max_cols, &mut out);
    }
    out
}

fn wrap_segment(seg: &str, max_cols: usize, out: &mut Vec<String>) {
    let mut line = String::new();
    let mut cols = 0usize;
    for tok in tokenize(seg) {
        let tw: usize = tok.chars().map(char_cols).sum();
        let is_space = tok.chars().all(|c| c == ' ' || c == '\t');
        if cols + tw > max_cols && cols > 0 {
            // Token doesn't fit on the current line. Leading spaces at a
            // wrap point are dropped (don't start a line with them).
            out.push(std::mem::take(&mut line));
            cols = 0;
            if is_space {
                continue;
            }
        }
        if tw > max_cols {
            // A single token wider than the whole line: hard-break it by
            // character (covers a long CJK run or an over-long word).
            for c in tok.chars() {
                let cw = char_cols(c);
                if cols + cw > max_cols && cols > 0 {
                    out.push(std::mem::take(&mut line));
                    cols = 0;
                }
                line.push(c);
                cols += cw;
            }
        } else {
            line.push_str(&tok);
            cols += tw;
        }
    }
    out.push(line);
}

/// Split a `\n`-free segment into breakable tokens: each wide glyph is
/// its own token; Latin runs and whitespace runs are grouped so words
/// don't split mid-run.
fn tokenize(seg: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut buf_space = false;
    for c in seg.chars() {
        if is_wide(c) {
            if !buf.is_empty() {
                tokens.push(std::mem::take(&mut buf));
            }
            tokens.push(c.to_string());
            continue;
        }
        let is_space = c == ' ' || c == '\t';
        if !buf.is_empty() && is_space != buf_space {
            tokens.push(std::mem::take(&mut buf));
        }
        buf_space = is_space;
        buf.push(c);
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_blank_lines_and_explicit_breaks() {
        assert_eq!(wrap_lines("a\n\nb", 80), vec!["a", "", "b"]);
    }

    #[test]
    fn latin_breaks_on_word_boundaries() {
        // "hello world foo" at 11 cols → "hello world" / "foo".
        let lines = wrap_lines("hello world foo", 11);
        assert_eq!(lines, vec!["hello world", "foo"]);
    }

    #[test]
    fn cjk_breaks_per_character_by_columns() {
        // 4 wide chars = 8 cols; budget 4 cols = 2 chars/line.
        let lines = wrap_lines("设计工具", 4);
        assert_eq!(lines, vec!["设计", "工具"]);
    }

    #[test]
    fn over_long_word_hard_breaks() {
        let lines = wrap_lines("abcdefgh", 3);
        assert_eq!(lines, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn single_line_when_it_fits() {
        assert_eq!(wrap_lines("short", 80), vec!["short"]);
    }
}
