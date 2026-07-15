use crate::error::{write_bytes, FfiError, FfiResult};
use crate::ime::no_focus;
use crate::lifecycle::{call_engine, Lifecycle};
use crate::{JianEngine, JianRect, JianStatus};
#[cfg(feature = "textlayout")]
use jian_core::render::FieldKey;
use jian_core::render::{
    byte_to_utf16_offset, normalize_utf16_offset, utf16_len, utf16_to_byte_offset, Granularity,
    TextGeometryError, WritingDirection,
};
use jian_core::runtime::EditableTextSnapshot;
use std::mem::size_of;
use std::ptr;

const SURROUNDING_WINDOW_UTF16: u32 = 4096;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianWritingDirection {
    LeftToRight = 0,
    RightToLeft = 1,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianTextGranularity {
    Character = 0,
    Word = 1,
}

impl TryFrom<i32> for JianTextGranularity {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Character),
            1 => Ok(Self::Word),
            _ => Err(()),
        }
    }
}

/// Borrowed surrounding-text window. `text_ptr` remains valid until the next
/// call on this engine; all offsets are absolute UTF-16 code-unit offsets.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JianTextState {
    pub size: usize,
    pub text_ptr: *const u8,
    pub text_len: usize,
    pub window_start: u32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub has_composing: bool,
    pub composing_start: u32,
    pub composing_end: u32,
}

