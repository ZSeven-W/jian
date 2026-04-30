//! Host-agnostic [`crate::startup::StartupStage::DataPath`] phase
//! implementations (Plan 19 capstone B1).
//!
//! `HostAgnosticBootstrap` wires real impls for the eight DataPath
//! phases against a [`Runtime`] (and, optionally, a source `.op`
//! file). It is the second half of the capstone foundation B0 laid:
//! B0 introduced the typed staging API, this module fills in the
//! actual work the host's Stage 1 measures.
//!
//! ### What this module does NOT do
//!
//! - Does not register Visual phases. The host crate's
//!   `startup_bootstrap` does that (B2) — it's the only layer that
//!   sees a winit `Window` + draw surface.
//! - Does not register Background phases. Plan 19 D1 (AOT writer +
//!   reader) and D2 (font subsetter) are the real bodies that fill
//!   in `BuildFullSpatial` / `LoadRemainingFonts` / `DecodeImages`;
//!   this module hands the host a default driver they can layer
//!   over.
//! - Does not own the `block_on`. The host calls
//!   `block_on(driver.run_stage(StartupStage::DataPath, &report,
//!   StartupConfig::default()))` from a worker thread before opening
//!   the window. This module's job is to populate the driver's impl
//!   table; lifecycle is the host's call.
//!
//! ### Phase implementation map
//!
//! | Phase                 | Real work                                            |
//! |-----------------------|------------------------------------------------------|
//! | `ReadFile`            | `std::fs::read_to_string` (only for `File` source)   |
//! | `ParseSchema`         | `jian_ops_schema::load_str`                          |
//! | `SeedStateGraph`      | `Runtime::new_from_document` (state + tree atomic)   |
//! | `BuildNodeTree`       | no-op (covered by `SeedStateGraph`)                  |
//! | `InitGpuContext`      | host-agnostic no-op (host overrides per backend)     |
//! | `LoadCoreFonts`       | host-agnostic no-op (real subset → Plan 19 D2)       |
//! | `ComputeFirstLayout`  | `Runtime::build_layout(viewport)`                    |
//! | `BuildVisibleSpatial` | `Runtime::rebuild_spatial_for_first_frame(viewport)` |
//!
//! `SeedStateGraph` and `BuildNodeTree` share a single
//! `Runtime::new_from_document` call because the runtime constructor
//! does both atomically. We attribute the wall-clock cost to
//! `SeedStateGraph` (the dependency-graph successor `ComputeFirstLayout`
//! reads from the constructed runtime regardless of attribution) and
//! leave `BuildNodeTree` as a marker no-op. A future Runtime refactor
//! could split the constructor; this module's contract stays the same.
//!
//! ### Per-phase `PhaseTiming.notes`
//!
//! Currently every phase records `notes: None`. Plan 19 mid-flight
//! note flagged a richer notes contract (e.g. `"Metal"` on
//! `InitGpuContext`, `"<N> bytes"` on `ReadFile`); the driver's
//! current `register` shape only accepts `Result<(), String>` so a
//! note would need a phase-result extension first. B2 / D2 land that
//! API change alongside the host-side overrides that have a real
//! note to attach.
//!
//! ### Sharing the runtime across phases
//!
//! Phase impl closures are `'static`, so they can't borrow a single
//! `Runtime` across stages. We thread an `Rc<BootstrapShared>` whose
//! interior cells hold the source string, the parsed schema, and the
//! constructed runtime. Phase ordering is gated by the dep graph so
//! the cells are always in the right state when each phase reads.
//! After `run_stage(DataPath)` returns, the host calls
//! [`BootstrapHandles::take_runtime`] to extract the constructed
//! runtime for the visual stage and beyond.

use crate::spatial::NodeBBox;
use crate::startup::driver::StartupDriver;
use crate::startup::phase::StartupPhase;
use crate::Runtime;
use jian_ops_schema::document::PenDocument;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Where the bootstrap reads its `.op` from.
///
/// `Schema` is heap-boxed so the enum's discriminant carries pointers
/// only — `PenDocument` itself is several hundred bytes once expanded
/// and would dominate the variant size budget otherwise.
pub enum BootstrapSource {
    /// Read the file on disk during the `ReadFile` phase. Synchronous
    /// I/O — phase impls are driven by a worker thread's `block_on`
    /// per Plan 19 host integration.
    File(PathBuf),
    /// Pre-loaded source text. The `ReadFile` phase short-circuits
    /// (records its sub-millisecond synchronisation cost only);
    /// useful for tests that don't want disk I/O on the timing path.
    String(String),
    /// Already-parsed schema. The `ReadFile` and `ParseSchema` phases
    /// both short-circuit. Useful when a host hot-reload re-runs
    /// startup against an in-memory schema.
    Schema(Box<PenDocument>),
}

