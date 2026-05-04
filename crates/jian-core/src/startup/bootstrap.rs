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
//! | `LoadCoreFonts`       | first-frame `FontPlan::scan_subtrees` (Plan 19 D2)   |
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
use jian_ops_schema::font_plan::FontPlan;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::pack::expressions::ExpressionsSnapshot;
use jian_ops_schema::pack::initial_layout::InitialLayoutSnapshot;
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

    /// Take the [`FontPlan`] the `LoadCoreFonts` phase scanned out of
    /// the schema's first-frame subtree. The host's font provider
    /// can iterate `plan.families()` and request a per-family
    /// codepoint subset for first-paint, then schedule the remaining
    /// glyphs from the Background stage's `LoadRemainingFonts` phase.
    ///
    /// Returns `None` if `LoadCoreFonts` hasn't run yet, or if
    /// `take_core_font_plan` was already called. Empty plan (no text
    /// nodes anywhere on the first frame) is still `Some` — `is_empty`
    /// on the plan distinguishes "ran but found nothing" from "didn't
    /// run".
    ///
    /// First-frame heuristic: when the doc declares explicit `pages`,
    /// scan the first page; otherwise scan every root child. This
    /// mirrors the same heuristic the player / perf / budget tests use
    /// for first-frame root selection so the plan covers what the
    /// renderer is about to paint. (Codex review of D2: a viewport-
    /// aware scan would need a layout-pass dep that defeats the
    /// LoadCoreFonts ⫶ SeedStateGraph parallelism — Plan 19 §C19
    /// explicitly accepts the wider page-scan trade-off here.)
    pub fn take_core_font_plan(&self) -> Option<FontPlan> {
        self.shared.core_font_plan.borrow_mut().take()
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
    /// Filled by `LoadCoreFonts` — the per-family codepoint plan a
    /// host's font provider uses to request first-paint subsets via
    /// [`BootstrapHandles::take_core_font_plan`]. `None` until the
    /// phase runs.
    core_font_plan: RefCell<Option<FontPlan>>,
    /// Caller-supplied first-frame viewport, in logical pixels.
    viewport: (f32, f32),
    /// Plan 19 D1 cold-start: optional pre-computed initial layout
    /// from `aot/initial_layout.bin`. When `Some`, `SeedStateGraph`
    /// preloads it into the runtime and `ComputeFirstLayout` falls
    /// through to a no-op — saving the taffy compute on first frame.
    /// A subsequent resize-driven `build_layout` clears the preload
    /// and runs a fresh compute, so resize correctness is preserved.
    aot_initial_layout: Option<InitialLayoutSnapshot>,
    /// Plan 19 D2 cold-start: optional pre-compiled-expression
    /// snapshot from `aot/expressions.bin`. When `Some`,
    /// `SeedStateGraph` installs the chunks into the runtime's
    /// expression cache *immediately after* `Runtime::
    /// new_from_document` constructs the cache. A binding's first
    /// `get_or_compile(source)` then returns the pre-compiled
    /// chunk and skips parse + compile. The seed is opportunistic:
    /// sources missing from the snapshot fall through to JIT
    /// compile, exactly as without `--aot`.
    aot_expressions: Option<ExpressionsSnapshot>,
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
        Self::install_data_path_with_aot(driver, source, viewport, None)
    }

    /// Plan 19 D1 cold-start variant: same as
    /// [`install_data_path`](Self::install_data_path) but accepts an
    /// optional `aot/initial_layout.bin` snapshot. When `Some`, the
    /// `SeedStateGraph` phase preloads it into the runtime and
    /// `ComputeFirstLayout` short-circuits to a no-op — the
    /// host-supplied viewport at the snapshot's authored size sees
    /// pre-baked rects on first paint.
    ///
    /// Hosts that don't ship a `.op.pack` (or whose pack omits the
    /// AOT entry) call `install_data_path` and pay the regular
    /// `ComputeFirstLayout` cost. For the Plan 19 D2 expressions
    /// preload as well, use [`Self::install_data_path_with_aot_full`].
    pub fn install_data_path_with_aot(
        driver: &mut StartupDriver,
        source: BootstrapSource,
        viewport: (f32, f32),
        aot_initial_layout: Option<InitialLayoutSnapshot>,
    ) -> BootstrapHandles {
        Self::install_data_path_with_aot_full(driver, source, viewport, aot_initial_layout, None)
    }

    /// Plan 19 D1 + D2 cold-start variant: accepts both an optional
    /// initial-layout snapshot AND an optional pre-compiled-
    /// expressions snapshot. When `aot_expressions` is `Some`, the
    /// `SeedStateGraph` phase installs the chunks into the
    /// runtime's `ExpressionCache` immediately after construction,
    /// so binding evaluation hits pre-compiled bytecode without
    /// paying parse + compile.
    pub fn install_data_path_with_aot_full(
        driver: &mut StartupDriver,
        source: BootstrapSource,
        viewport: (f32, f32),
        aot_initial_layout: Option<InitialLayoutSnapshot>,
        aot_expressions: Option<ExpressionsSnapshot>,
    ) -> BootstrapHandles {
        let shared = Rc::new(BootstrapShared {
            source_text: RefCell::new(None),
            schema: RefCell::new(None),
            runtime: RefCell::new(None),
            hidden_bboxes: RefCell::new(None),
            core_font_plan: RefCell::new(None),
            viewport,
            source,
            aot_initial_layout,
            aot_expressions,
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
        register_load_core_fonts(driver, &shared);
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
        // Clone (was `take`) so the schema cell stays populated for
        // `LoadCoreFonts`, which depends on `ParseSchema` and runs in
        // parallel with this phase under the dep graph (codex review
        // of D2: a `.take()` here races the font-plan scan and leaves
        // it without a schema in 50% of poll orders). The clone is
        // bounded by document size and avoids draining the schema for
        // parallel phases.
        let schema = shared
            .schema
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| "ParseSchema produced no schema".to_owned())?;
        let mut runtime = Runtime::new_from_document(schema)
            .map_err(|e| format!("Runtime::new_from_document: {e}"))?;
        // Plan 19 D2 cold-start: install the AOT pre-compiled-
        // expression snapshot before any binding evaluates. The
        // cache is empty at this point (`Runtime::new_from_document`
        // just constructed it), so every snapshot entry seeds a slot
        // and the first `BindingEffect::new_lazy` for a seeded source
        // returns the pre-compiled chunk. The `&` borrow keeps the
        // snapshot in `BootstrapShared` for the host (diagnostics /
        // replay); the helper clones each PackedChunk into the
        // cache's owned `Chunk` shape.
        //
        // Codex review BLOCK: a structurally-valid `aot/expressions
        // .bin` could still carry VM-unsafe bytecode (out-of-range
        // indices, backwards jumps that infinite-loop). Run the
        // structural verifier before installing; on failure, drop
        // the whole snapshot (no per-entry partial-install) and
        // emit a stderr warning so the host operator sees the
        // tampering signal. The runtime then JIT-compiles every
        // binding source on first eval, exactly as without `--aot`.
        if let Some(snap) = shared.aot_expressions.as_ref() {
            match snap.verify_all() {
                Ok(()) => {
                    runtime
                        .expr_cache
                        .install_precompiled(crate::expression::snapshot_to_chunks(snap));
                }
                Err((source, err)) => {
                    eprintln!(
                        "jian: warning — aot/expressions.bin entry for `{source}` failed \
                         structural verify ({err}); falling back to JIT compile for every \
                         binding"
                    );
                }
            }
        }
        // Plan 19 D1 cold-start: preload the AOT initial-layout
        // snapshot now so `ComputeFirstLayout` can short-circuit
        // without racing a future host-side mutation. We feed
        // `&InitialLayoutSnapshot` so the snapshot stays in the
        // shared cell for the host (e.g. for diagnostics / replay).
        //
        // Codex round 2 MEDIUM: only preload when the snapshot was
        // baked at the bootstrap's actual first-frame viewport. A
        // snapshot authored at 800×600 fed into a 320×240 bootstrap
        // would otherwise skip compute and feed mis-scaled rects
        // into `BuildVisibleSpatial`. The match uses an f32-bit-
        // exact comparison because both ends originate from the same
        // `--size` / pack-manifest source — drift would indicate a
        // bug, not legitimate flexibility, and the surface contract
        // only promises the snapshot for the authored viewport.
        if let Some(snap) = shared.aot_initial_layout.as_ref() {
            let (vw, vh) = shared.viewport;
            if snap.viewport.width == vw && snap.viewport.height == vh {
                let _ = runtime.preload_initial_layout(snap);
            }
        }
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

fn register_load_core_fonts(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    // Scan the first-frame subtree for the per-family codepoints the
    // first paint will need. Stores a `FontPlan` in
    // `BootstrapShared::core_font_plan` for the host to take via
    // `BootstrapHandles::take_core_font_plan` and feed into its
    // platform-specific font loader. The actual font I/O / subsetting
    // happens host-side (skia-safe + ttf-parser on desktop, CanvasKit
    // on web, etc.) — jian-core stays host-agnostic per Plan 19's
    // separation of concerns, the same pattern used for
    // `InitGpuContext`. Plan 19 §C19 D2.
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::LoadCoreFonts, move || async move {
        let schema_ref = shared.schema.borrow();
        let schema = schema_ref
            .as_ref()
            .ok_or_else(|| "ParseSchema produced no schema".to_owned())?;
        let plan = scan_first_frame_font_plan(schema);
        // End the schema borrow before publishing the plan. The next
        // write is to a different `RefCell` (`core_font_plan`), so
        // this isn't strictly required for borrow-rule correctness —
        // it's tidy hygiene that scopes the read narrowly.
        drop(schema_ref);
        *shared.core_font_plan.borrow_mut() = Some(plan);
        Ok(())
    });
}

/// First-frame font-plan scan: when the doc declares explicit `pages`,
/// scan the first page's children; otherwise scan every root child.
/// Mirrors the same first-frame-root heuristic the player / perf /
/// budget tests use, so the plan covers the first page / root set the
/// runtime seeds — and may over-include off-viewport text until layout
/// exists. A truly viewport-aware scan would need layout output, but
/// `LoadCoreFonts` runs in parallel with `ComputeFirstLayout` under
/// the Plan 19 dep graph; the page-level approximation is the
/// price of that parallelism (Plan 19 §C19 D2 explicitly accepts it).
fn scan_first_frame_font_plan(schema: &PenDocument) -> FontPlan {
    let roots: &[PenNode] = match (&schema.pages, &schema.children) {
        (Some(pages), _) if !pages.is_empty() => &pages[0].children,
        _ => schema.children.as_slice(),
    };
    FontPlan::scan_subtrees(roots.iter())
}

fn register_compute_first_layout(driver: &mut StartupDriver, shared: &Rc<BootstrapShared>) {
    let shared = Rc::clone(shared);
    driver.register(StartupPhase::ComputeFirstLayout, move || async move {
        let mut rt_cell = shared.runtime.borrow_mut();
        let rt = rt_cell
            .as_mut()
            .ok_or_else(|| "SeedStateGraph produced no runtime".to_owned())?;
        // Plan 19 D1 cold-start: when the bootstrap source carried a
        // `aot/initial_layout.bin` and `SeedStateGraph` preloaded
        // it, the runtime already has authoritative first-frame rects
        // FOR THE SNAPSHOT'S DOCUMENT. Only skip the taffy compute
        // when the preload covers every node in the active doc — a
        // partial preload (older pack + newer doc) paired with a
        // skip would leave the new nodes rect-less in
        // `BuildVisibleSpatial` (codex round 1 MEDIUM). Coverage
        // mismatch falls through to a real compute and the partial
        // preload is dropped first so it can't poison the result.
        if rt.layout.has_preload() {
            let covers = rt
                .document
                .as_ref()
                .map(|doc| rt.layout.preload_covers(&doc.tree))
                .unwrap_or(false);
            if covers {
                return Ok(());
            }
            rt.layout.drop_preload();
        }
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

    // ──────────────────────────────────────────────────────────────
    // Plan 19 D2 — font plan exposed via BootstrapHandles
    // ──────────────────────────────────────────────────────────────

    fn doc_with_text(content: &str) -> String {
        format!(
            r##"{{
              "formatVersion": "1.0", "version": "1.0.0", "id": "ft",
              "app": {{ "name": "ft", "version": "1", "id": "ft" }},
              "children": [
                {{ "type": "frame", "id": "root", "width": 320, "height": 240, "x": 0, "y": 0,
                  "children": [
                    {{ "type": "text", "id": "t1", "fontFamily": "Inter",
                       "x": 10, "y": 10, "content": {content:?} }}
                  ]
                }}
              ]
            }}"##
        )
    }

    #[test]
    fn load_core_fonts_populates_per_family_codepoints() {
        // The runtime side of Plan 19 §C19 D2: after `LoadCoreFonts`
        // runs, the host can take a `FontPlan` mapping `Inter` to the
        // codepoints in the first-frame's text content. Hosts then
        // request a per-family subset before first paint.
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(doc_with_text("Hi! 你好")),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let _report =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        let plan = handles
            .take_core_font_plan()
            .expect("LoadCoreFonts populated plan");
        let inter = plan.for_family("Inter").expect("Inter family scanned");
        // ASCII + CJK codepoints — the scan walks `Plain(String)`
        // content under the text node's `font_family`.
        assert!(inter.codepoints.contains(&u32::from('H')));
        assert!(inter.codepoints.contains(&u32::from('i')));
        // Mandarin "你" / "好" both appear.
        assert!(inter.codepoints.contains(&u32::from('你')));
        assert!(inter.codepoints.contains(&u32::from('好')));
    }

    #[test]
    fn load_core_fonts_returns_empty_plan_when_no_text() {
        // counter_doc has no text nodes — the plan still exists but
        // is empty. This distinguishes "phase ran, no glyphs needed"
        // from "phase didn't run" (which returns `None`).
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let _report =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        let plan = handles.take_core_font_plan().expect("LoadCoreFonts ran");
        assert!(plan.is_empty(), "no text nodes -> empty plan");
    }

    #[test]
    fn take_core_font_plan_is_idempotent_take_returns_none_second_call() {
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(doc_with_text("X")),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let _ =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        assert!(handles.take_core_font_plan().is_some());
        // Second take drains nothing (mirror of `take_runtime` /
        // `take_hidden_bboxes` semantics — Box::take returns None).
        assert!(handles.take_core_font_plan().is_none());
    }

    #[test]
    fn take_core_font_plan_is_none_before_run_stage() {
        // Codex round 1 MEDIUM: explicit guard for the "phase didn't
        // run yet" path. Calling `take_core_font_plan` on a freshly
        // installed bootstrap (no `run_stage` yet) must return `None`
        // — distinct from "ran, no glyphs" which returns `Some(empty)`.
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(doc_with_text("X")),
            (320.0, 240.0),
        );
        // Don't drive the driver — the LoadCoreFonts phase never fires.
        let _ = driver;
        assert!(
            handles.take_core_font_plan().is_none(),
            "no run_stage → no plan"
        );
    }

    fn two_page_doc() -> String {
        // First page uses `Inter`; second page uses `Roboto`. The
        // first-frame heuristic should see only `Inter` after running.
        // Root `children` is required by the schema even when pages
        // are set; it stays empty so the page-branch is what drives
        // the scan.
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "tp",
          "app": { "name": "tp", "version": "1", "id": "tp" },
          "pages": [
            { "id": "p1", "name": "Page 1", "children": [
              { "type": "text", "id": "t_p1", "fontFamily": "Inter",
                "x": 0, "y": 0, "content": "First" }
            ]},
            { "id": "p2", "name": "Page 2", "children": [
              { "type": "text", "id": "t_p2", "fontFamily": "Roboto",
                "x": 0, "y": 0, "content": "Second" }
            ]}
          ],
          "children": []
        }"##
        .to_owned()
    }

    #[test]
    fn first_frame_scan_only_covers_first_page_when_pages_declared() {
        // Codex round 1 MEDIUM: a multi-page doc tests the
        // `pages[0].children` branch. The font plan must only include
        // families used on the first page; second-page-only families
        // are deferred to LoadRemainingFonts (host-side, future).
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(two_page_doc()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let _ =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        let plan = handles.take_core_font_plan().expect("LoadCoreFonts ran");
        assert!(
            plan.for_family("Inter").is_some(),
            "first page family present"
        );
        assert!(
            plan.for_family("Roboto").is_none(),
            "second-page-only family must not appear in the first-frame plan"
        );
    }

    #[test]
    fn empty_pages_list_falls_back_to_root_children() {
        // Codex round 1 MEDIUM: the `(Some(pages), _) if !pages.is_empty()`
        // guard means an empty `pages: []` falls through to scanning
        // root `children`. Without the guard a doc declaring
        // `"pages": []` alongside root `children` would produce an
        // empty plan even though the renderer would paint the root
        // children. Not a realistic doc shape, but the guard exists
        // explicitly so the test pins it.
        let body = r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "ep",
          "app": { "name": "ep", "version": "1", "id": "ep" },
          "pages": [],
          "children": [
            { "type": "text", "id": "t1", "fontFamily": "Inter",
              "x": 0, "y": 0, "content": "Hello" }
          ]
        }"##;
        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::String(body.to_owned()),
            (320.0, 240.0),
        );
        let prior = StartupReport::default();
        let _ =
            block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
                .expect("data path run ok");
        let plan = handles.take_core_font_plan().expect("LoadCoreFonts ran");
        assert!(
            plan.for_family("Inter").is_some(),
            "empty pages list must fall back to root children, not produce empty plan"
        );
    }

    // ──────────────────────────────────────────────────────────────
    // Plan 19 D1 — AOT initial-layout preload bypasses ComputeFirstLayout
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn aot_initial_layout_preload_bypasses_compute_first_layout() {
        // Snapshot baked for both nodes in counter_doc (`root` frame
        // and `btn` rectangle). Authored rect for `btn` differs from
        // what taffy compute would produce — after bootstrap runs,
        // the runtime's `node_rect` must serve the snapshot rect,
        // proving ComputeFirstLayout was skipped.
        use jian_ops_schema::pack::initial_layout::{InitialLayoutSnapshot, PackedRect};
        use jian_ops_schema::pack::manifest::DefaultViewport;
        use std::collections::BTreeMap;

        let mut rects = BTreeMap::new();
        rects.insert(
            "root".to_owned(),
            PackedRect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 240.0,
            },
        );
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 7.0,
                y: 8.0,
                w: 9.0,
                h: 10.0,
            },
        );
        let snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };

        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path_with_aot(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0),
            Some(snap),
        );
        let prior = StartupReport::default();
        block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
            .expect("data path run ok");
        let rt = handles.take_runtime().expect("runtime present");
        let key = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
        let r = rt
            .layout
            .node_rect(key)
            .expect("preload-served rect available");
        // The taffy compute output for counter_doc places `btn` at
        // (100, 100, 100, 40); the snapshot we supplied placed it at
        // (7, 8, 9, 10). If ComputeFirstLayout had run, taffy would
        // have replaced the preload with its own rect.
        assert_eq!(r.origin.x, 7.0, "preload rect.x served, not compute output");
        assert_eq!(r.origin.y, 8.0);
        assert_eq!(r.size.width, 9.0);
        assert_eq!(r.size.height, 10.0);
        // And the preload cache is still live.
        assert!(rt.layout.has_preload());
    }

    #[test]
    fn aot_viewport_mismatch_falls_through_to_compute() {
        // Codex round 2 MEDIUM: snapshot authored at 800×600 fed into
        // a 320×240 bootstrap would feed mis-scaled rects into the
        // first-frame spatial / render paths if the preload engaged.
        // The bootstrap must skip the preload entirely when viewports
        // disagree.
        use jian_ops_schema::pack::initial_layout::{InitialLayoutSnapshot, PackedRect};
        use jian_ops_schema::pack::manifest::DefaultViewport;
        use std::collections::BTreeMap;

        let mut rects = BTreeMap::new();
        for id in &["root", "btn"] {
            rects.insert(
                (*id).to_owned(),
                PackedRect {
                    x: 999.0,
                    y: 999.0,
                    w: 1.0,
                    h: 1.0,
                },
            );
        }
        let snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 800.0,
                height: 600.0,
            },
            rects,
        };

        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path_with_aot(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0), // mismatched on purpose
            Some(snap),
        );
        let prior = StartupReport::default();
        block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
            .expect("mismatch falls through to compute");
        let rt = handles.take_runtime().expect("runtime present");
        // Preload was never installed — ComputeFirstLayout ran.
        assert!(
            !rt.layout.has_preload(),
            "viewport mismatch must skip preload"
        );
        let btn_key = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
        let r = rt.layout.node_rect(btn_key).expect("compute rect");
        // The (999, 999) sentinel from the rejected snapshot is gone.
        assert!(r.origin.x < 500.0 && r.origin.y < 500.0);
    }

    #[test]
    fn aot_partial_preload_falls_through_to_compute() {
        // Codex round 1 MEDIUM: an older pack carrying only `btn`
        // paired with a doc whose `root` frame must also have a rect
        // would have silently skipped compute and left `root` rect-
        // less. The bootstrap must drop the partial preload and run
        // a real `build_layout` so every doc node lands in the
        // spatial index.
        use jian_ops_schema::pack::initial_layout::{InitialLayoutSnapshot, PackedRect};
        use jian_ops_schema::pack::manifest::DefaultViewport;
        use std::collections::BTreeMap;

        let mut rects = BTreeMap::new();
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 7.0,
                y: 8.0,
                w: 9.0,
                h: 10.0,
            },
        );
        let snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };

        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path_with_aot(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0),
            Some(snap),
        );
        let prior = StartupReport::default();
        block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
            .expect("partial-preload bootstrap still completes");
        let rt = handles.take_runtime().expect("runtime present");
        // Preload was dropped; taffy compute supplied the rects.
        assert!(!rt.layout.has_preload(), "partial preload must be dropped");
        let root_key = rt.document.as_ref().unwrap().tree.get("root").unwrap();
        let btn_key = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
        // Both nodes have rects (real compute populated taffy).
        assert!(
            rt.layout.node_rect(root_key).is_some(),
            "root has compute rect"
        );
        let r = rt.layout.node_rect(btn_key).expect("btn has compute rect");
        // The (7, 8, 9, 10) sentinel from the partial preload is
        // gone; taffy's real rect dominates.
        assert!(r.origin.x != 7.0 || r.origin.y != 8.0);
    }

    #[test]
    fn aot_preload_does_not_block_resize_relayout() {
        // After the cold-start preload, a subsequent
        // `Runtime::build_layout` (i.e. host-driven resize) must
        // clear the preload and produce a fresh taffy compute. Without
        // this, the first resize would keep stale AOT rects forever.
        use jian_ops_schema::pack::initial_layout::{InitialLayoutSnapshot, PackedRect};
        use jian_ops_schema::pack::manifest::DefaultViewport;
        use std::collections::BTreeMap;

        let mut rects = BTreeMap::new();
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 999.0,
                y: 999.0,
                w: 1.0,
                h: 1.0,
            },
        );
        let snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };

        let mut driver = StartupDriver::new();
        let handles = HostAgnosticBootstrap::install_data_path_with_aot(
            &mut driver,
            BootstrapSource::String(counter_doc().to_owned()),
            (320.0, 240.0),
            Some(snap),
        );
        let prior = StartupReport::default();
        block_on(driver.run_stage(StartupStage::DataPath, &prior, StartupConfig::default()))
            .unwrap();
        let mut rt = handles.take_runtime().unwrap();
        // Simulate a host-driven resize.
        rt.build_layout((320.0, 240.0)).expect("relayout ok");
        assert!(!rt.layout.has_preload(), "resize clears AOT preload");
        let key = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
        let r = rt.layout.node_rect(key).unwrap();
        // Sanity: post-resize rect is from taffy, not the (999,999)
        // sentinel snapshot.
        assert!(r.origin.x < 500.0 && r.origin.y < 500.0);
    }
}