impl Default for JianTextState {
    fn default() -> Self {
        Self {
            size: size_of::<Self>(),
            text_ptr: ptr::null(),
            text_len: 0,
            window_start: 0,
            selection_start: 0,
            selection_end: 0,
            has_composing: false,
            composing_start: 0,
            composing_end: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JianTextRect {
    pub rect: JianRect,
    pub writing_direction: JianWritingDirection,
}

impl Lifecycle {
    pub(crate) fn refresh_text_geometry(&mut self) {
        #[cfg(feature = "textlayout")]
        {
            let snapshot = self.runtime.focused_editable_snapshot();
            if self.ime.geometry_snapshot == snapshot {
                return;
            }
            self.text_geometry.clear();
            if let Some(snapshot) = snapshot.as_ref() {
                let mut field = jian_skia::SkiaTextField::new(&snapshot.text, snapshot.bounds);
                field.text_origin = snapshot.text_origin;
                field.max_width = snapshot.max_width;
                field.font_family = snapshot.font_family.clone();
                field.font_size = snapshot.font_size;
                field.font_weight = snapshot.font_weight;
                field.line_height = if snapshot.multiline { 1.3 } else { 0.0 };
                self.text_geometry
                    .set_field(FieldKey::new(snapshot.field_id.as_str()), field);
            }
            self.ime.geometry_snapshot = snapshot;
        }
    }

    fn text_snapshot(&mut self) -> FfiResult<EditableTextSnapshot> {
        self.runtime
            .focused_editable_snapshot()
            .ok_or_else(no_focus)
    }

    fn geometry_snapshot(&mut self) -> FfiResult<EditableTextSnapshot> {
        if self.suspended {
            return Err(FfiError::new(
                JianStatus::NotReady,
                "text geometry is unavailable while suspended",
            ));
        }
        self.text_snapshot()
    }
}

fn rect(value: jian_core::geometry::Rect) -> JianRect {
    JianRect {
        x: value.origin.x,
        y: value.origin.y,
        width: value.size.width,
        height: value.size.height,
    }
}

fn geometry_error(error: TextGeometryError) -> FfiError {
    match error {
        TextGeometryError::NoFocus => no_focus(),
        TextGeometryError::NotReady => {
            FfiError::new(JianStatus::NotReady, "text geometry is not ready")
        }
    }
}

fn normalized_range(text: &str, start: u32, end: u32) -> (u32, u32) {
    let start = normalize_utf16_offset(text, start);
    let end = normalize_utf16_offset(text, end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

unsafe fn write_versioned_state(
    output: *mut JianTextState,
    value: &JianTextState,
) -> FfiResult<()> {
    if output.is_null() {
        return Err(FfiError::invalid("text-state output pointer is null"));
    }
    let caller_size = unsafe { output.cast::<usize>().read_unaligned() };
    if caller_size < size_of::<usize>() {
        return Err(FfiError::invalid("text-state size is below the minimum"));
    }
    if caller_size > size_of::<JianTextState>() {
        return Err(FfiError::invalid(
            "text-state size exceeds the known version",
        ));
    }
    unsafe {
        ptr::copy_nonoverlapping(
            ptr::from_ref(value).cast::<u8>(),
            output.cast::<u8>(),
            caller_size,
        )
    };
    Ok(())
}

unsafe fn validate_versioned_state(output: *mut JianTextState) -> FfiResult<()> {
    if output.is_null() {
        return Err(FfiError::invalid("text-state output pointer is null"));
    }
    let caller_size = unsafe { output.cast::<usize>().read_unaligned() };
    if caller_size < size_of::<usize>() {
        return Err(FfiError::invalid("text-state size is below the minimum"));
    }
    if caller_size > size_of::<JianTextState>() {
        return Err(FfiError::invalid(
            "text-state size exceeds the known version",
        ));
    }
    Ok(())
}

unsafe fn validate_output_buffer(
    buffer: *mut u8,
    length: usize,
    required: *mut usize,
) -> FfiResult<()> {
    if required.is_null() {
        return Err(FfiError::invalid("required-length pointer is null"));
    }
    if buffer.is_null() && length != 0 {
        return Err(FfiError::invalid(
            "output buffer is null with nonzero length",
        ));
    }
    if length > isize::MAX as usize {
        return Err(FfiError::invalid("output buffer length overflows"));
    }
    Ok(())
}

/// Return a borrowed surrounding-text window and absolute ranges.
///
/// # Safety
///
/// `engine` must be live and `output` must expose its declared writable size.
#[no_mangle]
pub unsafe extern "C" fn jian_text_get_state(
    engine: *mut JianEngine,
    output: *mut JianTextState,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            validate_versioned_state(output)?;
            let snapshot = lifecycle.text_snapshot()?;
            let selection = snapshot.selection.ordered();
            let selection_start = byte_to_utf16_offset(&snapshot.text, selection.0);
            let selection_end = byte_to_utf16_offset(&snapshot.text, selection.1);
            let length = utf16_len(&snapshot.text);
            let (window_start, window_end) = surrounding_window(
                &snapshot.text,
                selection_end,
                length,
                SURROUNDING_WINDOW_UTF16,
            );
            let start_byte = utf16_to_byte_offset(&snapshot.text, window_start);
            let end_byte = utf16_to_byte_offset(&snapshot.text, window_end);
            lifecycle.ime.state_buffer.clear();
            lifecycle
                .ime
                .state_buffer
                .push_str(&snapshot.text[start_byte..end_byte]);
            let composing = snapshot.composing_range.map(|range| {
                (
                    byte_to_utf16_offset(&snapshot.text, range.0),
                    byte_to_utf16_offset(&snapshot.text, range.1),
                )
            });
            let value = JianTextState {
                size: size_of::<JianTextState>(),
                text_ptr: lifecycle.ime.state_buffer.as_ptr(),
                text_len: lifecycle.ime.state_buffer.len(),
                window_start,
                selection_start,
                selection_end,
                has_composing: composing.is_some(),
                composing_start: composing.map_or(0, |range| range.0),
                composing_end: composing.map_or(0, |range| range.1),
            };
            write_versioned_state(output, &value)
        })
    }
}

fn surrounding_window(text: &str, focus: u32, length: u32, cap: u32) -> (u32, u32) {
    if length <= cap {
        return (0, length);
    }
    let start = normalize_utf16_offset(text, focus.saturating_sub(cap / 2));
    let mut end = normalize_utf16_offset(text, start.saturating_add(cap));
    let start = if end == length {
        normalize_utf16_offset(text, length.saturating_sub(cap))
    } else {
        start
    };
    end = normalize_utf16_offset(text, start.saturating_add(cap));
    (start, end)
}

/// Copy an arbitrary effective-text range into a caller buffer.
///
/// # Safety
///
/// `engine` must be live; output pointers must cover their declared sizes.
#[no_mangle]
pub unsafe extern "C" fn jian_text_get_range(
    engine: *mut JianEngine,
    start: u32,
    end: u32,
    buffer: *mut u8,
    length: usize,
    required: *mut usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            validate_output_buffer(buffer, length, required)?;
            let snapshot = lifecycle.text_snapshot()?;
            let (start, end) = normalized_range(&snapshot.text, start, end);
            let start = utf16_to_byte_offset(&snapshot.text, start);
            let end = utf16_to_byte_offset(&snapshot.text, end);
            write_bytes(
                &snapshot.text.as_bytes()[start..end],
                buffer,
                length,
                required,
            )
        })
    }
}