/// Hands the host the runtime constructed by the bootstrap. Returned
/// from [`HostAgnosticBootstrap::install_data_path`]; consumed after
/// `run_stage(DataPath)` resolves.
pub struct BootstrapHandles {
    shared: Rc<BootstrapShared>,
}

impl BootstrapHandles {
    /// Take the constructed runtime out of the shared cell. Returns
    /// `None` if the bootstrap aborted before SeedStateGraph (or if
    /// `take_runtime` is called twice on the same handle).
    pub fn take_runtime(&self) -> Option<Runtime> {
        self.shared.runtime.borrow_mut().take()
    }

    /// Take the off-viewport bbox set the visible-spatial phase
    /// produced. The Background stage's `BuildFullSpatial` consumes
    /// this via [`crate::spatial::SpatialIndex::fill_rest`] so the
    /// spatial index ends up covering every node without a second
    /// scene-tree walk. Returns `None` if `BuildVisibleSpatial`
    /// hasn't run yet, or if `take_hidden_bboxes` was already called.
    pub fn take_hidden_bboxes(&self) -> Option<Vec<NodeBBox>> {
        self.shared.hidden_bboxes.borrow_mut().take()
    }
}

/// Internal cells the phase impls read / write through `Rc`.
struct BootstrapShared {
    source: BootstrapSource,
    /// Filled by `ReadFile` (or seeded by `BootstrapSource::String`).
    source_text: RefCell<Option<String>>,
    /// Filled by `ParseSchema` (or seeded by `BootstrapSource::Schema`).
    schema: RefCell<Option<PenDocument>>,
    /// Filled by `SeedStateGraph` via `Runtime::new_from_document`.
    runtime: RefCell<Option<Runtime>>,
    /// Filled by `BuildVisibleSpatial` — the off-viewport bbox set
    /// `rebuild_spatial_for_first_frame` returned. The `Background`
    /// stage's `BuildFullSpatial` reads this to call
    /// `SpatialIndex::fill_rest` without re-walking every node.
    /// `None` until visible spatial runs.
    hidden_bboxes: RefCell<Option<Vec<NodeBBox>>>,
    /// Caller-supplied first-frame viewport, in logical pixels.
    viewport: (f32, f32),
}

/// Host-agnostic DataPath bootstrap. Stateless type — every method
/// is associated; instances exist only to scope the public surface.
pub struct HostAgnosticBootstrap;

impl HostAgnosticBootstrap {
    /// Register impls for every [`crate::startup::StartupStage::DataPath`]
    /// phase against `driver`. Returns a [`BootstrapHandles`] the
    /// host uses to recover the constructed runtime after the stage
    /// completes.
    ///
    /// `viewport` is the first-frame logical-pixel size. The host
    /// reads this from the user's `--size` flag, the schema's root
    /// frame, or a platform default.
    pub fn install_data_path(
        driver: &mut StartupDriver,
        source: BootstrapSource,
        viewport: (f32, f32),
    ) -> BootstrapHandles {
        let shared = Rc::new(BootstrapShared {
            source_text: RefCell::new(None),
            schema: RefCell::new(None),
            runtime: RefCell::new(None),
            hidden_bboxes: RefCell::new(None),
            viewport,
            source,
        });
        // Pre-seed cells from the source variant so the relevant
        // phase impls below short-circuit (their bodies see the
        // pre-seeded cell and return Ok immediately).
        match &shared.source {
            BootstrapSource::File(_) => {}
            BootstrapSource::String(s) => {
                *shared.source_text.borrow_mut() = Some(s.clone());
            }
            BootstrapSource::Schema(doc) => {
                *shared.source_text.borrow_mut() = Some(String::new());
                *shared.schema.borrow_mut() = Some((**doc).clone());
            }
        }

        register_read_file(driver, &shared);
        register_parse_schema(driver, &shared);
        register_seed_state_graph(driver, &shared);
        register_build_node_tree(driver, &shared);
        register_init_gpu_context(driver);
        register_load_core_fonts(driver);
        register_compute_first_layout(driver, &shared);
        register_build_visible_spatial(driver, &shared);

        BootstrapHandles { shared }
    }
}

