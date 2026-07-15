use crate::desc::JianPointerPhase;
#[cfg(debug_assertions)]
use crate::error::{read_utf8, FfiError};
use crate::lifecycle::call_engine;
use crate::{JianEngine, JianStatus};

/// Dispatch one touch event in surface-logical coordinates.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_pointer(
    engine: *mut JianEngine,
    id: u32,
    phase: i32,
    x: f32,
    y: f32,
    now_ms: u64,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let phase = match phase {
                value if value == JianPointerPhase::Down as i32 => JianPointerPhase::Down,
                value if value == JianPointerPhase::Move as i32 => JianPointerPhase::Move,
                value if value == JianPointerPhase::Up as i32 => JianPointerPhase::Up,
                value if value == JianPointerPhase::Cancel as i32 => JianPointerPhase::Cancel,
                _ => return Err(crate::error::FfiError::invalid("pointer phase is invalid")),
            };
            lifecycle.pointer(id, phase, x, y, now_ms)
        })
    }
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_app_number(
    engine: *mut JianEngine,
    key_ptr: *const u8,
    key_len: usize,
    value: *mut f64,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if value.is_null() {
                return Err(FfiError::invalid("app-number output pointer is null"));
            }
            let key = read_utf8(key_ptr, key_len, crate::error::STRING_CAP, "state key")?;
            let number = lifecycle
                .app_number(&key)
                .ok_or_else(|| FfiError::invalid("app state is not numeric"))?;
            value.write(number);
            Ok(())
        })
    }
}
