//! M4 Task-1 spike: one Skia-drawn red rect through EGL/GLES into an
//! `ANativeWindow`. Everything runs synchronously inside the JNI call —
//! deliberately NOT the engine architecture, just the toolchain proof.

use jni::objects::{JClass, JObject};
use jni::sys::jint;
use jni::JNIEnv;
use khronos_egl as egl;

type EglInstance = egl::Instance<egl::Static>;

/// # Safety
/// Called by the JVM with a valid `Surface`; the window is used only inside
/// this call.
#[no_mangle]
pub unsafe extern "system" fn Java_dev_jian_spike_MainActivity_nativeSpike(
    env: JNIEnv,
    _class: JClass,
    surface: JObject,
) -> jint {
    match run(env, surface) {
        Ok(()) => 0,
        Err(message) => {
            // Surface the failure in logcat; the spike has no richer channel.
            eprintln!("jian-spike FAILED: {message}");
            log(&format!("jian-spike FAILED: {message}"));
            1
        }
    }
}

fn log(message: &str) {
    // `eprintln!` is not visible in logcat; write via __android_log_write.
    use std::ffi::CString;
    let tag = CString::new("JianSpike").unwrap();
    let text = CString::new(message).unwrap_or_else(|_| CString::new("log error").unwrap());
    unsafe {
        ndk_sys::__android_log_write(
            ndk_sys::android_LogPriority::ANDROID_LOG_INFO.0 as i32,
            tag.as_ptr(),
            text.as_ptr(),
        );
    }
}

unsafe fn run(env: JNIEnv, surface: JObject) -> Result<(), String> {
    let window =
        ndk_sys::ANativeWindow_fromSurface(env.get_raw() as *mut _, surface.as_raw() as *mut _);
    if window.is_null() {
        return Err("ANativeWindow_fromSurface returned null".into());
    }
    let result = render(window);
    ndk_sys::ANativeWindow_release(window);
    result
}

unsafe fn render(window: *mut ndk_sys::ANativeWindow) -> Result<(), String> {
    let egl = EglInstance::new(egl::Static);
    let display = egl
        .get_display(egl::DEFAULT_DISPLAY)
        .ok_or("no default EGL display")?;
    egl.initialize(display)
        .map_err(|e| format!("eglInitialize: {e:?}"))?;

    let attribs = [
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES2_BIT,
        egl::NONE,
    ];
    let config = egl
        .choose_first_config(display, &attribs)
        .map_err(|e| format!("eglChooseConfig: {e:?}"))?
        .ok_or("no matching EGL config")?;

    let context_attribs = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
    let context = egl
        .create_context(display, config, None, &context_attribs)
        .map_err(|e| format!("eglCreateContext: {e:?}"))?;

    let egl_surface = egl
        .create_window_surface(display, config, window.cast(), None)
        .map_err(|e| format!("eglCreateWindowSurface: {e:?}"))?;
    egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
        .map_err(|e| format!("eglMakeCurrent: {e:?}"))?;

    let width = ndk_sys::ANativeWindow_getWidth(window);
    let height = ndk_sys::ANativeWindow_getHeight(window);
    log(&format!("jian-spike window {width}x{height}"));

    let interface =
        skia_safe::gpu::gl::Interface::new_native().ok_or("skia GL Interface::new_native")?;
    let mut context = skia_safe::gpu::direct_contexts::make_gl(interface, None)
        .ok_or("skia make_gl DirectContext")?;

    let fb_info = skia_safe::gpu::gl::FramebufferInfo {
        fboid: 0,
        format: skia_safe::gpu::gl::Format::RGBA8.into(),
        ..Default::default()
    };
    let target = skia_safe::gpu::backend_render_targets::make_gl(
        (width, height),
        None, // sample count
        8,    // stencil bits (matches common window configs)
        fb_info,
    );
    let mut sk_surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
        &mut context,
        &target,
        skia_safe::gpu::SurfaceOrigin::BottomLeft,
        skia_safe::ColorType::RGBA8888,
        None,
        None,
    )
    .ok_or("wrap_backend_render_target")?;

    let canvas = sk_surface.canvas();
    canvas.clear(skia_safe::Color::WHITE);
    let mut paint = skia_safe::Paint::default();
    paint.set_color(skia_safe::Color::RED);
    paint.set_anti_alias(true);
    let rect = skia_safe::Rect::from_xywh(
        width as f32 * 0.2,
        height as f32 * 0.35,
        width as f32 * 0.6,
        height as f32 * 0.3,
    );
    canvas.draw_rect(rect, &paint);
    context.flush_and_submit();
    drop(sk_surface);

    egl.swap_buffers(display, egl_surface)
        .map_err(|e| format!("eglSwapBuffers: {e:?}"))?;
    log("jian-spike rendered red rect OK");
    Ok(())
}
