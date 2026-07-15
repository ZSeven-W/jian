use crate::error::{read_utf8, FfiError, DOCUMENT_CAP};
use crate::lifecycle::call_engine;
use crate::{JianEngine, JianStatus};

/// Debug-only raw-ABI reload hook used to verify request retirement.
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_reload(
    engine: *mut JianEngine,
    document: *const u8,
    length: usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let source = read_utf8(document, length, DOCUMENT_CAP, "document")?;
            lifecycle
                .runtime
                .load_str_and_relayout(&source)
                .map_err(|error| {
                    FfiError::new(
                        JianStatus::BadDocument,
                        format!("document could not be reloaded: {error}"),
                    )
                })?;
            lifecycle.capabilities.cancel_all();
            lifecycle.refresh_text_geometry();
            Ok(())
        })
    }
}

/// Debug-only raw-ABI probe for parked-build acceptance.
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_variant_build_count(
    engine: *mut JianEngine,
    count: *mut usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if count.is_null() {
                return Err(FfiError::invalid("variant build count output is null"));
            }
            count.write(lifecycle.runtime.last_variant_build_count());
            Ok(())
        })
    }
}
