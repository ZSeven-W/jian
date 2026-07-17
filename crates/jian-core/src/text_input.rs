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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    /// Selection inside `text`, expressed as UTF-8 byte offsets.
    pub selection: Selection,
    pub region: Option<(usize, usize)>,
    /// Explicit selection placed OUTSIDE the preedit while composing (the
    /// `jian.h` escape hatch / Android far-cursor semantics): absolute byte
    /// offsets in the EFFECTIVE text. Cleared whenever the platform re-places
    /// the composing selection; carried into the durable selection on a local
    /// commit; discarded by cancel and by the explicit-commit caret formula.
    pub detached: Option<Selection>,
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

    /// Text exposed to a platform input method, with preedit replacing its
    /// durable composing region.
    pub fn effective_text(&self) -> String {
        let Some(composition) = self.composition.as_ref() else {
            return self.text.clone();
        };
        let (start, end) = composition
            .region
            .unwrap_or_else(|| (self.caret(), self.caret()));
        let mut text = String::with_capacity(
            self.text
                .len()
                .saturating_sub(end.saturating_sub(start))
                .saturating_add(composition.text.len()),
        );
        text.push_str(&self.text[..start]);
        text.push_str(&composition.text);
        text.push_str(&self.text[end..]);
        text
    }

    /// Selection in the effective (durable + preedit) UTF-8 text.
    pub fn effective_selection(&self) -> Selection {
        let Some(composition) = self.composition.as_ref() else {
            return self.selection;
        };
        if let Some(detached) = composition.detached {
            return detached;
        }
        let start = composition
            .region
            .map(|region| region.0)
            .unwrap_or(self.caret());
        Selection {
            anchor: start.saturating_add(composition.selection.anchor),
            focus: start.saturating_add(composition.selection.focus),
        }
    }

    pub fn effective_composing_range(&self) -> Option<(usize, usize)> {
        let composition = self.composition.as_ref()?;
        let start = composition
            .region
            .map(|region| region.0)
            .unwrap_or(self.caret());
        Some((start, start.saturating_add(composition.text.len())))
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
        self.selection = Selection {
            anchor: 0,
            focus: self.text.len(),
        };
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
        self.set_composing_text(text, cursor, cursor, now_ms);
    }

    pub fn set_composing_text(
        &mut self,
        text: impl Into<String>,
        selection_start: usize,
        selection_end: usize,
        now_ms: u64,
    ) {
        let text = text.into();
        let region = self.composition.as_ref().and_then(|value| value.region);
        let region = region.unwrap_or_else(|| {
            let (start, end) = self.take_pending_range();
            if start != end {
                self.text.replace_range(start..end, "");
            }
            self.selection = Selection::caret(start);
            (start, start)
        });
        let start = prev_char_boundary(&text, selection_start.min(selection_end));
        let end = prev_char_boundary(&text, selection_start.max(selection_end));
        self.composition = Some(Composition {
            text,
            cursor: end,
            selection: Selection {
                anchor: start,
                focus: end,
            },
            region: Some(region),
            detached: None,
        });
        self.touch(now_ms);
    }

    pub fn clear_composition(&mut self) {
        self.composition = None;
    }

    /// Cancel preedit locally. Its durable region is deleted and the caret is
    /// left at the composing start; a selection consumed at start is not
    /// restored.
    pub fn cancel_composition(&mut self, now_ms: u64) -> bool {
        let Some(composition) = self.composition.take() else {
            return false;
        };
        let (start, end) = composition
            .region
            .unwrap_or_else(|| (self.caret(), self.caret()));
        self.text.replace_range(start..end, "");
        self.selection = Selection::caret(start);
        self.select_all = false;
        self.touch(now_ms);
        true
    }

    pub fn reset_transient(&mut self) {
        self.selection = Selection::caret(self.text.len());
        self.select_all = false;
        self.composition = None;
    }

    pub fn set_composing_region(&mut self, start: usize, end: usize) {
        let (raw_start, raw_end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let start = prev_char_boundary(&self.text, raw_start);
        let end = next_char_boundary(&self.text, raw_end);
        if let Some(composition) = &mut self.composition {
            composition.region = Some((start, end));
            composition.detached = None;
        } else {
            let text = self.text[start..end].to_owned();
            self.composition = Some(Composition {
                cursor: text.len(),
                selection: Selection::caret(text.len()),
                text,
                region: Some((start, end)),
                detached: None,
            });
        }
        self.selection = Selection::caret(start);
    }

    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str, now_ms: u64) {
        let (raw_start, raw_end) = (start.min(end), start.max(end));
        let start = prev_char_boundary(&self.text, raw_start);
        let end = prev_char_boundary(&self.text, raw_end);
        self.text.replace_range(start..end, replacement);
        self.selection = Selection::caret(start + replacement.len());
        self.select_all = false;
        self.touch(now_ms);
    }

    pub fn commit_composition(&mut self, now_ms: u64) {
        if let Some(c) = self.composition.take() {
            let detached = c.detached;
            if let Some((start, end)) = c.region {
                self.replace_range(start, end, &c.text, now_ms);
            } else {
                self.insert_str(&c.text, now_ms);
            }
            // Effective coordinates ARE the post-splice durable coordinates,
            // so a detached selection carries over unchanged.
            if let Some(selection) = detached {
                let anchor = prev_char_boundary(&self.text, selection.anchor.min(self.text.len()));
                let focus = prev_char_boundary(&self.text, selection.focus.min(self.text.len()));
                self.selection = Selection { anchor, focus };
            }
        }
    }

    /// Replace the active composition (or pending durable selection) with
    /// committed text and return the durable insertion start.
    pub fn commit_text(&mut self, text: &str, now_ms: u64) -> usize {
        if let Some(composition) = self.composition.take() {
            let (start, end) = composition
                .region
                .unwrap_or_else(|| (self.caret(), self.caret()));
            self.replace_range(start, end, text, now_ms);
            return start;
        }
        let (start, end) = self.take_pending_range();
        self.replace_range(start, end, text, now_ms);
        start
    }

    /// Set a selection expressed in the effective text. A range inside the
    /// active preedit remains a composing-relative selection; a range outside
    /// DETACHES the selection while keeping the preedit alive (the `jian.h`
    /// escape hatch — Android far-cursor `setComposingText` semantics).
    pub fn set_effective_selection(&mut self, start: usize, end: usize, now_ms: u64) {
        if self.composition.is_some() {
            let (base, limit) = {
                let composition = self.composition.as_ref().expect("checked above");
                let base = composition.region.map(|region| region.0).unwrap_or(0);
                (base, base.saturating_add(composition.text.len()))
            };
            if start >= base && end >= base && start <= limit && end <= limit {
                let composition = self.composition.as_mut().expect("checked above");
                let start = prev_char_boundary(&composition.text, start - base);
                let end = prev_char_boundary(&composition.text, end - base);
                composition.selection = Selection {
                    anchor: start,
                    focus: end,
                };
                composition.cursor = end;
                // Returning inside the preedit re-attaches the selection.
                composition.detached = None;
                self.touch(now_ms);
                return;
            }
            let effective = self.effective_text();
            let anchor = prev_char_boundary(&effective, start.min(end).min(effective.len()));
            let focus = prev_char_boundary(&effective, start.max(end).min(effective.len()));
            let composition = self.composition.as_mut().expect("checked above");
            composition.detached = Some(Selection { anchor, focus });
            self.select_all = false;
            self.touch(now_ms);
            return;
        }
        let start = prev_char_boundary(&self.text, start);
        let end = prev_char_boundary(&self.text, end);
        self.selection = Selection {
            anchor: start,
            focus: end,
        };
        self.select_all = false;
        self.touch(now_ms);
    }

    /// Replace a range expressed in effective-text byte offsets.
    pub fn replace_effective_range(
        &mut self,
        start: usize,
        end: usize,
        replacement: &str,
        now_ms: u64,
    ) {
        if self.composition.is_some() {
            self.commit_composition(now_ms);
        }
        self.replace_range(start, end, replacement, now_ms);
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
    fn select_all_then_plain_arrows_collapse_to_edges() {
        let mut s = TextInputState::with_text("hello");
        s.select_all();
        s.move_left(false, 0);
        assert_eq!(s.caret(), 0);
        assert_eq!(s.highlight_range(), None);

        s.select_all();
        s.move_right(false, 0);
        assert_eq!(s.caret(), "hello".len());
        assert_eq!(s.highlight_range(), None);
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
#[test]
fn composing_region_commit_replaces_durable_range() {
    let mut state = TextInputState::with_text("abcdef");
    state.set_composition("XY", 2, 0);
    state.set_composing_region(2, 5);
    state.commit_composition(1);
    assert_eq!(state.text(), "abXYf");
    assert_eq!(state.caret(), 4);
}

#[test]
fn replace_range_clamps_to_utf8_boundaries() {
    let mut state = TextInputState::with_text("a设计z");
    state.replace_range(2, 6, "X", 0);
    assert_eq!(state.text(), "aX计z");
    state.replace_range(1, 2, "", 1);
    assert_eq!(state.text(), "a计z");
}

#[test]
fn composing_region_swaps_reversed_offsets_and_forward_snaps_end() {
    let mut state = TextInputState::with_text("a设计z");
    state.set_composition("X", 1, 0);
    state.set_composing_region(6, 2);
    assert_eq!(state.composition().unwrap().region, Some((1, 7)));
    state.commit_composition(1);
    assert_eq!(state.text(), "aXz");
}

#[cfg(test)]
mod detached_selection_tests {
    use super::*;

    // ---- Detached-selection matrix (jian.h escape hatch, M4 plan Task 3b) ----

    fn composing_state() -> TextInputState {
        let mut s = TextInputState::with_text("hello ");
        // Compose "wo" at the end: region (6, 6), preedit "wo".
        s.set_composing_text("wo", 2, 2, 1);
        s
    }

    #[test]
    fn outside_selection_detaches_without_committing() {
        let mut s = composing_state();
        s.set_effective_selection(0, 0, 2);
        let c = s.composition().unwrap();
        assert_eq!(c.text, "wo");
        assert_eq!(
            c.detached,
            Some(Selection {
                anchor: 0,
                focus: 0
            })
        );
        assert_eq!(
            s.effective_selection(),
            Selection {
                anchor: 0,
                focus: 0
            }
        );
        assert_eq!(s.effective_text(), "hello wo");
    }

    #[test]
    fn returning_inside_reattaches_the_composing_selection() {
        let mut s = composing_state();
        s.set_effective_selection(0, 0, 2);
        s.set_effective_selection(7, 7, 3); // back inside the preedit
        let c = s.composition().unwrap();
        assert_eq!(c.detached, None);
        assert_eq!(
            c.selection,
            Selection {
                anchor: 1,
                focus: 1
            }
        );
        assert_eq!(
            s.effective_selection(),
            Selection {
                anchor: 7,
                focus: 7
            }
        );
    }

    #[test]
    fn composing_update_while_detached_reattaches() {
        let mut s = composing_state();
        s.set_effective_selection(0, 0, 2);
        // The platform re-places the cursor with the new composing text.
        s.set_composing_text("wor", 3, 3, 3);
        let c = s.composition().unwrap();
        assert_eq!(c.detached, None);
        assert_eq!(
            s.effective_selection(),
            Selection {
                anchor: 9,
                focus: 9
            }
        );
        assert_eq!(s.effective_text(), "hello wor");
    }

    #[test]
    fn local_commit_while_detached_preserves_the_selection() {
        let mut s = composing_state();
        s.set_effective_selection(2, 4, 2);
        // The old behavior committed here — this assert is what makes the
        // test red against it.
        assert!(s.composition().is_some());
        s.commit_composition(3);
        assert_eq!(s.text(), "hello wo");
        assert_eq!(
            s.selection(),
            Selection {
                anchor: 2,
                focus: 4
            }
        );
        assert!(s.composition().is_none());
    }

    #[test]
    fn cancel_while_detached_removes_the_preedit_with_caret_at_start() {
        let mut s = composing_state();
        s.set_effective_selection(0, 0, 2);
        assert!(s.cancel_composition(3));
        assert_eq!(s.text(), "hello ");
        // ABI local-cancel rule: caret at the composition start.
        assert_eq!(s.caret(), 6);
        assert!(s.composition().is_none());
    }
}
