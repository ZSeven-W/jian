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
use crate::scene::collect_draws_with_state;
use crate::DesktopHost;
use jian_core::geometry::size as make_size;
use jian_core::render::RenderBackend;
use jian_skia::surface::SkiaSurface;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Dev-mode poll interval. Short enough to feel "instant" (≤ frame
/// time at common refresh rates), long enough to keep idle CPU near
/// zero when no file changes arrive.
const RELOAD_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Convert a winit physical-pixel `PhysicalSize<u32>` into an `(f32, f32)`
/// logical-pixel tuple suitable for `Runtime::build_layout`.
fn logical_size_f32(phys: winit::dpi::PhysicalSize<u32>, scale: f64) -> (f32, f32) {
    let w = (phys.width as f64 / scale).max(1.0) as f32;
    let h = (phys.height as f64 / scale).max(1.0) as f32;
    (w, h)
}

/// Map `HostConfig::fullscreen` to a `winit::window::Fullscreen` value.
/// Borderless-on-current-monitor when on, `None` when off. Extracted so
/// the mapping has a unit test without needing a real display.
fn fullscreen_for_config(on: bool) -> Option<winit::window::Fullscreen> {
    if on {
        Some(winit::window::Fullscreen::Borderless(None))
    } else {
        None
    }
}

impl DesktopHost {
    /// Open a window and run the event loop until the user closes it.
    /// Blocks the calling thread; returns `Ok(())` on clean shutdown.
    pub fn run(self) -> Result<(), winit::error::EventLoopError> {
        let event_loop = EventLoop::new()?;
        // Dev / MCP modes poll ~5×/sec so the run loop stays warm for
        // file events + bridge drains; default keeps the original
        // `Wait` so idle CPU is zero.
        let needs_polling = self.reload_rx.is_some() || self.has_mcp_drain();
        let initial_flow = if needs_polling {
            ControlFlow::WaitUntil(Instant::now() + RELOAD_POLL_INTERVAL)
        } else {
            ControlFlow::Wait
        };
        event_loop.set_control_flow(initial_flow);
        let mut app = RunApp::new(self);
        event_loop.run_app(&mut app)
    }

    #[cfg(feature = "mcp")]
    fn has_mcp_drain(&self) -> bool {
        self.mcp_drain.is_some()
    }

    #[cfg(not(feature = "mcp"))]
    fn has_mcp_drain(&self) -> bool {
        false
    }
}

struct RunApp {
    host: DesktopHost,
    translator: PointerTranslator,
    window: Option<Rc<Window>>,
    softbuffer: Option<SoftbufferState>,
    /// Physical surface dimensions (winit `inner_size()` values).
    last_size: (u32, u32),
    /// Pixel-ratio between physical and logical coords. On a 2x Retina
    /// display this is 2.0. Layout + state live in logical units; the
    /// canvas + pointer input are scaled so the raster surface is filled
    /// pixel-perfect.
    scale_factor: f64,
    /// Materialised native menu (when feature `menus` is on AND
    /// `host.config.menu` is non-None). Built once on first window
    /// create and kept alive for the program's lifetime — muda holds
    /// raw pointers internally, so dropping it would invalidate the
    /// menu bar.
    #[cfg(feature = "menus")]
    menu: Option<muda::Menu>,
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
            scale_factor: 1.0,
            #[cfg(feature = "menus")]
            menu: None,
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
        let scale = self.scale_factor as f32;

        // 1. Collect draw ops and rasterize via SkiaBackend. The canvas
        // is scaled by DPR so logical-unit rects fill physical pixels.
        let ops = if let Some(doc) = self.host.runtime.document.as_ref() {
            collect_draws_with_state(doc, &self.host.runtime.layout, &self.host.runtime.state)
        } else {
            Vec::new()
        };
        self.host.backend.begin_frame(&mut state.skia, 0xffffffff);
        let dpr_scaled = (scale - 1.0).abs() > f32::EPSILON;
        if dpr_scaled {
            self.host
                .backend
                .push_transform(&jian_core::geometry::Affine2::scale(scale, scale));
        }
        for op in &ops {
            self.host.backend.draw(op);
        }
        if dpr_scaled {
            self.host.backend.pop();
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
        let mut attrs = Window::default_attributes()
            .with_title(&self.host.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                initial.width as f64,
                initial.height as f64,
            ));
        // Apply the runtime window icon if the host configured one.
        // `to_winit_icon` cloning is fine: the icon is at most a few
        // KB of RGBA pixels and runs once per window creation. A
        // conversion error logs to stderr rather than aborting —
        // a missing-icon is preferable to a missing-window.
        //
        // Per-platform reach (winit 0.30 docs):
        //   - Windows / X11: `set_window_icon` populates the
        //     taskbar + titlebar icon.
        //   - macOS: `set_window_icon` is a no-op. We additionally
        //     push the PNG source bytes through
        //     `set_macos_dock_icon_from_png` so the Dock icon
        //     reflects the schema's `app.icon` instead of the
        //     default unbundled-binary "exec" placeholder.
        //   - Wayland: both paths are no-ops; the launcher reads the
        //     `.desktop` file's `Icon=` line. Plan 8 Task 10
        //     (packaging) ships that.
        if let Some(icon) = self.host.config.icon.clone() {
            #[cfg(target_os = "macos")]
            if let Some(png) = icon.source_png() {
                if let Err(e) = crate::app_icon::set_macos_dock_icon_from_png(png) {
                    eprintln!("jian-host-desktop: macOS Dock icon update failed: {e}");
                }
            }
            match crate::app_icon::to_winit_icon(icon) {
                Ok(winit_icon) => attrs = attrs.with_window_icon(Some(winit_icon)),
                Err(e) => eprintln!("jian-host-desktop: icon conversion failed: {e}"),
            }
        }
        // Borderless-fullscreen on the current monitor when configured.
        // Exclusive fullscreen needs a video-mode query and changes the
        // display resolution; borderless skips both and works the same
        // way across macOS / Windows / Linux. The user can still
        // multitask via OS shortcuts (Cmd+Tab / Alt+Tab / etc.).
        if let Some(fs) = fullscreen_for_config(self.host.config.fullscreen) {
            attrs = attrs.with_fullscreen(Some(fs));
        }
        let window = event_loop
            .create_window(attrs)
            .expect("jian-host-desktop: failed to create window");
        let phys = window.inner_size();
        self.scale_factor = window.scale_factor();
        self.last_size = (phys.width.max(1), phys.height.max(1));
        self.window = Some(Rc::new(window));
        self.ensure_surface(self.last_size.0, self.last_size.1);

        #[cfg(feature = "menus")]
        if self.menu.is_none() {
            if let Some(spec) = self.host.config.menu.clone() {
                let built = crate::menus::build_muda_menu(&spec);
                for w in &built.warnings {
                    eprintln!("jian-host-desktop: menu warning: {}", w);
                }
                if let Some(window) = self.window.as_ref() {
                    match crate::menus::init_menu_for_window(&built.menu, window.as_ref()) {
                        Ok(()) => {
                            // Hold the menu for the program lifetime;
                            // muda keeps raw pointers internally so
                            // dropping it would invalidate the menu bar.
                            self.menu = Some(built.menu);
                        }
                        Err(e) => {
                            // Don't latch the failure: leaving
                            // `self.menu = None` lets the next window
                            // (e.g. after a crash-recovery rebuild)
                            // retry. The Menu instance built here is
                            // dropped — its append() calls registered
                            // child items into a global registry but
                            // the bar binding never landed, so dropping
                            // is safe.
                            eprintln!("jian-host-desktop: menu init failed: {}", e);
                        }
                    }
                }
            }
        }
        // Layout + viewport live in *logical* coordinates; only the
        // raster surface and pointer input use physical pixels.
        let logical = logical_size_f32(phys, self.scale_factor);
        let _ = self.host.runtime.build_layout(logical);
        self.host.runtime.viewport.size = make_size(logical.0, logical.1);
        self.host.runtime.rebuild_spatial();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if std::env::var("JIAN_DEBUG_POINTER").is_ok() {
            // Log every event variant name so a "no MouseInput
            // arriving" diagnosis is one trace away.
            let name = match &event {
                WindowEvent::ActivationTokenDone { .. } => "ActivationTokenDone",
                WindowEvent::Resized(_) => "Resized",
                WindowEvent::Moved(_) => "Moved",
                WindowEvent::CloseRequested => "CloseRequested",
                WindowEvent::Destroyed => "Destroyed",
                WindowEvent::DroppedFile(_) => "DroppedFile",
                WindowEvent::HoveredFile(_) => "HoveredFile",
                WindowEvent::HoveredFileCancelled => "HoveredFileCancelled",
                WindowEvent::Focused(_) => "Focused",
                WindowEvent::KeyboardInput { .. } => "KeyboardInput",
                WindowEvent::ModifiersChanged(_) => "ModifiersChanged",
                WindowEvent::Ime(_) => "Ime",
                WindowEvent::CursorMoved { .. } => "CursorMoved",
                WindowEvent::CursorEntered { .. } => "CursorEntered",
                WindowEvent::CursorLeft { .. } => "CursorLeft",
                WindowEvent::MouseWheel { .. } => "MouseWheel",
                WindowEvent::MouseInput { state, button, .. } => {
                    eprintln!(
                        "win-event: MouseInput state={:?} button={:?}",
                        state, button
                    );
                    "MouseInput"
                }
                WindowEvent::PinchGesture { .. } => "PinchGesture",
                WindowEvent::PanGesture { .. } => "PanGesture",
                WindowEvent::DoubleTapGesture { .. } => "DoubleTapGesture",
                WindowEvent::RotationGesture { .. } => "RotationGesture",
                WindowEvent::TouchpadPressure { .. } => "TouchpadPressure",
                WindowEvent::AxisMotion { .. } => "AxisMotion",
                WindowEvent::Touch(_) => "Touch",
                WindowEvent::ScaleFactorChanged { .. } => "ScaleFactorChanged",
                WindowEvent::ThemeChanged(_) => "ThemeChanged",
                WindowEvent::Occluded(_) => "Occluded",
                WindowEvent::RedrawRequested => "RedrawRequested",
            };
            // Skip the noisy CursorMoved spam — already traced below
            // when it produces a PointerEvent.
            if name != "CursorMoved" && name != "RedrawRequested" {
                eprintln!("win-event: {}", name);
            }
        }
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(new) => {
                self.last_size = (new.width.max(1), new.height.max(1));
                self.ensure_surface(self.last_size.0, self.last_size.1);
                let logical = logical_size_f32(*new, self.scale_factor);
                let _ = self.host.runtime.build_layout(logical);
                self.host.runtime.viewport.size = make_size(logical.0, logical.1);
                self.host.runtime.rebuild_spatial();
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
                return;
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: new_scale,
                ..
            } => {
                self.scale_factor = *new_scale;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
                return;
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use jian_core::gesture::pointer::WheelEvent as JianWheel;
                use winit::event::MouseScrollDelta;
                // Use the translator's last-known cursor position so
                // the wheel hit-tests at the spot the user is hovering.
                let pos = self
                    .translator
                    .cursor
                    .unwrap_or(self.translator.last_known_cursor);
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        // Lines → logical pixels: 16 px per line is
                        // the conventional desktop fallback. Trackpads
                        // generally report PixelDelta directly.
                        (*x * 16.0, *y * 16.0)
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        let s = self.scale_factor as f32;
                        ((p.x as f32) / s, (p.y as f32) / s)
                    }
                };
                let logical_pos = if self.scale_factor != 1.0 {
                    let s = self.scale_factor as f32;
                    jian_core::geometry::point(pos.x / s, pos.y / s)
                } else {
                    pos
                };
                self.host.runtime.dispatch_wheel(JianWheel {
                    position: logical_pos,
                    delta: jian_core::geometry::point(dx, dy),
                    modifiers: self.translator.modifiers,
                    timestamp: Instant::now(),
                });
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
                return;
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.translator.update_modifiers(mods.state());
                return;
            }
            _ => {}
        }

        if let Some(mut pe) = self.translator.translate(&event) {
            // winit delivers cursor positions in physical pixels; the
            // runtime hit-tests against logical-coord layout rects, so
            // divide the incoming position by the scale factor.
            if self.scale_factor != 1.0 {
                let s = self.scale_factor as f32;
                pe.position = jian_core::geometry::point(pe.position.x / s, pe.position.y / s);
            }
            // Trace gate — only emits when JIAN_DEBUG_POINTER=1 so the
            // normal jian player run stays quiet. Set the env var and
            // re-run to see every translated PointerEvent + the
            // semantic events the dispatch fires.
            let trace = std::env::var("JIAN_DEBUG_POINTER").is_ok();
            if trace {
                eprintln!(
                    "pointer: phase={:?} kind={:?} pos=({:.1},{:.1}) buttons={:?}",
                    pe.phase, pe.kind, pe.position.x, pe.position.y, pe.buttons
                );
            }
            let emitted = self.host.runtime.dispatch_pointer(pe);
            if trace && !emitted.is_empty() {
                eprintln!("  emitted {} semantic events:", emitted.len());
                for ev in &emitted {
                    eprintln!("    {:?}", ev);
                }
            }
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Drive LongPress + other timer-based recognisers each iteration
        // of the event loop; only request a redraw if the tick fired a
        // semantic event.
        let emitted = self.host.runtime.tick(Instant::now());
        let mut needs_redraw = !emitted.is_empty();

        // Dev-mode reload: drain the channel; the latest pending doc
        // wins. Re-build layout + spatial against the current logical
        // size so the canvas reflects the new schema immediately.
        if self.host.reload_rx.is_some() {
            let mut latest: Option<jian_ops_schema::document::PenDocument> = None;
            if let Some(ref rx) = self.host.reload_rx {
                while let Ok(doc) = rx.try_recv() {
                    latest = Some(doc);
                }
            }
            if let Some(schema) = latest {
                if let Err(e) = self.apply_reload(schema) {
                    eprintln!("jian dev: reload failed: {}", e);
                } else {
                    needs_redraw = true;
                }
            }
        }

        // MCP bridge drain (cfg-gated). Each pending Request maps to a
        // one-shot reply; failures to send back surface as the bridge
        // worker dropping its receiver, which is fine — we just skip.
        // Bridge dispatch can mutate state, so request a redraw if we
        // actually executed an action.
        #[cfg(feature = "mcp")]
        if self.drain_mcp_requests() {
            needs_redraw = true;
        }

        // Re-arm the polling timer when either reload or mcp wired the
        // host into poll-mode; default Wait stays untouched.
        let needs_polling = self.host.reload_rx.is_some() || self.has_mcp_drain();
        if needs_polling {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + RELOAD_POLL_INTERVAL,
            ));
        }

        if needs_redraw {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}

