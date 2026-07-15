//! Metal-backed Skia frames for a shell-owned `CAMetalLayer`.
//!
//! [`MetalSurface`] owns the Metal command queue and Skia context, but stores
//! only a raw borrowed pointer to the layer. It never retains or releases the
//! layer. The shell must keep that layer alive until `MetalSurface` is dropped
//! and must replace it only by dropping/suspending this object before creating
//! another one.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLDrawable, MTLPixelFormat, MTLTexture,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use skia_safe::gpu::{self, backend_render_targets, mtl, SurfaceOrigin};
use skia_safe::ColorType;

use crate::SkiaSurface;

/// A persistent Metal queue and Skia context bound to a borrowed layer.
pub struct MetalSurface {
    layer: NonNull<CAMetalLayer>,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    context: gpu::DirectContext,
}

impl MetalSurface {
    /// Creates a Metal renderer for a shell-owned `CAMetalLayer *`.
    ///
    /// The layer must already have an `MTLDevice`, use a Skia-compatible pixel
    /// format (the Player uses `BGRA8Unorm`), and have `framebufferOnly = false`.
    ///
    /// # Safety
    ///
    /// `layer` must point to a live `CAMetalLayer` on the current thread. The
    /// caller must keep it alive and must not replace it until this value is
    /// dropped. This function borrows the pointer and never retains/releases
    /// the layer.
    pub unsafe fn from_ca_metal_layer(layer: *mut c_void) -> Result<Self, &'static str> {
        let layer = NonNull::new(layer.cast::<CAMetalLayer>())
            .ok_or("jian-skia: CAMetalLayer pointer is null")?;
        let layer_ref = unsafe { layer.as_ref() };
        if unsafe { layer_ref.framebufferOnly() } {
            return Err("jian-skia: CAMetalLayer.framebufferOnly must be false");
        }
        if unsafe { layer_ref.pixelFormat() } != MTLPixelFormat::BGRA8Unorm {
            return Err("jian-skia: CAMetalLayer.pixelFormat must be BGRA8Unorm");
        }

        let device =
            unsafe { layer_ref.device() }.ok_or("jian-skia: CAMetalLayer has no MTLDevice")?;
        let command_queue = device
            .newCommandQueue()
            .ok_or("jian-skia: MTLDevice could not create a command queue")?;
        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&device) as mtl::Handle,
                Retained::as_ptr(&command_queue) as mtl::Handle,
            )
        };
        let context = gpu::direct_contexts::make_metal(&backend, None)
            .ok_or("jian-skia: Skia could not create a Metal direct context")?;

        Ok(Self {
            layer,
            command_queue,
            context,
        })
    }

    /// Acquires, paints, flushes, and presents one layer drawable.
    ///
    /// `Ok(false)` means `nextDrawable` returned `nil`; `draw` is not called,
    /// allowing the engine to leave its dirty bit set and retry later.
    pub fn draw_frame(
        &mut self,
        draw: impl FnOnce(&mut SkiaSurface),
    ) -> Result<bool, &'static str> {
        let layer = unsafe { self.layer.as_ref() };
        let Some(drawable) = (unsafe { layer.nextDrawable() }) else {
            return Ok(false);
        };
        let texture = unsafe { drawable.texture() };
        let width = i32::try_from(texture.width())
            .map_err(|_| "jian-skia: Metal drawable width exceeds i32")?;
        let height = i32::try_from(texture.height())
            .map_err(|_| "jian-skia: Metal drawable height exceeds i32")?;
        if width <= 0 || height <= 0 {
            return Err("jian-skia: Metal drawable has zero size");
        }

        let texture_info =
            unsafe { mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle) };
        let backend_target = backend_render_targets::make_mtl((width, height), &texture_info);
        let surface = gpu::surfaces::wrap_backend_render_target(
            &mut self.context,
            &backend_target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            None,
            None,
        )
        .ok_or("jian-skia: Skia could not wrap the Metal drawable")?;
        let mut surface = SkiaSurface { inner: surface };

        draw(&mut surface);
        self.context.flush_and_submit();
        drop(surface);

        let command_buffer = self
            .command_queue
            .commandBuffer()
            .ok_or("jian-skia: MTLCommandQueue could not create a command buffer")?;
        let drawable = ProtocolObject::<dyn MTLDrawable>::from_ref(&*drawable);
        command_buffer.presentDrawable(drawable);
        command_buffer.commit();
        Ok(true)
    }
}