fn register_read_file(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::ReadFile, move || async move {
        match &shared.source {
            BootstrapSource::File(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("read {}: {e}", path.display()))?;
                *shared.source_text.borrow_mut() = Some(text);
                Ok(())
            }
            BootstrapSource::String(_) | BootstrapSource::Schema(_) => {
                // Already pre-seeded by `install_data_path`; the
                // phase records its timing as zero-cost.
                Ok(())
            }
        }
    });
}

fn register_parse_schema(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::ParseSchema, move || async move {
        if shared.schema.borrow().is_some() {
            // BootstrapSource::Schema seeded the cell already.
            return Ok(());
        }
        let text_ref = shared.source_text.borrow();
        let text = text_ref
            .as_deref()
            .ok_or_else(|| "ReadFile produced no source text".to_owned())?;
        let outcome = jian_ops_schema::load_str(text).map_err(|e| format!("parse: {e}"))?;
        // Drop the borrow before mutating the schema cell to avoid a
        // RefCell collision when subsequent phases read source_text.
        drop(text_ref);
        *shared.schema.borrow_mut() = Some(outcome.value);
        Ok(())
    });
}

fn register_seed_state_graph(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::SeedStateGraph, move || async move {
        if shared.runtime.borrow().is_some() {
            // Idempotent if a host re-runs the data stage.
            return Ok(());
        }
        let schema = shared
            .schema
            .borrow_mut()
            .take()
            .ok_or_else(|| "ParseSchema produced no schema".to_owned())?;
        let runtime = Runtime::new_from_document(schema)
            .map_err(|e| format!("Runtime::new_from_document: {e}"))?;
        *shared.runtime.borrow_mut() = Some(runtime);
        Ok(())
    });
}

fn register_build_node_tree(driver: &mut StartupDriver, _shared: &Rc<BootstrapShared>) {
    // `Runtime::new_from_document` already built the node tree atomically
    // with seeding the state graph (Plan 19 design note in the bootstrap
    // module doc). `BuildNodeTree`'s wall-clock portion is therefore a
    // sub-millisecond synchronisation — recorded so the dependency graph
    // stays whole. A future Runtime refactor that splits state seeding
    // from tree building will drop a real body here.
    driver.register(StartupPhase::BuildNodeTree, || async move { Ok(()) });
}

fn register_init_gpu_context(driver: &mut StartupDriver) {
    // Host-agnostic: jian-core doesn't know what backend the host
    // picked. Hosts that own a GPU context (Plan 8 desktop, future
    // OpenPencil canvas) override this registration with a real
    // `spawn_gpu_init` await before calling `run_stage`. Headless
    // measurement paths (`jian perf startup --dry-run-visual`)
    // leave this no-op in place.
    driver.register(StartupPhase::InitGpuContext, || async move { Ok(()) });
}

fn register_load_core_fonts(driver: &mut StartupDriver) {
    // Plan 19 D2 (font subsetter wiring) lands the real body — until
    // then this records its timing as a no-op so the report still
    // covers every DataPath phase.
    driver.register(StartupPhase::LoadCoreFonts, || async move { Ok(()) });
}

fn register_compute_first_layout(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::ComputeFirstLayout, move || async move {
        let mut rt_cell = shared.runtime.borrow_mut();
        let rt = rt_cell
            .as_mut()
            .ok_or_else(|| "SeedStateGraph produced no runtime".to_owned())?;
        rt.build_layout(shared.viewport)
            .map_err(|e| format!("build_layout: {e}"))?;
        Ok(())
    });
}

