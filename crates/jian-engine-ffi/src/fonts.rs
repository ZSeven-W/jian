use crate::capabilities::CAPABILITY_CAP;
use crate::error::{FfiError, FfiResult};
use crate::lifecycle::call_engine;
use crate::{JianEngine, JianStatus};

/// Register a TTF/OTF face in Jian's process-global Skia registry.
///
/// # Safety
/// `engine` must be live and `bytes` must cover `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_register_font(
    engine: *mut JianEngine,
    bytes: *const u8,
    length: usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let owned = read_font(bytes, length)?;
            #[cfg(feature = "textlayout")]
            {
                jian_skia::register_imported_font(owned)
                    .map(|_| ())
                    .map_err(|error| {
                        FfiError::invalid(format!("font registration failed: {error}"))
                    })?;
                // §6.5 fanout: immediate relayout on the registering engine
                // (other engines catch the generation in their next pump).
                lifecycle.note_font_registered();
                Ok(())
            }
            #[cfg(not(feature = "textlayout"))]
            {
                let _ = lifecycle;
                let _ = owned;
                Err(FfiError::invalid(
                    "font registration requires the textlayout feature",
                ))
            }
        })
    }
}

unsafe fn read_font(bytes: *const u8, length: usize) -> FfiResult<Vec<u8>> {
    if length == 0 {
        return Err(FfiError::invalid("font data is empty"));
    }
    if length > CAPABILITY_CAP {
        return Err(FfiError::invalid("font data exceeds 64 MiB"));
    }
    if bytes.is_null() || length > isize::MAX as usize {
        return Err(FfiError::invalid("font pointer or length is invalid"));
    }
    Ok(unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec())
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_font_generation(
    engine: *mut JianEngine,
    generation: *mut u64,
) -> JianStatus {
    unsafe {
        call_engine(engine, |_| {
            if generation.is_null() {
                return Err(FfiError::invalid("font generation output is null"));
            }
            #[cfg(feature = "textlayout")]
            generation.write(jian_skia::font_generation());
            #[cfg(not(feature = "textlayout"))]
            generation.write(0);
            Ok(())
        })
    }
}
