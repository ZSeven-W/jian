//! EGL/GLES-backed Skia frames for a shell-owned `ANativeWindow`.
//!
//! Mirrors the ownership shape of [`super::metal::MetalSurface`]: the window
//! handle is BORROWED (the caller — `jian-jni` — owns the
//! `ANativeWindow_fromSurface` / `ANativeWindow_release` pairing), suspend is
//! expressed by dropping this object, and resume constructs a new one from
//! the next window. The `EGLDisplay` is process-lifetime: it is initialized
//! once and `eglTerminate` is never called (per-display refcounts are shared
//! across engines in one process; terminating from one engine would tear the
//! display out from under the others).

use std::ffi::c_void;

use khronos_egl as egl;
use skia_safe::gpu::{self, backend_render_targets, SurfaceOrigin};
use skia_safe::ColorType;

use crate::SkiaSurface;

type Instance = egl::Instance<egl::Static>;

/// `EGL_OPENGL_ES3_BIT` (EGL 1.5 / `EGL_KHR_create_context`); khronos-egl
/// only exports the ES1/ES2 renderable bits as constants.
const OPENGL_ES3_BIT: egl::Int = 0x0040;

/// One-frame failure taxonomy (M4 plan Task 2).
#[derive(Debug)]
pub enum EglFrameError {
    /// A defunct-object EGL code (`CONTEXT_LOST`, `BAD_CONTEXT`,
    /// `BAD_SURFACE`, `BAD_NATIVE_WINDOW`) from ANY call in the frame —
    /// make-current included. The object must be dropped; recovery is the
    /// shell-driven suspend → resume cycle. Never retried in place.
    ContextLost(String),
    /// Any other EGL/Skia failure. Same `GpuError` mapping and the same
    /// shell recovery policy; the variants differ only in the logged text.
    Fatal(String),
}

impl std::fmt::Display for EglFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContextLost(detail) => write!(f, "ContextLost: {detail}"),
            Self::Fatal(detail) => write!(f, "Fatal: {detail}"),
        }
    }
}

fn is_defunct(error: egl::Error) -> bool {
    matches!(
        error,
        egl::Error::ContextLost
            | egl::Error::BadContext
            | egl::Error::BadSurface
            | egl::Error::BadNativeWindow
    )
}

/// Process-lifetime default display, initialized exactly once on SUCCESS.
/// A failed initialization is NOT cached — a later engine may retry (e.g.
/// after the platform recovers) instead of being poisoned forever.
fn shared_display(instance: &Instance) -> Result<egl::Display, String> {
    static DISPLAY: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);
    let mut stored = DISPLAY.lock().map_err(|_| "EGL display lock poisoned")?;
    if let Some(raw) = *stored {
        return Ok(unsafe { egl::Display::from_ptr(raw as *mut c_void) });
    }
    // SAFETY: DEFAULT_DISPLAY is the spec-defined sentinel; no pointer is
    // dereferenced.
    let display =
        unsafe { instance.get_display(egl::DEFAULT_DISPLAY) }.ok_or("no default EGL display")?;
    instance
        .initialize(display)
        .map_err(|error| format!("eglInitialize: {error:?}"))?;
    *stored = Some(display.as_ptr() as usize);
    Ok(display)
}

/// A persistent EGL context and Skia GL context bound to a borrowed window.
pub struct EglSurface {
    instance: Instance,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    /// `Option` so `Drop` can tear Skia down while the context is current
    /// (a `Drop` body runs BEFORE fields drop).
    gr_context: Option<gpu::DirectContext>,
    sample_count: usize,
    stencil_bits: usize,
    /// Extent of the render target the CURRENT `BackendRenderTarget` cache
    /// (none kept — rebuilt per frame) was last built for; informational.
    last_extent: (i32, i32),
}

