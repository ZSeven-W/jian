//! Single-field text-editing state machine.
//!
//! Owns the draft string, caret byte offset, selection, select-all flag,
//! IME composition, and the caret-blink anchor for one editable field.
//! Hosts feed it keystrokes and byte offsets resolved by widget hit-testing;
//! paint code reads `caret_visible(now)` and `highlight_range()`.
//! This module is pure logic: no fonts, no pixels, no platform APIs.

use crate::anim;

/// Caret half-period: 500 ms on, 500 ms off.
pub const CARET_BLINK_PERIOD_MS: u64 = 500;

/// Byte-offset selection. `anchor` is where the selection started;
/// `focus` is the moving end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub focus: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    pub fn ordered(self) -> (usize, usize) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }
}

/// In-flight IME preedit. `cursor` is a byte offset inside `text`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composition {
    pub text: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextInputState {
    text: String,
    selection: Selection,
    select_all: bool,
    blink_anchor_ms: u64,
    composition: Option<Composition>,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            text: String::new(),
            selection: Selection::caret(0),
            select_all: false,
            blink_anchor_ms: 0,
            composition: None,
        }
    }
}

impl TextInputState {
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let caret = text.len();
        Self {
            text,
            selection: Selection::caret(caret),
            ..Default::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.selection.focus
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn is_select_all(&self) -> bool {
        self.select_all
    }

    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    pub fn highlight_range(&self) -> Option<(usize, usize)> {
        if self.select_all && !self.text.is_empty() {
            return Some((0, self.text.len()));
        }
        let (start, end) = self.selection.ordered();
        (start != end).then_some((start, end))
    }

    pub fn touch(&mut self, now_ms: u64) {
        self.blink_anchor_ms = now_ms;
    }

    pub fn caret_visible(&self, now_ms: u64) -> bool {
        anim::blink_visible(now_ms, self.blink_anchor_ms, CARET_BLINK_PERIOD_MS)
    }

    pub fn next_blink_flip_ms(&self, now_ms: u64) -> u64 {
        anim::next_blink_flip_ms(now_ms, self.blink_anchor_ms, CARET_BLINK_PERIOD_MS)
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.selection = Selection::caret(self.text.len());
        self.select_all = false;
        self.composition = None;
    }

    pub fn select_all(&mut self) {
        self.select_all = true;
    }

    fn take_pending_range(&mut self) -> (usize, usize) {
        if self.select_all {
            self.select_all = false;
            (0, self.text.len())
        } else {
            self.selection.ordered()
        }
    }

    pub fn insert_str(&mut self, s: &str, now_ms: u64) {
        let (start, end) = self.take_pending_range();
        self.text.replace_range(start..end, s);
        self.selection = Selection::caret(start + s.len());
        self.touch(now_ms);
    }

    fn consume_pending(&mut self, now_ms: u64) -> bool {
        let (start, end) = self.take_pending_range();
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.selection = Selection::caret(start);
        self.touch(now_ms);
        true
    }

    pub fn backspace(&mut self, now_ms: u64) {
        if self.consume_pending(now_ms) {
            return;
        }
        let caret = self.selection.focus;
        if caret == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.text, caret - 1);
        self.text.replace_range(prev..caret, "");
        self.selection = Selection::caret(prev);
        self.touch(now_ms);
    }

    pub fn delete_forward(&mut self, now_ms: u64) {
        if self.consume_pending(now_ms) {
            return;
        }
        let caret = self.selection.focus;
        if caret >= self.text.len() {
            return;
        }
        let next = next_char_boundary(&self.text, caret + 1);
        self.text.replace_range(caret..next, "");
        self.touch(now_ms);
    }

    fn move_caret_to(&mut self, byte: usize, select: bool, now_ms: u64) {
        let byte = prev_char_boundary(&self.text, byte);
        self.selection = if select {
            Selection {
                anchor: self.selection.anchor,
                focus: byte,
            }
        } else {
            Selection::caret(byte)
        };
        self.select_all = false;
        self.touch(now_ms);
    }

    pub fn move_left(&mut self, select: bool, now_ms: u64) {
        let (start, _) = self.selection.ordered();
        let target = if !select && !self.selection.is_collapsed() {
            start
        } else if self.selection.focus == 0 {
            0
        } else {
            prev_char_boundary(&self.text, self.selection.focus - 1)
        };
        self.move_caret_to(target, select, now_ms);
    }