fn register_build_visible_spatial(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::BuildVisibleSpatial, move || async move {
        use crate::geometry::rect;
        let mut rt_cell = shared.runtime.borrow_mut();
        let rt = rt_cell
            .as_mut()
            .ok_or_else(|| "SeedStateGraph produced no runtime".to_owned())?;
        let (vw, vh) = shared.viewport;
        let viewport_rect = rect(0.0, 0.0, vw, vh);
        let hidden = rt.rebuild_spatial_for_first_frame(viewport_rect);
        // Drop the runtime borrow before mutating the hidden cell —
        // the background stage will take this via the handle, and we
        // don't want to leave runtime borrowed if a future phase
        // needs &mut access during the same poll.
        drop(rt_cell);
        *shared.hidden_bboxes.borrow_mut() = Some(hidden);
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::{StartupConfig, StartupReport, StartupStage};
    use futures::executor::block_on;

    fn counter_doc() -> &'static str {
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "ct",
          "app": { "name": "ct", "version": "1", "id": "ct" },
          "state": { "count": { "type": "int", "default": 0 } },
          "children": [
            { "type": "frame", "id": "root", "width": 320, "height": 240, "x": 0, "y": 0,
              "children": [
                { "type": "rectangle", "id": "btn",
                  "x": 100, "y": 100, "width": 100, "height": 40,
                  "events": { "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] } }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn data_path_runs_all_eight_phases_against_string_source() {
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let report =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        // Every DataPath phase records.
        let phases: std::collections::HashSet<_> = report.phases.iter().map(|t| t.phase).collect();
        let expected: std::collections::HashSet<_> = StartupPhase::ALL
            .iter()
            .copied()
            .filter(|p| p.stage() == StartupStage::DataPath)
            .collect();
        assert_eq!(phases, expected);
        // Runtime is constructed and laid out.
        let rt = handles.take_runtime().expect("runtime present");
        let count = rt
            .state
            .app_get("count")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(count, 0, "default state seeded");
        assert!(
            rt.layout
                .node_rect(rt.document.as_ref().unwrap().tree.get("btn").unwrap())
                .is_some(),
            "btn has a computed layout rect"
        );
    }

    #[test]
    fn data_path_makes_hidden_bboxes_available_for_background() {
        // Build a doc whose nodes mostly fall OUTSIDE the first-frame
        // viewport so the visible/hidden split is non-empty. Background
        // stage's `BuildFullSpatial` reads this set via
        // `take_hidden_bboxes` to call `SpatialIndex::fill_rest`.
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"long",
          "app":{"name":"long","version":"1","id":"long"},
          "children":[
            { "type":"frame","id":"root","width":320,"height":2400,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"row1","x":0,"y":0,"width":320,"height":40 },
                { "type":"rectangle","id":"row2","x":0,"y":40,"width":320,"height":40 },
                { "type":"rectangle","id":"row99","x":0,"y":2300,"width":320,"height":40 }
              ]
            }
          ]
        }"##;
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(doc.to_owned()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
            .expect("data path ok");
        let hidden = handles
            .take_hidden_bboxes()
            .expect("hidden bbox set populated");
        // row99 is way past the 240-pixel-tall viewport — it must be
        // in the hidden set. row1 / row2 are visible — the root frame
        // itself is bigger than the viewport but its bbox intersects.
        // We only assert the hidden set is non-empty (the precise
        // partition depends on layout, which the test fixture pins
        // approximately).
        assert!(
            !hidden.is_empty(),
            "expected at least one off-viewport node, got 0 hidden bboxes"
        );
        // Calling take a second time yields None — single ownership.
        assert!(handles.take_hidden_bboxes().is_none());
    }

    #[test]
    fn data_path_with_pre_parsed_schema_short_circuits_read_and_parse() {
        let schema: PenDocument = jian_ops_schema::load_str(counter_doc()).unwrap().value;
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::Schema(Box::new(schema)),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let report =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .unwrap();
        // Still records 8 phases — short-circuit doesn't drop any.
        assert_eq!(
            report
                .phases
                .iter()
                .filter(|t| t.phase.stage() == StartupStage::DataPath)
                .count(),
            8
        );
        assert!(handles.take_runtime().is_some());
    }

    #[test]
    fn data_path_with_missing_file_surfaces_phase_failure() {
        let mut driver = StartupDriver::new();
        let _handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::File("/this/path/does/not/exist.op".into()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let err =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect_err("missing file must surface");
        match err {
            crate::startup::driver::StartupError::PhaseFailed { phase, message } => {
                assert_eq!(phase, StartupPhase::ReadFile);
                assert!(
                    message.contains("read") || message.contains("No such"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected PhaseFailed, got {other:?}"),
        }
    }

    #[test]
    fn data_path_with_unparseable_text_surfaces_at_parse_schema() {
        let mut driver = StartupDriver::new();
        let _handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String("not json at all".to_owned()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let err =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect_err("bad source must fail at parse");
        match err {
            crate::startup::driver::StartupError::PhaseFailed { phase, .. } => {
                assert_eq!(phase, StartupPhase::ParseSchema);
            }
            other => panic!("expected PhaseFailed, got {other:?}"),
        }
    }
}