impl EglSurface {
    /// Creates an EGL/GLES renderer for a shell-owned `ANativeWindow*`.
    ///
    /// Transactional: every intermediate failure unwinds the EGL objects
    /// created so far — a failed constructor leaves NO live EGL object.
    ///
    /// # Safety
    ///
    /// `window` must point to a live `ANativeWindow`. The caller keeps it
    /// alive until this value is dropped and replaces it only via
    /// drop-then-construct (the FFI suspend → resume path).
    pub unsafe fn from_native_window(window: *mut c_void) -> Result<Self, String> {
        if window.is_null() {
            return Err("jian-skia: ANativeWindow pointer is null".into());
        }
        let instance = Instance::new(egl::Static);
        let display = shared_display(&instance)?;

        // Two-pass config selection: ES3 first, ES2 on failure (a single
        // ES2|ES3 mask can fail outright on ES2-only devices).
        let config_attribs = |renderable: egl::Int| {
            [
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
                renderable,
                egl::NONE,
            ]
        };
        let (config, context_version) =
            match instance.choose_first_config(display, &config_attribs(OPENGL_ES3_BIT)) {
                Ok(Some(config)) => (config, 3),
                _ => match instance
                    .choose_first_config(display, &config_attribs(egl::OPENGL_ES2_BIT))
                {
                    Ok(Some(config)) => (config, 2),
                    Ok(None) => return Err("jian-skia: no matching EGL config (ES3 or ES2)".into()),
                    Err(error) => return Err(format!("jian-skia: eglChooseConfig: {error:?}")),
                },
            };
        // Query failures are constructor failures: inventing render-target
        // metadata would hand Skia wrong default-framebuffer properties.
        let sample_count = instance
            .get_config_attrib(display, config, egl::SAMPLES)
            .map_err(|error| format!("jian-skia: eglGetConfigAttrib(SAMPLES): {error:?}"))?
            .max(0) as usize;
        let stencil_bits = instance
            .get_config_attrib(display, config, egl::STENCIL_SIZE)
            .map_err(|error| format!("jian-skia: eglGetConfigAttrib(STENCIL_SIZE): {error:?}"))?
            .max(0) as usize;

        let context_attribs = [egl::CONTEXT_CLIENT_VERSION, context_version, egl::NONE];
        let context = instance
            .create_context(display, config, None, &context_attribs)
            .map_err(|error| format!("jian-skia: eglCreateContext: {error:?}"))?;

        let surface = match instance.create_window_surface(display, config, window.cast(), None) {
            Ok(surface) => surface,
            Err(error) => {
                let _ = instance.destroy_context(display, context);
                return Err(format!("jian-skia: eglCreateWindowSurface: {error:?}"));
            }
        };
        if let Err(error) =
            instance.make_current(display, Some(surface), Some(surface), Some(context))
        {
            let _ = instance.destroy_surface(display, surface);
            let _ = instance.destroy_context(display, context);
            return Err(format!("jian-skia: eglMakeCurrent: {error:?}"));
        }

        let unwind = |instance: &Instance| {
            let _ = instance.make_current(display, None, None, None);
            let _ = instance.destroy_surface(display, surface);
            let _ = instance.destroy_context(display, context);
        };
        let Some(interface) = gpu::gl::Interface::new_native() else {
            unwind(&instance);
            return Err("jian-skia: Skia could not load native GL symbols".into());
        };
        let Some(gr_context) = gpu::direct_contexts::make_gl(interface, None) else {
            unwind(&instance);
            return Err("jian-skia: Skia could not create a GL direct context".into());
        };

        Ok(Self {
            instance,
            display,
            context,
            surface,
            gr_context: Some(gr_context),
            sample_count,
            stencil_bits,
            last_extent: (0, 0),
        })
    }

    /// Paints one frame.
    ///
    /// `Ok(false)` = unpaintable-but-live (zero-extent surface, transient
    /// `BAD_ACCESS` make-current contention): `draw` was NOT called, the
    /// engine keeps its dirty bit, and the FFI arm folds the retry into the
    /// frame's single end-of-frame directive. Defunct-object codes from ANY
    /// call return [`EglFrameError::ContextLost`]; everything else is
    /// [`EglFrameError::Fatal`].
    pub fn draw_frame(
        &mut self,
        draw: impl FnOnce(&mut SkiaSurface),
    ) -> Result<bool, EglFrameError> {
        if let Err(error) = self.instance.make_current(
            self.display,
            Some(self.surface),
            Some(self.surface),
            Some(self.context),
        ) {
            if is_defunct(error) {
                return Err(EglFrameError::ContextLost(format!(
                    "eglMakeCurrent: {error:?}"
                )));
            }
            if error == egl::Error::BadAccess {
                return Ok(false);
            }
            return Err(EglFrameError::Fatal(format!("eglMakeCurrent: {error:?}")));
        }

        // The extent is queried EVERY frame: rotation may deliver only a
        // resize (no suspend/resume), so it must never be cached across
        // frames.
        let width = self
            .instance
            .query_surface(self.display, self.surface, egl::WIDTH)
            .map_err(|error| self.classify(error, "eglQuerySurface(WIDTH)"))?;
        let height = self
            .instance
            .query_surface(self.display, self.surface, egl::HEIGHT)
            .map_err(|error| self.classify(error, "eglQuerySurface(HEIGHT)"))?;
        if width <= 0 || height <= 0 {
            return Ok(false);
        }
        self.last_extent = (width, height);

        let fb_info = gpu::gl::FramebufferInfo {
            fboid: 0,
            format: gpu::gl::Format::RGBA8.into(),
            ..Default::default()
        };
        let target = backend_render_targets::make_gl(
            (width, height),
            self.sample_count,
            self.stencil_bits,
            fb_info,
        );
        let gr_context = self
            .gr_context
            .as_mut()
            .ok_or_else(|| EglFrameError::Fatal("GL direct context missing".into()))?;
        let surface = gpu::surfaces::wrap_backend_render_target(
            gr_context,
            &target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        )
        .ok_or_else(|| EglFrameError::Fatal("Skia could not wrap the GL framebuffer".into()))?;
        let mut surface = SkiaSurface { inner: surface };

        draw(&mut surface);
        gr_context.flush_and_submit();
        // The wrapped surface must not outlive the swap.
        drop(surface);

        self.instance
            .swap_buffers(self.display, self.surface)
            .map_err(|error| self.classify(error, "eglSwapBuffers"))?;
        Ok(true)
    }

    fn classify(&self, error: egl::Error, call: &str) -> EglFrameError {
        if is_defunct(error) {
            EglFrameError::ContextLost(format!("{call}: {error:?}"))
        } else {
            EglFrameError::Fatal(format!("{call}: {error:?}"))
        }
    }
}

impl Drop for EglSurface {
    fn drop(&mut self) {
        // Skia must be torn down while the GL context is current; if it
        // cannot be made current anymore, abandon instead (releases CPU-side
        // resources without touching GL).
        let current_ok = self
            .instance
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .is_ok();
        if let Some(mut gr_context) = self.gr_context.take() {
            if current_ok {
                drop(gr_context);
            } else {
                gr_context.abandon();
                drop(gr_context);
            }
        }
        let _ = self.instance.make_current(self.display, None, None, None);
        let _ = self.instance.destroy_surface(self.display, self.surface);
        let _ = self.instance.destroy_context(self.display, self.context);
        // The display is process-lifetime: eglTerminate is never called.
    }
}