    pub fn move_right(&mut self, select: bool, now_ms: u64) {
        let (_, end) = self.selection.ordered();
        let target = if !select && !self.selection.is_collapsed() {
            end
        } else {
            next_char_boundary(&self.text, self.selection.focus + 1)
        };
        self.move_caret_to(target, select, now_ms);
    }

    pub fn move_home(&mut self, select: bool, now_ms: u64) {
        self.move_caret_to(0, select, now_ms);
    }

    pub fn move_end(&mut self, select: bool, now_ms: u64) {
        self.move_caret_to(self.text.len(), select, now_ms);
    }

    pub fn set_caret(&mut self, byte: usize, now_ms: u64) {
        self.move_caret_to(byte, false, now_ms);
    }

    pub fn drag_to(&mut self, byte: usize, now_ms: u64) {
        self.move_caret_to(byte, true, now_ms);
    }

    pub fn set_composition(&mut self, text: impl Into<String>, cursor: usize, now_ms: u64) {
        let _ = self.consume_pending(now_ms);
        self.composition = Some(Composition {
            text: text.into(),
            cursor,
        });
        self.touch(now_ms);
    }

    pub fn clear_composition(&mut self) {
        self.composition = None;
    }

    pub fn commit_composition(&mut self, now_ms: u64) {
        if let Some(c) = self.composition.take() {
            self.insert_str(&c.text, now_ms);
        }
    }
}

/// Largest char boundary less than or equal to `index`.
pub fn prev_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Smallest char boundary greater than or equal to `index`.
pub fn next_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_respect_cjk_boundaries() {
        let mut s = TextInputState::with_text("设计");
        s.insert_str("a", 0);
        assert_eq!(s.text(), "设计a");
        s.backspace(10);
        s.backspace(20);
        assert_eq!(s.text(), "设");
        assert_eq!(s.caret(), "设".len());
    }

    #[test]
    fn select_all_then_type_replaces_everything() {
        let mut s = TextInputState::with_text("hello");
        s.select_all();
        s.insert_str("x", 0);
        assert_eq!(s.text(), "x");
        assert!(!s.is_select_all());
        assert_eq!(s.caret(), 1);
    }

    #[test]
    fn select_all_then_backspace_clears() {
        let mut s = TextInputState::with_text("hello");
        s.select_all();
        s.backspace(0);
        assert_eq!(s.text(), "");
    }

    #[test]
    fn selection_replace_on_insert() {
        let mut s = TextInputState::with_text("abcdef");
        s.set_caret(1, 0);
        s.drag_to(4, 0);
        assert_eq!(s.highlight_range(), Some((1, 4)));
        s.insert_str("X", 0);
        assert_eq!(s.text(), "aXef");
        assert_eq!(s.caret(), 2);
    }

    #[test]
    fn plain_arrow_collapses_selection_to_edge() {
        let mut s = TextInputState::with_text("abcd");
        s.set_caret(3, 0);
        s.drag_to(1, 0);
        s.move_left(false, 0);
        assert_eq!(s.caret(), 1);
        s.drag_to(3, 0);
        s.move_right(false, 0);
        assert_eq!(s.caret(), 3);
    }

    #[test]
    fn edits_reset_blink_phase() {
        let mut s = TextInputState::with_text("a");
        assert!(!s.caret_visible(750));
        s.insert_str("b", 750);
        assert!(s.caret_visible(750));
        assert_eq!(s.next_blink_flip_ms(750), 1250);
    }

    #[test]
    fn composition_commits_at_caret() {
        let mut s = TextInputState::with_text("ab");
        s.set_caret(1, 0);
        s.set_composition("中文", "中文".len(), 0);
        assert!(s.composition().is_some());
        s.commit_composition(0);
        assert_eq!(s.text(), "a中文b");
        assert!(s.composition().is_none());
    }

    #[test]
    fn boundary_helpers_clamp() {
        assert_eq!(prev_char_boundary("设", 1), 0);
        assert_eq!(prev_char_boundary("设", 99), "设".len());
        assert_eq!(next_char_boundary("设", 1), "设".len());
        assert_eq!(next_char_boundary("", 5), 0);
    }
}
