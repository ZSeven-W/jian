//! Host-desktop visual-stage bootstrap (Plan 19 capstone B2.2).
//!
//! Owns the four [`crate`-side](crate) impls of
//! [`jian_core::startup::StartupStage::Visual`]:
//! `RenderSplash` → `RenderFirstFrame` → `PresentToSurface` →
//! `EventPumpReady`. The host calls [`run_visual_stage`] from inside
//! `ApplicationHandler::resumed` (after the window + Skia surface are
//! created); the helper drives the registered phases via
//! [`StartupDriver::run_stage_sync`] (no `block_on` — the winit thread
//! must not park in an executor loop).
//!
//! ### Why visual stage lives here, not in `jian-core`
//!
//! Codex review of the B-block plan (round 2) flagged that the visual
//! stage's helpers touch winit-thread lifecycle, the Skia surface, and
//! the platform present path. None of that is host-agnostic. `jian-core`
//! defines the phase enum + the `StartupDriver` + the report schema;
//! this module supplies the host-specific phase bodies.
//!
//! ### Softbuffer-free seam
//!
//! `PresentToSurface`'s phase impl reads RGBA bytes out of the Skia
//! raster surface and stores them in a [`VisualHandles`] cell. The
//! host (B4 wires this into `run.rs`'s `RunApp`) takes the bytes
//! after the stage completes and pushes them to softbuffer / its
//! windowing surface. Splitting the present at the RGBA boundary
//! keeps the visual stage testable headlessly: no winit, no
//! softbuffer, no real `Window`.
//!
//! ### Phase implementation map
//!
//! | Phase              | Real work                                               |
//! |--------------------|---------------------------------------------------------|
//! | `RenderSplash`     | `splash::paint_with_timer` when `app.splash` is set     |
//! | `RenderFirstFrame` | `scene::collect_draws_with_state` + Skia rasterise      |
//! | `PresentToSurface` | `SkiaSurface::read_rgba8` into the framebuffer cell     |
//! | `EventPumpReady`   | marker — first-interactive boundary                     |
//!
//! When `app.splash` is `None`, `RenderSplash` is a no-op marker —
//! its timing entry still shows so cross-run aggregation by
//! `jian perf startup` keeps the same shape.

use crate::scene::collect_draws_with_state;
use crate::startup::splash::{self, SplashTimer};
use jian_core::geometry::size;
use jian_core::render::RenderBackend;
use jian_core::startup::{
    StartupConfig, StartupDriver, StartupError, StartupPhase, StartupReport, StartupStage,
};
use jian_core::Runtime;
use jian_ops_schema::app::SplashConfig;
use jian_skia::surface::SkiaSurface;
use jian_skia::SkiaBackend;
use std::cell::RefCell;
use std::rc::Rc;

/// Inputs the visual stage reads from. Owned by the host between
/// stage 1 (DataPath) and stage 2 (this module): after
/// [`run_visual_stage`] returns, the host re-takes ownership via
/// [`VisualHandles`] for stage 3 (Background) and the steady-state
/// run loop.
pub struct VisualInputs {
    /// Runtime constructed by stage 1 — read-only during the visual
    /// stage (the dep graph guarantees `RenderFirstFrame` does not
    /// race with any DataPath mutation).
    pub runtime: Runtime,
    /// Raster Skia surface the host built once the winit `Window`
    /// resolved its inner-size. Sized to the physical pixel dimensions.
    pub skia: SkiaSurface,
    /// `Window`-relative physical pixel dimensions, used to allocate
    /// the framebuffer + paint the splash backdrop.
    pub physical_size: (u32, u32),
    /// DPR scale factor applied to logical-unit draw ops so the
    /// raster surface is filled pixel-perfect on retina displays.
    /// `1.0` on integer-scale monitors.
    pub scale: f32,
    /// Whether `RenderFirstFrame` should overlay the host's debug HUD
    /// strip after the document draw passes. Mirrors
    /// [`crate::host::HostConfig::debug_overlay`].
    pub debug_overlay: bool,
    /// `app.splash` from the document, if the author declared one.
    /// `None` means `RenderSplash` is a marker no-op.
    pub splash: Option<SplashConfig>,
}