/// Return the caret rectangle for the current selection focus.
///
/// # Safety
///
/// `engine` must be live and `output` must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_text_caret_rect(
    engine: *mut JianEngine,
    output: *mut JianRect,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if output.is_null() {
                return Err(FfiError::invalid("caret-rect output pointer is null"));
            }
            let snapshot = lifecycle.geometry_snapshot()?;
            let offset = byte_to_utf16_offset(&snapshot.text, snapshot.selection.focus);
            output.write(rect(
                lifecycle
                    .runtime
                    .text_caret_rect(offset)
                    .map_err(geometry_error)?,
            ));
            Ok(())
        })
    }
}

/// Return the caret rectangle for an explicit UTF-16 offset.
///
/// # Safety
///
/// `engine` must be live and `output` must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_text_caret_rect_for_offset(
    engine: *mut JianEngine,
    offset: u32,
    output: *mut JianRect,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if output.is_null() {
                return Err(FfiError::invalid("caret-rect output pointer is null"));
            }
            let snapshot = lifecycle.geometry_snapshot()?;
            let offset = normalize_utf16_offset(&snapshot.text, offset);
            output.write(rect(
                lifecycle
                    .runtime
                    .text_caret_rect(offset)
                    .map_err(geometry_error)?,
            ));
            Ok(())
        })
    }
}

/// Return shaped selection rectangles and the total required count.
///
/// # Safety
///
/// `engine` must be live; `output` must cover `capacity` elements and `count`
/// must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_text_rects_for_range(
    engine: *mut JianEngine,
    start: u32,
    end: u32,
    output: *mut JianTextRect,
    capacity: usize,
    count: *mut usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if count.is_null() {
                return Err(FfiError::invalid("text-rect count pointer is null"));
            }
            if capacity != 0 && output.is_null() {
                return Err(FfiError::invalid(
                    "text-rect output is null with nonzero capacity",
                ));
            }
            let snapshot = lifecycle.geometry_snapshot()?;
            let (start, end) = normalized_range(&snapshot.text, start, end);
            let values = lifecycle
                .runtime
                .text_rects_for_range(start, end)
                .map_err(geometry_error)?;
            count.write(values.len());
            for (index, value) in values.iter().take(capacity).enumerate() {
                output.add(index).write(JianTextRect {
                    rect: rect(value.rect),
                    writing_direction: match value.writing_direction {
                        WritingDirection::LeftToRight => JianWritingDirection::LeftToRight,
                        WritingDirection::RightToLeft => JianWritingDirection::RightToLeft,
                    },
                });
            }
            Ok(())
        })
    }
}

/// Hit-test a surface-logical point to a UTF-16 text position.
///
/// # Safety
///
/// `engine` must be live and `output` must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_text_position_at_point(
    engine: *mut JianEngine,
    x: f32,
    y: f32,
    output: *mut u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if !x.is_finite() || !y.is_finite() {
                return Err(FfiError::invalid(
                    "text hit-test coordinates must be finite",
                ));
            }
            if output.is_null() {
                return Err(FfiError::invalid("text-position output pointer is null"));
            }
            lifecycle.geometry_snapshot()?;
            output.write(
                lifecycle
                    .runtime
                    .text_position_at_point(x, y)
                    .map_err(geometry_error)?,
            );
            Ok(())
        })
    }
}

/// Hit-test a surface-logical point to a UAX #29 text range.
///
/// # Safety
///
/// `engine` must be live and both range outputs must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_text_range_at_point(
    engine: *mut JianEngine,
    x: f32,
    y: f32,
    granularity: i32,
    start: *mut u32,
    end: *mut u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if !x.is_finite() || !y.is_finite() {
                return Err(FfiError::invalid(
                    "text hit-test coordinates must be finite",
                ));
            }
            if start.is_null() || end.is_null() {
                return Err(FfiError::invalid("text-range output pointer is null"));
            }
            let granularity = JianTextGranularity::try_from(granularity)
                .map_err(|_| FfiError::invalid("text granularity is invalid"))?;
            lifecycle.geometry_snapshot()?;
            let value = lifecycle
                .runtime
                .text_range_at_point(
                    x,
                    y,
                    match granularity {
                        JianTextGranularity::Character => Granularity::Character,
                        JianTextGranularity::Word => Granularity::Word,
                    },
                )
                .map_err(geometry_error)?;
            start.write(value.0);
            end.write(value.1);
            Ok(())
        })
    }
}
