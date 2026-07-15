#[cfg(debug_assertions)]
use crate::desc::JianTestCallClass;
use crate::desc::{Callbacks, CreateOptions, JianPointerPhase};
use crate::diagnostics;
use crate::error::{FfiError, FfiResult};
use crate::render::{paint_commands, prepare_commands};
use crate::viewport::{JianInsets, JianRect};
use crate::JianStatus;
use jian_core::geometry::point;
use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::runtime::{FrameDirective, Runtime};
use jian_skia::{SkiaBackend, SkiaSurface};
use std::cell::{Cell, RefCell, UnsafeCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread::ThreadId;

const MAX_PHYSICAL_AXIS: u32 = 16_384;
const MAX_PHYSICAL_AREA: u64 = 33_554_432;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderMode {
    Unselected,
    Cpu,
    Gpu,
}

pub(crate) struct Lifecycle {
    runtime: Runtime,
    backend: SkiaBackend,
    mode: RenderMode,
    suspended: bool,
    pending_relayout: bool,
    logical: (f32, f32),
    physical: (u32, u32),
    dpr: f32,
    insets: JianInsets,
    keyboard: f32,
    callbacks: Callbacks,
    cpu_surface: Option<SkiaSurface>,
    #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
    metal_surface: Option<jian_skia::surface::metal::MetalSurface>,
    _storage_dir: Option<String>,
    _asset_base: Option<String>,
}

impl Lifecycle {
    pub(crate) fn new(options: CreateOptions) -> FfiResult<Self> {
        let physical = validate_viewport(options.width, options.height, options.dpr)?;
        let mut runtime = Runtime::new();
        runtime.load_str(&options.document).map_err(|error| {
            FfiError::new(
                JianStatus::BadDocument,
                format!("document could not be loaded: {error}"),
            )
        })?;
        runtime.enable_action_reporting();
        runtime
            .state
            .set_viewport(options.width, options.height, options.dpr);
        runtime
            .state
            .set_viewport_occlusion(0.0, 0.0, 0.0, 0.0, 0.0);
        runtime.scheduler.flush();
        #[cfg(feature = "textlayout")]
        runtime
            .build_layout_with(
                std::rc::Rc::new(jian_skia::SkiaMeasure::new()),
                (options.width, options.height),
            )
            .map_err(layout_error)?;
        #[cfg(not(feature = "textlayout"))]
        runtime
            .build_layout((options.width, options.height))
            .map_err(layout_error)?;
        runtime
            .state
            .set_viewport(options.width, options.height, options.dpr);
        runtime
            .state
            .set_viewport_occlusion(0.0, 0.0, 0.0, 0.0, 0.0);
        runtime.scheduler.flush();
        if let Some(asset_base) = options.asset_base.as_ref() {
            runtime.set_image_document_dir(asset_base);
        }
        if options.callbacks.ime_control.is_none() {
            runtime.push_load_warning(
                "ime_control callback is null; platform composition will cancel locally",
            );
        }

        let mut lifecycle = Self {
            runtime,
            backend: SkiaBackend::new(),
            mode: RenderMode::Unselected,
            suspended: false,
            pending_relayout: false,
            logical: (options.width, options.height),
            physical,
            dpr: options.dpr,
            insets: JianInsets::default(),
            keyboard: 0.0,
            callbacks: options.callbacks,
            cpu_surface: None,
            #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
            metal_surface: None,
            _storage_dir: options.storage_dir,
            _asset_base: options.asset_base,
        };
        lifecycle.emit_runtime_diagnostics();
        Ok(lifecycle)
    }

    pub(crate) fn pixel_size(&self) -> (u32, u32) {
        self.physical
    }

    #[cfg(debug_assertions)]
    pub(crate) fn insets(&self) -> (JianInsets, f32) {
        (self.insets, self.keyboard)
    }

    pub(crate) fn resize(&mut self, width: f32, height: f32, dpr: f32) -> FfiResult<()> {
        let physical = validate_viewport(width, height, dpr)?;
        let previous = (
            self.logical,
            self.physical,
            self.dpr,
            self.insets,
            self.keyboard,
        );
        self.logical = (width, height);
        self.physical = physical;
        self.dpr = dpr;
        self.clamp_occlusion();
        self.runtime
            .set_viewport_size_without_relayout((width, height));
        self.runtime.state.set_viewport(width, height, dpr);
        self.sync_occlusion_signals();

        if self.suspended {
            self.runtime.set_text_geometry_ready(false);
            self.pending_relayout = true;
        } else if let Err(error) = self.runtime.relayout() {
            self.logical = previous.0;
            self.physical = previous.1;
            self.dpr = previous.2;
            self.insets = previous.3;
            self.keyboard = previous.4;
            self.runtime.set_viewport_size_without_relayout(previous.0);
            self.runtime
                .state
                .set_viewport(previous.0 .0, previous.0 .1, previous.2);
            self.sync_occlusion_signals();
            let _ = self.runtime.relayout();
            return Err(layout_error(error));
        }
        self.cpu_surface = None;
        self.notify(None);
        Ok(())
    }

    pub(crate) fn set_safe_area(&mut self, insets: JianInsets) -> FfiResult<()> {
        let values = [insets.top, insets.right, insets.bottom, insets.left];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(FfiError::invalid(
                "safe-area insets must be finite and nonnegative",
            ));
        }
        if insets.top > self.logical.1
            || insets.bottom > self.logical.1
            || insets.left > self.logical.0
            || insets.right > self.logical.0
            || insets.top + insets.bottom > self.logical.1
            || insets.left + insets.right > self.logical.0
        {
            return Err(FfiError::invalid(
                "safe-area insets exceed the logical surface",
            ));
        }
        let previous = self.insets;
        self.insets = insets;
        if let Err(error) = self.occlusion_changed() {
            self.insets = previous;
            self.restore_occlusion_after_failure();
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn set_keyboard(&mut self, height: f32) -> FfiResult<()> {
        if !height.is_finite() || height < 0.0 || height > self.logical.1 {
            return Err(FfiError::invalid(
                "keyboard height must fit the logical surface",
            ));
        }
        let previous = self.keyboard;
        self.keyboard = height;
        if let Err(error) = self.occlusion_changed() {
            self.keyboard = previous;
            self.restore_occlusion_after_failure();
            return Err(error);
        }
        Ok(())
    }

    fn occlusion_changed(&mut self) -> FfiResult<()> {
        self.sync_occlusion_signals();
        if self.suspended {
            self.pending_relayout = true;
            self.runtime.set_text_geometry_ready(false);
        } else {
            self.runtime.relayout().map_err(layout_error)?;
        }
        self.notify(None);
        Ok(())
    }

    fn sync_occlusion_signals(&mut self) {
        self.runtime.state.set_viewport_occlusion(
            self.insets.top,
            self.insets.right,
            self.insets.bottom,
            self.insets.left,
            self.keyboard,
        );
        self.runtime.scheduler.flush();
        self.runtime.mark_dirty();
    }

    fn restore_occlusion_after_failure(&mut self) {
        self.sync_occlusion_signals();
        if !self.suspended {
            let _ = self.runtime.relayout();
        }
    }

    fn clamp_occlusion(&mut self) {
        self.insets.top = self.insets.top.min(self.logical.1);
        self.insets.bottom = self.insets.bottom.min(self.logical.1);
        scale_pair(
            &mut self.insets.top,
            &mut self.insets.bottom,
            self.logical.1,
        );
        self.insets.left = self.insets.left.min(self.logical.0);
        self.insets.right = self.insets.right.min(self.logical.0);
        scale_pair(
            &mut self.insets.left,
            &mut self.insets.right,
            self.logical.0,
        );
        self.keyboard = self.keyboard.min(self.logical.1);
    }

    pub(crate) unsafe fn attach_surface(&mut self, handle: *mut std::ffi::c_void) -> FfiResult<()> {
        if self.mode != RenderMode::Unselected {
            return Err(FfiError::invalid("render mode is already selected"));
        }
        #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
        {
            let surface = unsafe { create_metal_surface(handle) }?;
            self.metal_surface = Some(surface);
        }
        #[cfg(not(all(feature = "metal", any(target_os = "macos", target_os = "ios"))))]
        unsafe {
            create_metal_surface(handle)?;
        }
        self.mode = RenderMode::Gpu;
        self.suspended = false;
        self.runtime.mark_dirty();
        self.notify(None);
        Ok(())
    }

    pub(crate) fn suspend(&mut self) {
        if self.mode == RenderMode::Unselected || self.suspended {
            return;
        }
        #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
        {
            self.metal_surface = None;
        }
        self.suspended = true;
        self.pending_relayout = true;
        self.runtime.set_text_geometry_ready(false);
    }

    pub(crate) unsafe fn resume(
        &mut self,
        surface: Option<*mut std::ffi::c_void>,
    ) -> FfiResult<()> {
        match self.mode {
            RenderMode::Unselected => {
                if surface.is_some() {
                    return Err(FfiError::invalid(
                        "resume before mode selection requires a null surface",
                    ));
                }
                return Ok(());
            }
            RenderMode::Cpu => {
                if surface.is_some() {
                    return Err(FfiError::invalid("CPU resume requires a null surface"));
                }
                if !self.suspended {
                    return Ok(());
                }
            }
            RenderMode::Gpu => {
                if !self.suspended {
                    return Err(FfiError::invalid(
                        "GPU surface replacement requires suspend then resume",
                    ));
                }
                let handle = surface
                    .ok_or_else(|| FfiError::invalid("GPU resume requires a surface descriptor"))?;
                #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
                {
                    let metal = unsafe { create_metal_surface(handle) }?;
                    self.metal_surface = Some(metal);
                }
                #[cfg(not(all(feature = "metal", any(target_os = "macos", target_os = "ios"))))]
                unsafe {
                    create_metal_surface(handle)?;
                }
            }
        }
        if self.pending_relayout {
            if let Err(error) = self.runtime.relayout() {
                #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
                if self.mode == RenderMode::Gpu {
                    self.metal_surface = None;
                }
                return Err(layout_error(error));
            }
        }
        self.suspended = false;
        self.pending_relayout = false;
        self.runtime.mark_dirty();
        self.notify(None);
        Ok(())
    }

    pub(crate) fn frame_gpu(&mut self, now_ms: u64) -> FfiResult<()> {
        if self.suspended {
            return Err(FfiError::new(JianStatus::Suspended, "engine is suspended"));
        }
        if self.mode != RenderMode::Gpu {
            return Err(FfiError::invalid("GPU mode has not been attached"));
        }
        self.frame_gpu_selected(now_ms)
    }

    #[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
    fn frame_gpu_selected(&mut self, now_ms: u64) -> FfiResult<()> {
        let directive = self.runtime.pump(now_ms);
        let commands = prepare_commands(&mut self.runtime, &mut self.backend);
        let presented = {
            let mut surface = self
                .metal_surface
                .take()
                .ok_or_else(|| FfiError::new(JianStatus::GpuError, "GPU surface is unavailable"))?;
            let result = surface
                .draw_frame(|frame| paint_commands(&mut self.backend, frame, commands, self.dpr));
            self.metal_surface = Some(surface);
            result.map_err(|message| FfiError::new(JianStatus::GpuError, message))?
        };
        if presented {
            self.runtime.frame_presented();
        }
        self.notify(Some(directive));
        Ok(())
    }

    #[cfg(not(all(feature = "metal", any(target_os = "macos", target_os = "ios"))))]
    fn frame_gpu_selected(&mut self, _now_ms: u64) -> FfiResult<()> {
        Err(FfiError::new(
            JianStatus::GpuError,
            "Metal support is unavailable in this build",
        ))
    }

    pub(crate) unsafe fn frame_cpu(
        &mut self,
        now_ms: u64,
        buffer: *mut u8,
        buffer_len: usize,
        stride: usize,
    ) -> FfiResult<()> {
        if self.suspended {
            return Err(FfiError::new(JianStatus::Suspended, "engine is suspended"));
        }
        if self.mode == RenderMode::Gpu {
            return Err(FfiError::invalid("engine is permanently in GPU mode"));
        }
        let (width, height) = self.physical;
        let row_bytes = (width as usize)
            .checked_mul(4)
            .ok_or_else(|| FfiError::invalid("CPU row-byte calculation overflowed"))?;
        if buffer.is_null() {
            return Err(FfiError::invalid("CPU frame buffer is null"));
        }
        if stride < row_bytes {
            return Err(FfiError::invalid("CPU frame stride is too small"));
        }
        let required = stride
            .checked_mul(height as usize)
            .ok_or_else(|| FfiError::invalid("CPU frame size calculation overflowed"))?;
        if required > buffer_len || buffer_len > isize::MAX as usize {
            return Err(FfiError::invalid("CPU frame buffer is too small"));
        }
        let contiguous_len = row_bytes
            .checked_mul(height as usize)
            .ok_or_else(|| FfiError::invalid("CPU readback size overflowed"))?;

        let mut surface = match self.cpu_surface.take() {
            Some(surface)
                if surface.width() == width as i32 && surface.height() == height as i32 =>
            {
                surface
            }
            _ => SkiaSurface::try_new_raster(width as i32, height as i32).ok_or_else(|| {
                FfiError::new(JianStatus::OutOfMemory, "CPU raster allocation failed")
            })?,
        };
        let directive = self.runtime.pump(now_ms);
        let commands = prepare_commands(&mut self.runtime, &mut self.backend);
        paint_commands(&mut self.backend, &mut surface, commands, self.dpr);

        let mut pixels = Vec::new();
        pixels.try_reserve_exact(contiguous_len).map_err(|_| {
            FfiError::new(JianStatus::OutOfMemory, "CPU readback allocation failed")
        })?;
        pixels.resize(contiguous_len, 0);
        if !surface.read_rgba8(&mut pixels) {
            self.cpu_surface = Some(surface);
            return Err(FfiError::new(
                JianStatus::GpuError,
                "Skia CPU readback failed",
            ));
        }
        let output = unsafe { std::slice::from_raw_parts_mut(buffer, required) };
        for (source, destination) in pixels
            .chunks_exact(row_bytes)
            .zip(output.chunks_exact_mut(stride))
        {
            destination[..row_bytes].copy_from_slice(source);
        }
        self.runtime.frame_presented();
        self.cpu_surface = Some(surface);
        if self.mode == RenderMode::Unselected {
            self.mode = RenderMode::Cpu;
        }
        self.notify(Some(directive));
        Ok(())
    }

    pub(crate) fn pointer(
        &mut self,
        id: u32,
        phase: JianPointerPhase,
        x: f32,
        y: f32,
        now_ms: u64,
    ) -> FfiResult<()> {
        if self.suspended {
            return Err(FfiError::new(JianStatus::Suspended, "engine is suspended"));
        }
        if !x.is_finite() || !y.is_finite() {
            return Err(FfiError::invalid("pointer coordinates must be finite"));
        }
        let phase = match phase {
            JianPointerPhase::Down => PointerPhase::Down,
            JianPointerPhase::Move => PointerPhase::Move,
            JianPointerPhase::Up => PointerPhase::Up,
            JianPointerPhase::Cancel => PointerPhase::Cancel,
        };
        self.runtime.dispatch_pointer(PointerEvent::simple_at(
            id,
            phase,
            point(x.clamp(-1.0e6, 1.0e6), y.clamp(-1.0e6, 1.0e6)),
            now_ms,
        ));
        self.runtime.mark_dirty();
        self.notify(None);
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn app_number(&self, key: &str) -> Option<f64> {
        self.runtime.state.app_get(key)?.as_f64()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn viewport_number(&self, key: &str) -> Option<f64> {
        self.runtime.state.viewport_snapshot().get(key)?.as_f64()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn node_rect(&self, id: &str) -> Option<JianRect> {
        let document = self.runtime.document.as_ref()?;
        let key = document.tree.get(id)?;
        let rect = self.runtime.layout.node_rect(key)?;
        Some(JianRect {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }

    pub(crate) fn emit_runtime_diagnostics(&mut self) {
        diagnostics::drain_runtime(
            &mut self.runtime,
            self.callbacks.runtime_error,
            self.callbacks.user_data,
        );
    }

    pub(crate) fn emit_call_error(&self, error: &FfiError) {
        diagnostics::emit_call_error(
            self.callbacks.runtime_error,
            self.callbacks.user_data,
            error.status,
            &error.message,
        );
    }

    #[cfg(debug_assertions)]
    pub(crate) fn test_suspended_status(&self, class: JianTestCallClass) -> JianStatus {
        if !self.suspended {
            return JianStatus::Ok;
        }
        match class {
            JianTestCallClass::TextGeometry => JianStatus::NotReady,
            JianTestCallClass::TextContent
            | JianTestCallClass::ImeText
            | JianTestCallClass::CapabilityResult
            | JianTestCallClass::RegisterFont => JianStatus::Ok,
        }
    }

    fn notify(&self, directive: Option<FrameDirective>) {
        let Some(callback) = self.callbacks.needs_redraw else {
            return;
        };
        let next = directive.and_then(|value| value.next_wake_ms);
        unsafe { callback(self.callbacks.user_data, next.is_some(), next.unwrap_or(0)) };
    }
}

fn layout_error(error: jian_core::error::CoreError) -> FfiError {
    FfiError::new(JianStatus::LayoutError, format!("layout failed: {error}"))
}

pub(crate) fn validate_viewport(width: f32, height: f32, dpr: f32) -> FfiResult<(u32, u32)> {
    if !width.is_finite() || width <= 0.0 {
        return Err(FfiError::invalid("width must be finite and positive"));
    }
    if !height.is_finite() || height <= 0.0 {
        return Err(FfiError::invalid("height must be finite and positive"));
    }
    if !dpr.is_finite() || dpr <= 0.0 || dpr > 16.0 {
        return Err(FfiError::invalid("dpr must be finite and in (0, 16]"));
    }
    let physical_width = (f64::from(width) * f64::from(dpr)).round();
    let physical_height = (f64::from(height) * f64::from(dpr)).round();
    if !(1.0..=f64::from(MAX_PHYSICAL_AXIS)).contains(&physical_width)
        || !(1.0..=f64::from(MAX_PHYSICAL_AXIS)).contains(&physical_height)
    {
        return Err(FfiError::invalid("physical dimensions are out of range"));
    }
    let width = physical_width as u32;
    let height = physical_height as u32;
    let area = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| FfiError::invalid("physical area overflowed"))?;
    if area > MAX_PHYSICAL_AREA {
        return Err(FfiError::invalid("physical area exceeds the limit"));
    }
    Ok((width, height))
}

fn scale_pair(first: &mut f32, second: &mut f32, extent: f32) {
    let sum = *first + *second;
    if sum > extent && sum > 0.0 {
        let factor = extent / sum;
        *first *= factor;
        *second *= factor;
    }
}

#[cfg(all(feature = "metal", any(target_os = "macos", target_os = "ios")))]
unsafe fn create_metal_surface(
    handle: *mut std::ffi::c_void,
) -> FfiResult<jian_skia::surface::metal::MetalSurface> {
    unsafe { jian_skia::surface::metal::MetalSurface::from_ca_metal_layer(handle) }
        .map_err(|message| FfiError::new(JianStatus::GpuError, message))
}

#[cfg(not(all(feature = "metal", any(target_os = "macos", target_os = "ios"))))]
unsafe fn create_metal_surface(_handle: *mut std::ffi::c_void) -> FfiResult<()> {
    Err(FfiError::new(
        JianStatus::GpuError,
        "Metal support is unavailable in this build",
    ))
}

pub struct JianEngine {
    owner: ThreadId,
    in_call: Cell<bool>,
    poisoned: Cell<bool>,
    last_error: RefCell<String>,
    lifecycle: UnsafeCell<Lifecycle>,
}

impl JianEngine {
    pub(crate) fn new(lifecycle: Lifecycle) -> Self {
        Self {
            owner: std::thread::current().id(),
            in_call: Cell::new(false),
            poisoned: Cell::new(false),
            last_error: RefCell::new(String::new()),
            lifecycle: UnsafeCell::new(lifecycle),
        }
    }

    pub(crate) fn error(&self) -> String {
        self.last_error.borrow().clone()
    }
}

pub(crate) unsafe fn call_engine(
    pointer: *mut JianEngine,
    call: impl FnOnce(&mut Lifecycle) -> FfiResult<()>,
) -> JianStatus {
    if pointer.is_null() {
        return JianStatus::InvalidArg;
    }
    let engine = unsafe { &*pointer };
    if engine.owner != std::thread::current().id() || engine.in_call.get() {
        return JianStatus::WrongThread;
    }
    if engine.poisoned.get() {
        return JianStatus::Poisoned;
    }
    engine.in_call.set(true);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let lifecycle = unsafe { &mut *engine.lifecycle.get() };
        let result = call(lifecycle);
        if let Err(error) = &result {
            lifecycle.emit_call_error(error);
        }
        lifecycle.emit_runtime_diagnostics();
        result
    }));
    engine.in_call.set(false);
    match outcome {
        Ok(Ok(())) => JianStatus::Ok,
        Ok(Err(error)) => {
            *engine.last_error.borrow_mut() = error.message;
            error.status
        }
        Err(_) => {
            engine.poisoned.set(true);
            *engine.last_error.borrow_mut() = "panic crossed the Jian ABI boundary".to_owned();
            JianStatus::Poisoned
        }
    }
}

pub(crate) unsafe fn destroy_engine(pointer: *mut JianEngine) -> JianStatus {
    if pointer.is_null() {
        return JianStatus::InvalidArg;
    }
    let engine = unsafe { &*pointer };
    if engine.owner != std::thread::current().id() || engine.in_call.get() {
        return JianStatus::WrongThread;
    }
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(pointer));
    })) {
        Ok(()) => JianStatus::Ok,
        Err(_) => JianStatus::Poisoned,
    }
}