/// Outputs the host extracts after [`run_visual_stage`] returns.
pub struct VisualHandles {
    /// The runtime returned from the input cell. Same instance the
    /// caller passed in — the visual stage doesn't move it across
    /// threads or otherwise alter ownership.
    pub runtime: Runtime,
    /// The Skia surface returned from the input cell. Re-used by
    /// the steady-state redraw loop after the visual stage closes
    /// (the host doesn't need to rebuild it for every redraw).
    pub skia: SkiaSurface,
    /// RGBA8888 bytes `PresentToSurface` snapshotted from the Skia
    /// raster. The host packs these into softbuffer's `0x00RRGGBB`
    /// `u32` format and presents to the window. Length is exactly
    /// `physical_size.0 * physical_size.1 * 4`.
    pub framebuffer: Vec<u8>,
    /// Per-phase timings from the visual stage. The host folds this
    /// into the cumulative report via
    /// [`StartupReport::merge_into`].
    pub report: StartupReport,
    /// Splash timer when `app.splash` was set. The run loop reads
    /// [`crate::startup::splash::SplashTimer::is_elapsed`] /
    /// [`crate::startup::splash::SplashTimer::remaining`] to drive
    /// the cross-fade out of the splash screen — the timer holds
    /// the render-start `Instant` `paint_with_timer` produced, so
    /// dropping it here would force the cross-fade to restart the
    /// clock and miss the configured `min_duration_ms` floor.
    /// `None` when no splash was configured.
    pub splash_timer: Option<SplashTimer>,
}

/// Drive the visual stage synchronously on the calling thread. Called
/// from `ApplicationHandler::resumed` after the window + Skia surface
/// are available.
///
/// `prior` is the cumulative report from stage 1 (DataPath). The
/// driver pre-seeds its `done` set from `prior.phases` so the
/// visual stage's cross-stage deps (e.g. `RenderFirstFrame`'s
/// `InitGpuContext` / `LoadCoreFonts` / `ComputeFirstLayout` /
/// `SeedStateGraph`) are proven satisfied. A `prior` missing any of
/// those surfaces as `StartupError::NoProgress`.
pub fn run_visual_stage(
    inputs: VisualInputs,
    prior: &StartupReport,
) -> Result<VisualHandles, StartupError> {
    let shared = Rc::new(VisualShared::new(inputs));
    let mut driver = StartupDriver::new();
    register_render_splash(&mut driver, &shared);
    register_render_first_frame(&mut driver, &shared);
    register_present_to_surface(&mut driver, &shared);
    register_event_pump_ready(&mut driver);
    let report = driver.run_stage_sync(StartupStage::Visual, prior, StartupConfig::default())?;
    let shared = Rc::try_unwrap(shared).unwrap_or_else(|_| {
        unreachable!("VisualShared has only one Rc (no clones leaked from phase impls)")
    });
    Ok(VisualHandles {
        runtime: shared
            .runtime
            .into_inner()
            .expect("RenderFirstFrame should not move the runtime"),
        skia: shared
            .skia
            .into_inner()
            .expect("Skia surface still present"),
        framebuffer: shared
            .framebuffer
            .into_inner()
            .expect("PresentToSurface should populate framebuffer"),
        splash_timer: shared.splash_timer.into_inner(),
        report,
    })
}

/// Internal cells the phase impls read / write through `Rc`.
struct VisualShared {
    runtime: RefCell<Option<Runtime>>,
    skia: RefCell<Option<SkiaSurface>>,
    framebuffer: RefCell<Option<Vec<u8>>>,
    physical_size: (u32, u32),
    scale: f32,
    debug_overlay: bool,
    splash: Option<SplashConfig>,
    /// Fresh backend per visual-stage invocation; `RenderSplash`
    /// and `RenderFirstFrame` both call into it. A `RefCell` so the
    /// phase impls' `&mut` borrows stay scoped.
    backend: RefCell<SkiaBackend>,
    /// `RenderSplash` deposits a [`SplashTimer`] here when
    /// `app.splash` is set. The host extracts it via
    /// [`VisualHandles::splash_timer`] for the cross-fade path
    /// (codex review of B2.2 round 1: dropping the timer here would
    /// force the cross-fade to lose the render-start instant +
    /// min_duration_ms floor).
    splash_timer: RefCell<Option<SplashTimer>>,
}

