use crate::error::{read_utf8, FfiError, FfiResult, STRING_CAP};
use crate::lifecycle::{call_engine, Lifecycle};
use crate::{JianEngine, JianStatus};
use jian_core::render::{normalize_utf16_offset, utf16_to_byte_offset};
use jian_core::runtime::{EditableInputKind, EditableTextSnapshot, ImeConfirmOutcome, ImeSnapshot};
use std::ffi::c_void;
use std::mem::size_of;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianInputKind {
    Text = 0,
    Number = 1,
    Secure = 2,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianReturnKeyHint {
    Default = 0,
    Done = 1,
    Go = 2,
    Next = 3,
    Search = 4,
    Send = 5,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianImeControlOp {
    Commit = 0,
    Cancel = 1,
    Dismiss = 2,
}

impl TryFrom<i32> for JianImeControlOp {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Commit),
            1 => Ok(Self::Cancel),
            2 => Ok(Self::Dismiss),
            _ => Err(()),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JianFieldInfo {
    pub size: usize,
    pub input_kind: JianInputKind,
    pub return_key_hint: JianReturnKeyHint,
}

pub type JianInputFocusChanged =
    unsafe extern "C" fn(user_data: *mut c_void, focused: bool, info: *const JianFieldInfo);
pub type JianTextStateChanged = unsafe extern "C" fn(user_data: *mut c_void);
pub type JianImeControl = unsafe extern "C" fn(user_data: *mut c_void, op: i32, request_id: u64);

pub(crate) struct ImeState {
    batch_depth: u8,
    deferred_text_change: bool,
    explicit_text_change: bool,
    pub(crate) state_buffer: String,
    #[cfg(feature = "textlayout")]
    pub(crate) geometry_snapshot: Option<EditableTextSnapshot>,
}

impl ImeState {
    pub(crate) fn new() -> Self {
        Self {
            batch_depth: 0,
            deferred_text_change: false,
            explicit_text_change: false,
            state_buffer: String::new(),
            #[cfg(feature = "textlayout")]
            geometry_snapshot: None,
        }
    }
}

pub(crate) struct ImeObservation {
    focused: Option<EditableTextSnapshot>,
    composition: Option<ImeSnapshot>,
}

impl Lifecycle {
    pub(crate) fn begin_ime_observation(&mut self) -> ImeObservation {
        self.ime.explicit_text_change = false;
        ImeObservation {
            composition: self.runtime.focused_ime_snapshot(),
            focused: self.runtime.focused_editable_snapshot(),
        }
    }

    pub(crate) fn finish_ime_observation(&mut self, before: ImeObservation) {
        let mut after = self.runtime.focused_editable_snapshot();
        let before_key = focus_key(before.focused.as_ref());
        let after_key = focus_key(after.as_ref());
        let focus_changed = before_key != after_key;
        let mut text_changed = self.ime.explicit_text_change
            || (!focus_changed
                && text_signature(before.focused.as_ref()) != text_signature(after.as_ref()));

        if focus_changed {
            if let Some(snapshot) = before.composition.as_ref() {
                if let Some(callback) = self.callbacks.ime_control {
                    let request_id = self.runtime.begin_ime_handshake(snapshot.clone());
                    unsafe {
                        callback(
                            self.callbacks.user_data,
                            JianImeControlOp::Dismiss as i32,
                            request_id,
                        )
                    };
                } else if self.runtime.cancel_ime_snapshot_locally(snapshot) {
                    text_changed = true;
                }
            }
            after = self.runtime.focused_editable_snapshot();
            if self.ime.batch_depth != 0 {
                self.ime.batch_depth = 0;
                text_changed = true;
                self.ime.deferred_text_change = false;
            }
            self.emit_focus_changed(after.as_ref());
        }

        self.refresh_text_geometry();
        if text_changed {
            self.emit_or_defer_text_changed();
        }
    }

    pub(crate) fn note_explicit_text_change(&mut self) {
        self.ime.explicit_text_change = true;
        self.runtime.mark_dirty();
    }

    pub(crate) fn text_batch_begin(&mut self) -> FfiResult<()> {
        if self.runtime.focused_editable_snapshot().is_none() {
            return Err(no_focus());
        }
        if self.ime.batch_depth >= 64 {
            return Err(FfiError::invalid("text batch depth exceeds 64"));
        }
        self.ime.batch_depth += 1;
        Ok(())
    }

    pub(crate) fn text_batch_end(&mut self) -> FfiResult<()> {
        if self.ime.batch_depth == 0 {
            return Err(FfiError::invalid("text batch end has no matching begin"));
        }
        self.ime.batch_depth -= 1;
        if self.ime.batch_depth == 0 && self.ime.deferred_text_change {
            self.ime.deferred_text_change = false;
            self.emit_text_changed();
        }
        Ok(())
    }

    fn emit_or_defer_text_changed(&mut self) {
        if self.ime.batch_depth != 0 {
            self.ime.deferred_text_change = true;
        } else {
            self.emit_text_changed();
        }
    }

    fn emit_text_changed(&self) {
        if let Some(callback) = self.callbacks.text_state_changed {
            unsafe { callback(self.callbacks.user_data) };
        }
    }

    fn emit_focus_changed(&self, focused: Option<&EditableTextSnapshot>) {
        let Some(callback) = self.callbacks.input_focus_changed else {
            return;
        };
        let info = focused.map(|snapshot| JianFieldInfo {
            size: size_of::<JianFieldInfo>(),
            input_kind: match snapshot.input_kind {
                EditableInputKind::Text => JianInputKind::Text,
                EditableInputKind::Number => JianInputKind::Number,
                EditableInputKind::Secure => JianInputKind::Secure,
            },
            return_key_hint: return_key_hint(&snapshot.return_key_hint),
        });
        unsafe {
            callback(
                self.callbacks.user_data,
                info.is_some(),
                info.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
            )
        };
    }
}

fn focus_key(snapshot: Option<&EditableTextSnapshot>) -> Option<(&str, &str)> {
    snapshot.map(|value| (value.page_id.as_str(), value.field_id.as_str()))
}

type TextSignature<'a> = (
    &'a str,
    jian_core::text_input::Selection,
    Option<(usize, usize)>,
);

fn text_signature(snapshot: Option<&EditableTextSnapshot>) -> Option<TextSignature<'_>> {
    snapshot.map(|value| (value.text.as_str(), value.selection, value.composing_range))
}

