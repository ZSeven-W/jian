//! IME composition events. Mirrors the W3C `CompositionEvent` triple
//! (compositionstart / compositionupdate / compositionend).
//!
//! **Index semantics (Step 1b §2.4 lock-in):**
//!
//! - `text`: UTF-8 `String` (Rust native). The host translator is
//!   responsible for converting the JS UTF-16 `CompositionEvent.data`
//!   into UTF-8 before constructing this event; once it reaches
//!   widget code the encoding is unambiguous.
//! - `selection`: UTF-8 byte offsets within `text` for the IME-
//!   highlighted preedit segment, when the browser exposes one.
//!   `None` = no selection / browser does not surface a target range.
//!
//! The conversion contract for shell-web is documented in the
//! Step 1b implementation plan §3.5: `event/ime.rs`'s
//! `utf16_selection_to_utf8` walks the UTF-16 prefix with
//! `char::len_utf16()` to map UTF-16 code-unit ranges to UTF-8 byte
//! ranges.

use core::ops::Range;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImeEvent {
    pub kind: ImeKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImeKind {
    /// Composition started. `text` is empty.
    CompositionStart,
    /// Composition in progress. `text` holds the in-flight preedit.
    /// `selection` covers the IME-highlighted segment in UTF-8 bytes.
    CompositionUpdate { selection: Option<Range<usize>> },
    /// Composition committed. `text` holds the final commit string.
    CompositionEnd,
}