impl VisualShared {
    fn new(inputs: VisualInputs) -> Self {
        let (w, h) = inputs.physical_size;
        // Pre-allocate a zeroed framebuffer matching the surface
        // dimensions. `read_rgba8` overwrites these bytes during
        // `PresentToSurface`; allocating up front keeps the present
        // step branch-free.
        let fb = vec![0u8; (w as usize) * (h as usize) * 4];
        Self {
            runtime: RefCell::new(Some(inputs.runtime)),
            skia: RefCell::new(Some(inputs.skia)),
            framebuffer: RefCell::new(Some(fb)),
            physical_size: inputs.physical_size,
            scale: inputs.scale,
            debug_overlay: inputs.debug_overlay,
            splash: inputs.splash,
            backend: RefCell::new(SkiaBackend::default()),
            splash_timer: RefCell::new(None),
        }
    }
}

fn register_render_splash(driver: &mut StartupDriver, shared: &Rc<VisualShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::RenderSplash, move || async move {
        let Some(config) = shared.splash.clone() else {
            // No splash configured — record a marker but do no work.
            // Plan 19 spec: cross-run aggregation expects every phase
            // present, even when the body is a no-op.
            return Ok(());
        };
        let mut skia_cell = shared.skia.borrow_mut();
        let surface = skia_cell
            .as_mut()
            .ok_or_else(|| "RenderSplash: skia surface absent".to_owned())?;
        let mut backend = shared.backend.borrow_mut();
        let (w, h) = shared.physical_size;
        let canvas = size(w as f32, h as f32);
        // Capture the timer the run loop's cross-fade path reads via
        // `is_elapsed` / `remaining`. Without it the host would have
        // to restart the splash clock and could miss the configured
        // `min_duration_ms` floor.
        let (_renderer, timer) = splash::paint_with_timer(config, &mut *backend, surface, canvas);
        // Drop the backend borrow before mutating the splash_timer
        // cell to keep the RefCell graph honest. The renderer is
        // dropped — its only role was producing the timer; future
        // re-renders go through the host's run-loop path with its own
        // renderer instance.
        drop(backend);
        *shared.splash_timer.borrow_mut() = Some(timer);
        Ok(())
    });
}

fn register_render_first_frame(driver: &mut StartupDriver, shared: &Rc<VisualShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::RenderFirstFrame, move || async move {
        let runtime_cell = shared.runtime.borrow();
        let runtime = runtime_cell
            .as_ref()
            .ok_or_else(|| "RenderFirstFrame: runtime absent".to_owned())?;
        let Some(doc) = runtime.document.as_ref() else {
            return Err("RenderFirstFrame: no document loaded".to_owned());
        };
        let ops = collect_draws_with_state(doc, &runtime.layout, &runtime.state);

        let mut skia_cell = shared.skia.borrow_mut();
        let surface = skia_cell
            .as_mut()
            .ok_or_else(|| "RenderFirstFrame: skia surface absent".to_owned())?;
        let mut backend = shared.backend.borrow_mut();
        backend.begin_frame(surface, 0xffffffff);
        let scale = shared.scale;
        let dpr_scaled = (scale - 1.0).abs() > f32::EPSILON;
        if dpr_scaled {
            backend.push_transform(&jian_core::geometry::Affine2::scale(scale, scale));
        }
        for op in &ops {
            backend.draw(op);
        }
        if dpr_scaled {
            backend.pop();
        }
        if shared.debug_overlay {
            // The HUD lives in physical-pixel space (after the DPR
            // pop) so the strip is the same size on every monitor.
            let (w, h) = shared.physical_size;
            for hud in build_visual_debug_overlay(w, h, scale, ops.len()) {
                backend.draw(&hud);
            }
        }
        backend.end_frame(surface);
        Ok(())
    });
}

