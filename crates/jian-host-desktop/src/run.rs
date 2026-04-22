//! `DesktopHost::run` — the real winit event loop (feature `run`).
//!
//! Pipeline:
//!   winit WindowEvent
//!     → `PointerTranslator::translate` → `Runtime::dispatch_pointer`
//!     → Runtime flushes the signal scheduler (Plan 6 fix)
//!     → `request_redraw`
//!   winit RedrawRequested
//!     → `scene::collect_draws(document, layout)` → `DrawOp[]`
//!     → `SkiaBackend::{begin_frame, draw*, end_frame}` on a raster surface
//!     → copy raster pixels into `softbuffer::Buffer` → `present()`
//!
//! Raster + softbuffer keeps the presenter platform-agnostic: no
//! Metal / D3D / GL context plumbing until a jian-host-desktop host
//! upgrade lands. Works on every OS winit supports.

use crate::pointer::PointerTranslator;
use crate::scene::collect_draws;
use crate::DesktopHost;
use jian_core::geometry::size as make_size;
use jian_core::render::RenderBackend;
use jian_skia::surface::SkiaSurface;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

impl DesktopHost {
    /// Open a window and run the event loop until the user closes it.
    /// Blocks the calling thread; returns `Ok(())` on clean shutdown.
    pub fn run(self) -> Result<(), winit::error::EventLoopError> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        let mut app = RunApp::new(self);
        event_loop.run_app(&mut app)
    }
}

struct RunApp {
    host: DesktopHost,
    translator: PointerTranslator,
    window: Option<Rc<Window>>,
    softbuffer: Option<SoftbufferState>,
    last_size: (u32, u32),
}

struct SoftbufferState {
    // softbuffer's `Surface` keeps a raw-window-handle reference to the
    // window, so we keep the window `Rc` alive alongside it.
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    skia: SkiaSurface,
}

impl RunApp {
    fn new(host: DesktopHost) -> Self {
        let initial = host.config.initial_size;
        Self {
            host,
            translator: PointerTranslator::new(),
            window: None,
            softbuffer: None,
            last_size: (
                initial.width.max(1.0) as u32,
                initial.height.max(1.0) as u32,
            ),
        }
    }

    fn ensure_surface(&mut self, width: u32, height: u32) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let state = self.softbuffer.get_or_insert_with(|| {
            let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
            let surface =
                softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
            SoftbufferState {
                surface,
                skia: SkiaSurface::new_raster(width.max(1) as i32, height.max(1) as i32),
            }
        });
        let w = NonZeroU32::new(width.max(1)).unwrap();
        let h = NonZeroU32::new(height.max(1)).unwrap();
        let _ = state.surface.resize(w, h);
        if state.skia.width() != width as i32 || state.skia.height() != height as i32 {
            state.skia = SkiaSurface::new_raster(width.max(1) as i32, height.max(1) as i32);
        }
    }

    fn redraw(&mut self) {
        let Some(state) = self.softbuffer.as_mut() else {
            return;
        };
        let (w, h) = self.last_size;

        // 1. Collect draw ops and rasterize via SkiaBackend.
        let ops = if let Some(doc) = self.host.runtime.document.as_ref() {
            collect_draws(doc, &self.host.runtime.layout)
        } else {
            Vec::new()
        };
        self.host.backend.begin_frame(&mut state.skia, 0xffffffff);
        for op in &ops {
            self.host.backend.draw(op);
        }
        self.host.backend.end_frame(&mut state.skia);

        // 2. Snapshot raster bytes as RGBA8888 via SkiaSurface helper.
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        if !state.skia.read_rgba8(&mut rgba) {
            return;
        }

        // 3. Pack RGBA → softbuffer's 0x00RRGGBB u32 pixel format.
        let Ok(mut buf) = state.surface.buffer_mut() else {
            return;
        };
        for (i, pixel) in buf.iter_mut().enumerate() {
            let r = rgba[i * 4] as u32;
            let g = rgba[i * 4 + 1] as u32;
            let b = rgba[i * 4 + 2] as u32;
            *pixel = (r << 16) | (g << 8) | b;
        }
        if let Some(window) = self.window.as_ref() {
            window.pre_present_notify();
        }
        let _ = buf.present();
    }
}

impl ApplicationHandler for RunApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let initial = self.host.config.initial_size;
        let attrs = Window::default_attributes()
            .with_title(&self.host.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                initial.width as f64,
                initial.height as f64,
            ));
        let window = event_loop
            .create_window(attrs)
            .expect("jian-host-desktop: failed to create window");
        let phys = window.inner_size();
        self.last_size = (phys.width.max(1), phys.height.max(1));
        self.window = Some(Rc::new(window));
        self.ensure_surface(self.last_size.0, self.last_size.1);
        // Re-run layout with the actual window size so the scene fills
        // the physical surface on first paint.
        let _ = self
            .host
            .runtime
            .build_layout((phys.width as f32, phys.height as f32));
        self.host.runtime.viewport.size = make_size(phys.width as f32, phys.height as f32);
        self.host.runtime.rebuild_spatial();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(new) => {
                self.last_size = (new.width.max(1), new.height.max(1));
                self.ensure_surface(self.last_size.0, self.last_size.1);
                let _ = self
                    .host
                    .runtime
                    .build_layout((new.width as f32, new.height as f32));
                self.host.runtime.viewport.size = make_size(new.width as f32, new.height as f32);
                self.host.runtime.rebuild_spatial();
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
                return;
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.translator.update_modifiers(mods.state());
                return;
            }
            _ => {}
        }

        if let Some(pe) = self.translator.translate(&event) {
            self.host.runtime.dispatch_pointer(pe);
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drive LongPress + other timer-based recognisers each iteration
        // of the event loop; only request a redraw if the tick fired a
        // semantic event.
        let emitted = self.host.runtime.tick(Instant::now());
        if !emitted.is_empty() {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}