impl RunApp {
    /// Swap the runtime's document for `schema` and rebuild the layout
    /// + spatial index against the current logical surface size.
    fn apply_reload(
        &mut self,
        schema: jian_ops_schema::document::PenDocument,
    ) -> Result<(), String> {
        let logical = logical_size_f32(
            winit::dpi::PhysicalSize::new(self.last_size.0, self.last_size.1),
            self.scale_factor,
        );
        // The runtime keeps the same StateGraph, services, and gates —
        // we only rebuild the document tree + layout. Existing app
        // state survives the reload (matches user expectations of
        // "edit + save" iteration).
        self.host
            .runtime
            .replace_document(schema)
            .map_err(|e| format!("{:?}", e))?;
        self.host
            .runtime
            .build_layout(logical)
            .map_err(|e| format!("layout: {:?}", e))?;
        self.host.runtime.viewport.size = make_size(logical.0, logical.1);
        self.host.runtime.rebuild_spatial();
        // Refresh the MCP action set so AI clients see the new
        // aiNames immediately after save (no `tools/list` cache to
        // invalidate — rmcp re-queries on demand). When MCP is wired
        // but the surface didn't exist yet (host author called
        // `with_mcp` before any document was loaded), build it now.
        #[cfg(feature = "mcp")]
        {
            let host = &mut self.host;
            if host.mcp_drain.is_some() {
                if let Some(doc) = host.runtime.document.as_ref() {
                    match host.mcp_surface.as_mut() {
                        Some(surface) => {
                            surface.refresh(&doc.schema, &host.mcp_salt);
                        }
                        None => {
                            let mut surface = jian_action_surface::ActionSurface::from_document(
                                &doc.schema,
                                &host.mcp_salt,
                            )
                            .with_session_id("mcp");
                            if let Some(audit) = host.mcp_audit.as_ref() {
                                surface = surface.with_audit(audit.clone());
                            }
                            host.mcp_surface = Some(surface);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn has_mcp_drain(&self) -> bool {
        #[cfg(feature = "mcp")]
        {
            self.host.mcp_drain.is_some()
        }
        #[cfg(not(feature = "mcp"))]
        {
            false
        }
    }

    /// Drain the MCP bridge once per `about_to_wait`. Returns `true`
    /// iff we executed at least one action (caller redraws).
    /// `list_available_actions` doesn't mutate state and doesn't
    /// trigger a redraw.
    ///
    /// Both `list` and `execute` honour spec §4.2 #4 (dynamic state
    /// gate): live `bindings.visible` / `bindings.disabled` filter
    /// the listing AND short-circuit execute with `state_gated`.
    /// Without the gate, MCP would advertise every statically-visible
    /// action regardless of UI state — and §10 data-hiding forbids
    /// that, since AI clients infer node tree structure from the
    /// presence/absence of derived names.
    #[cfg(feature = "mcp")]
    fn drain_mcp_requests(&mut self) -> bool {
        use jian_action_surface::mcp::Request;
        use jian_action_surface::{ClosureGate, RuntimeDispatcher};
        use jian_core::action_surface::RuntimeStateGate;
        let mut executed = false;
        let Some(drain) = self.host.mcp_drain.as_mut() else {
            return false;
        };
        let pending: Vec<Request> = drain.drain();
        for req in pending {
            if !req.worker_listening() {
                // Client disconnected mid-call — spec §10 wants the
                // surface untouched in that case.
                continue;
            }
            match req {
                Request::List { opts, reply } => {
                    let Some(surface) = self.host.mcp_surface.as_ref() else {
                        continue;
                    };
                    // Build the dynamic gate from the live runtime —
                    // immutable borrow only, no conflict with surface.
                    let response = match self.host.runtime.document.as_ref() {
                        Some(doc) => {
                            let gate = RuntimeStateGate::new(
                                doc,
                                &self.host.runtime.state,
                                self.host.runtime.expr_cache.clone(),
                            );
                            surface.list_with_gate(opts, &gate)
                        }
                        None => surface.list(opts),
                    };
                    let _ = reply.send(response);
                }
                Request::Execute {
                    name,
                    params,
                    reply,
                } => {
                    let Some(surface) = self.host.mcp_surface.as_mut() else {
                        continue;
                    };
                    // The state gate borrows `&Runtime`, the
                    // dispatcher needs `&mut Runtime`. Resolve the
                    // gate verdict for THIS action up-front, capture
                    // it in a `ClosureGate`, then drop the immutable
                    // borrow before constructing the dispatcher.
                    let allowed = {
                        let runtime = &self.host.runtime;
                        match runtime.document.as_ref() {
                            Some(doc) => {
                                let gate = RuntimeStateGate::new(
                                    doc,
                                    &runtime.state,
                                    runtime.expr_cache.clone(),
                                );
                                // `find_action` matches canonical names
                                // AND aliases — same matcher
                                // `execute_with_gate` uses, so an alias
                                // can't bypass this pre-computed gate
                                // verdict.
                                surface
                                    .find_action(&name)
                                    .map(|a| gate.allows(&a.source_node_id))
                                    // unknown_action is handled by the
                                    // surface itself — pretend "allowed"
                                    // here so the lookup error wins.
                                    .unwrap_or(true)
                            }
                            None => true,
                        }
                    };
                    let gate = ClosureGate(move |_id: &str| allowed);
                    let mut dispatcher = RuntimeDispatcher::new(&mut self.host.runtime);
                    let outcome =
                        surface.execute_with_gate(&name, params.as_ref(), &mut dispatcher, &gate);
                    if matches!(outcome, jian_action_surface::ExecuteOutcome::Ok) {
                        executed = true;
                    }
                    let _ = reply.send(outcome);
                }
            }
        }
        executed
    }
}

#[cfg(test)]
mod fullscreen_tests {
    use super::fullscreen_for_config;
    use winit::window::Fullscreen;

    #[test]
    fn off_returns_none() {
        assert!(fullscreen_for_config(false).is_none());
    }

    #[test]
    fn on_returns_borderless_current_monitor() {
        match fullscreen_for_config(true) {
            Some(Fullscreen::Borderless(None)) => {}
            other => panic!("expected Borderless(None), got {other:?}"),
        }
    }
}