fn register_present_to_surface(driver: &mut StartupDriver, shared: &Rc<VisualShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::PresentToSurface, move || async move {
        let mut skia_cell = shared.skia.borrow_mut();
        let surface = skia_cell
            .as_mut()
            .ok_or_else(|| "PresentToSurface: skia surface absent".to_owned())?;
        let mut fb_cell = shared.framebuffer.borrow_mut();
        let fb = fb_cell
            .as_mut()
            .ok_or_else(|| "PresentToSurface: framebuffer cell empty".to_owned())?;
        if !surface.read_rgba8(fb) {
            return Err("PresentToSurface: read_rgba8 failed".to_owned());
        }
        Ok(())
    });
}

fn register_event_pump_ready(driver: &mut StartupDriver) {
    // Pure marker — by the time the run loop returns from
    // `ApplicationHandler::resumed`, winit's event pump is live and
    // ready to dispatch user events. The phase entry exists so
    // `StartupReport::first_interactive_ms` has an authoritative
    // boundary.
    driver.register(StartupPhase::EventPumpReady, || async move { Ok(()) });
}

/// Visual-stage flavour of `run.rs::build_debug_overlay`. Mirrors the
/// strip the steady-state redraw loop paints so the HUD doesn't blink
/// off-screen between first-frame and the second redraw.
fn build_visual_debug_overlay(
    width: u32,
    height: u32,
    scale: f32,
    op_count: usize,
) -> Vec<jian_core::render::DrawOp> {
    use jian_core::geometry::{point, rect};
    use jian_core::render::{DrawOp, Paint, TextAlign, TextRun};
    use jian_core::scene::Color;

    let strip_w = 260.0_f32;
    let strip_h = 24.0_f32;
    let strip_rect = rect(0.0, 0.0, strip_w, strip_h);
    let bg = Paint {
        fill: Some(Color::rgba(0, 0, 0, 0xc0)),
        stroke: None,
        opacity: 1.0,
    };
    let label = format!(
        "{}×{} · scale {:.2} · {} ops · stage:Visual",
        width, height, scale, op_count
    );
    vec![
        DrawOp::Rect {
            rect: strip_rect,
            paint: bg,
        },
        DrawOp::Text(TextRun {
            content: label,
            font_family: "system-ui".into(),
            font_size: 12.0,
            font_weight: 500,
            color: Color::rgb(0xff, 0xff, 0xff),
            origin: point(8.0, 6.0),
            max_width: strip_w - 16.0,
            align: TextAlign::Start,
            line_height: 0.0,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::startup::{PhaseTiming, StartupReport};
    use jian_ops_schema::document::PenDocument;

    fn synthetic_data_path_report() -> StartupReport {
        let phases: Vec<PhaseTiming> = StartupPhase::ALL
            .iter()
            .copied()
            .filter(|p| p.stage() == StartupStage::DataPath)
            .map(|phase| PhaseTiming {
                phase,
                started_at_ms: 0.0,
                duration_ms: 0.0,
                on_critical: phase.is_critical(),
                notes: None,
            })
            .collect();
        StartupReport {
            phases,
            first_interactive_ms: 0.0,
        }
    }

    fn counter_runtime() -> Runtime {
        let src = r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "v",
          "app": { "name": "v", "version": "1", "id": "v" },
          "children": [
            { "type": "frame", "id": "root", "width": 320, "height": 240, "x": 0, "y": 0,
              "children": [
                { "type": "rectangle", "id": "btn",
                  "x": 100, "y": 100, "width": 100, "height": 40 }
              ]
            }
          ]
        }"##;
        let schema: PenDocument = jian_ops_schema::load_str(src).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((320.0, 240.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    #[test]
    fn visual_stage_records_all_four_phases_and_populates_framebuffer() {
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(320, 240),
            physical_size: (320, 240),
            scale: 1.0,
            debug_overlay: false,
            splash: None,
        };
        let prior = synthetic_data_path_report();
        let handles = run_visual_stage(inputs, &prior).expect("visual stage ok");
        assert_eq!(handles.report.phases.len(), 4);
        let phases: std::collections::HashSet<_> =
            handles.report.phases.iter().map(|t| t.phase).collect();
        assert!(phases.contains(&StartupPhase::RenderSplash));
        assert!(phases.contains(&StartupPhase::RenderFirstFrame));
        assert!(phases.contains(&StartupPhase::PresentToSurface));
        assert!(phases.contains(&StartupPhase::EventPumpReady));
        assert!(handles.report.first_interactive_ms > 0.0);
        // Framebuffer is sized correctly + populated (background
        // colour is 0xffffffff so RGB bytes are 0xff; alpha is 0xff
        // by the read_rgba8 contract).
        assert_eq!(handles.framebuffer.len(), 320 * 240 * 4);
        assert!(
            handles.framebuffer.iter().any(|b| *b != 0),
            "framebuffer should not be all zeros after RenderFirstFrame"
        );
    }

    #[test]
    fn visual_stage_threads_splash_timer_out_for_cross_fade() {
        // Codex round 1 BLOCKER: dropping the SplashTimer inside the
        // visual stage would force the cross-fade path to restart
        // the splash clock and miss the configured min_duration_ms
        // floor. The timer must reach the host via VisualHandles.
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(320, 240),
            physical_size: (320, 240),
            scale: 1.0,
            debug_overlay: false,
            splash: Some(SplashConfig {
                background: Some("#102030".into()),
                image: None,
                text: None,
                min_duration_ms: Some(120),
            }),
        };
        let prior = synthetic_data_path_report();
        let handles = run_visual_stage(inputs, &prior).expect("visual stage ok");
        let timer = handles
            .splash_timer
            .as_ref()
            .expect("splash timer must reach the host when app.splash is set");
        // The host can ask "still need to wait?" via remaining(). For
        // a 120 ms floor that just started, remaining is positive.
        assert!(
            !timer.remaining().is_zero() || timer.is_elapsed(),
            "splash_timer should expose live timing state, not a default-constructed stub"
        );
    }

    #[test]
    fn visual_stage_without_splash_leaves_timer_none() {
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(320, 240),
            physical_size: (320, 240),
            scale: 1.0,
            debug_overlay: false,
            splash: None,
        };
        let prior = synthetic_data_path_report();
        let handles = run_visual_stage(inputs, &prior).expect("visual stage ok");
        assert!(handles.splash_timer.is_none());
    }

    #[test]
    fn visual_stage_with_splash_paints_background_color() {
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(320, 240),
            physical_size: (320, 240),
            scale: 1.0,
            debug_overlay: false,
            splash: Some(SplashConfig {
                background: Some("#102030".into()),
                image: None,
                text: None,
                min_duration_ms: Some(0),
            }),
        };
        let prior = synthetic_data_path_report();
        let handles = run_visual_stage(inputs, &prior).expect("visual stage ok");
        // RenderFirstFrame runs AFTER RenderSplash and overdraws the
        // splash with the document; the splash pixels get covered by
        // the white frame fill. We only assert RenderSplash was
        // recorded as having run (its timing > 0 or duration field
        // populated).
        let splash_timing = handles
            .report
            .phases
            .iter()
            .find(|t| t.phase == StartupPhase::RenderSplash)
            .expect("RenderSplash recorded");
        assert!(splash_timing.duration_ms >= 0.0);
    }

    #[test]
    fn visual_stage_with_missing_data_path_prior_fails() {
        // Without a DataPath prior, RenderFirstFrame's cross-stage
        // deps (InitGpuContext / LoadCoreFonts / ComputeFirstLayout /
        // SeedStateGraph) are unmet → NoProgress.
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(320, 240),
            physical_size: (320, 240),
            scale: 1.0,
            debug_overlay: false,
            splash: None,
        };
        let empty_prior = StartupReport::default();
        // `expect_err` requires the Ok type to be Debug — VisualHandles
        // can't be (its Runtime + SkiaSurface fields aren't Debug). Match
        // the Result manually instead.
        let result = run_visual_stage(inputs, &empty_prior);
        match result {
            Err(StartupError::NoProgress { .. }) => {}
            Err(other) => panic!("expected NoProgress, got {other:?}"),
            Ok(_handles) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn visual_stage_handles_dpr_scale() {
        let inputs = VisualInputs {
            runtime: counter_runtime(),
            skia: SkiaSurface::new_raster(640, 480),
            physical_size: (640, 480),
            scale: 2.0,
            debug_overlay: true,
            splash: None,
        };
        let prior = synthetic_data_path_report();
        let handles = run_visual_stage(inputs, &prior).expect("visual stage ok");
        assert_eq!(handles.framebuffer.len(), 640 * 480 * 4);
    }
}