fn return_key_hint(value: &str) -> JianReturnKeyHint {
    match value.trim().to_ascii_lowercase().as_str() {
        "done" => JianReturnKeyHint::Done,
        "go" => JianReturnKeyHint::Go,
        "next" => JianReturnKeyHint::Next,
        "search" => JianReturnKeyHint::Search,
        "send" => JianReturnKeyHint::Send,
        _ => JianReturnKeyHint::Default,
    }
}

pub(crate) fn no_focus() -> FfiError {
    FfiError::new(JianStatus::NoFocus, "no editable field is focused")
}

fn normalized_bytes(text: &str, start: u32, end: u32) -> (usize, usize) {
    let start = normalize_utf16_offset(text, start);
    let end = normalize_utf16_offset(text, end);
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    (
        utf16_to_byte_offset(text, start),
        utf16_to_byte_offset(text, end),
    )
}

unsafe fn ffi_text(pointer: *const u8, length: usize, label: &str) -> FfiResult<String> {
    unsafe { read_utf8(pointer, length, STRING_CAP, label) }
}

/// Insert or replace the current platform selection.
///
/// # Safety
///
/// `engine` must be live and `text` must cover `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_text_insert(
    engine: *mut JianEngine,
    text: *const u8,
    length: usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let text = ffi_text(text, length, "insert text")?;
            if !lifecycle.runtime.edit_insert(&text) {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Replace an effective-text range.
///
/// # Safety
///
/// `engine` must be live and `text` must cover `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_text_replace_range(
    engine: *mut JianEngine,
    start: u32,
    end: u32,
    text: *const u8,
    length: usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let replacement = ffi_text(text, length, "replacement text")?;
            let snapshot = lifecycle
                .runtime
                .focused_editable_snapshot()
                .ok_or_else(no_focus)?;
            let (start, end) = normalized_bytes(&snapshot.text, start, end);
            if !lifecycle
                .runtime
                .edit_replace_range(start, end, &replacement)
            {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Set the effective platform selection.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_text_set_selection(
    engine: *mut JianEngine,
    start: u32,
    end: u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let snapshot = lifecycle
                .runtime
                .focused_editable_snapshot()
                .ok_or_else(no_focus)?;
            let (start, end) = normalized_bytes(&snapshot.text, start, end);
            if !lifecycle.runtime.edit_set_selection(start, end) {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Select durable text as the composing region.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_ime_set_composing_region(
    engine: *mut JianEngine,
    start: u32,
    end: u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let snapshot = lifecycle
                .runtime
                .focused_editable_snapshot()
                .ok_or_else(no_focus)?;
            let (start, end) = normalized_bytes(&snapshot.text, start, end);
            if !lifecycle.runtime.edit_set_composing_region(start, end) {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Set platform preedit and its composing-relative selection.
///
/// # Safety
///
/// `engine` must be live and `text` must cover `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_ime_set_composing_text(
    engine: *mut JianEngine,
    text: *const u8,
    length: usize,
    selection_start: u32,
    selection_end: u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let text = ffi_text(text, length, "composing text")?;
            let (start, end) = normalized_bytes(&text, selection_start, selection_end);
            if !lifecycle.runtime.edit_set_composing_text(&text, start, end) {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Commit platform text, optionally confirming an engine request.
///
/// # Safety
///
/// `engine` must be live and `text` must cover `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_ime_commit(
    engine: *mut JianEngine,
    text: *const u8,
    length: usize,
    new_cursor_position: i32,
    request_id: u64,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let text = ffi_text(text, length, "commit text")?;
            if request_id != 0 {
                if lifecycle.runtime.confirm_ime_commit_with_cursor(
                    request_id,
                    &text,
                    new_cursor_position,
                ) == ImeConfirmOutcome::Applied
                {
                    lifecycle.note_explicit_text_change();
                }
                return Ok(());
            }
            if !lifecycle.runtime.edit_commit(&text, new_cursor_position) {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Cancel local preedit or confirm cancellation of an engine request.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_ime_cancel(engine: *mut JianEngine, request_id: u64) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if request_id != 0 {
                if lifecycle.runtime.confirm_ime_cancel(request_id) == ImeConfirmOutcome::Applied {
                    lifecycle.note_explicit_text_change();
                }
                return Ok(());
            }
            if !lifecycle.runtime.edit_cancel() {
                return Err(no_focus());
            }
            lifecycle.runtime.mark_dirty();
            Ok(())
        })
    }
}

/// Begin a batch of platform text edits.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_text_batch_begin(engine: *mut JianEngine) -> JianStatus {
    unsafe { call_engine(engine, Lifecycle::text_batch_begin) }
}

/// End a batch of platform text edits.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_text_batch_end(engine: *mut JianEngine) -> JianStatus {
    unsafe { call_engine(engine, Lifecycle::text_batch_end) }
}
