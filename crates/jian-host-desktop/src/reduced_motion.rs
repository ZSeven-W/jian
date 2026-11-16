#[cfg(target_os = "macos")]
pub(crate) fn system_reduced_motion() -> bool {
    // SAFETY: the process-wide workspace getter reads an accessibility flag.
    unsafe {
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        let reduced: objc2::runtime::Bool =
            objc2::msg_send![&*workspace, accessibilityDisplayShouldReduceMotion];
        reduced.as_bool()
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn system_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };

    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the out pointer.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    success != 0 && enabled == 0
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn system_reduced_motion() -> bool {
    false
}
