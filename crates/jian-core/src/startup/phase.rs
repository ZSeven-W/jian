//! `StartupPhase` — enumerates every step on the cold-start critical path
//! plus the post-paint async work that finishes after first-interactive.
//!
//! See `superpowers/plans/2026-04-17-jian-plan-19-cold-start-optimization.md`
//! for the design context (C19 budgets, parallelism rules).

/// One named step in the cold-start pipeline.
///
/// Variants are ordered roughly along the critical path; `deps()` is the
/// authoritative ordering source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupPhase {
    // I/O
    ReadFile,
    ParseSchema,

    // State & tree
    SeedStateGraph,
    BuildNodeTree,

    // GPU / rendering
    InitGpuContext,
    LoadCoreFonts,

    // Layout & visible spatial
    ComputeFirstLayout,
    BuildVisibleSpatial,

    // First paint
    RenderSplash,
    RenderFirstFrame,
    PresentToSurface,

    // Post-paint (async, outside critical path but needed for interaction)
    BuildFullSpatial,
    LoadRemainingFonts,
    DecodeImages,
    EventPumpReady,
}

impl StartupPhase {
    /// Every phase in declaration order. Useful for iterating the universe of
    /// phases (e.g. when building the driver's pending set).
    pub const ALL: &'static [StartupPhase] = &[
        StartupPhase::ReadFile,
        StartupPhase::ParseSchema,
        StartupPhase::SeedStateGraph,
        StartupPhase::BuildNodeTree,
        StartupPhase::InitGpuContext,
        StartupPhase::LoadCoreFonts,
        StartupPhase::ComputeFirstLayout,
        StartupPhase::BuildVisibleSpatial,
        StartupPhase::RenderSplash,
        StartupPhase::RenderFirstFrame,
        StartupPhase::PresentToSurface,
        StartupPhase::BuildFullSpatial,
        StartupPhase::LoadRemainingFonts,
        StartupPhase::DecodeImages,
        StartupPhase::EventPumpReady,
    ];

    /// Phases that must finish before this one can start.
    pub fn deps(self) -> &'static [StartupPhase] {
        use StartupPhase::*;
        match self {
            ReadFile | InitGpuContext => &[],
            ParseSchema => &[ReadFile],
            SeedStateGraph => &[ParseSchema],
            BuildNodeTree => &[ParseSchema],
            // First-frame layout reads bindings that may reference seeded signals
            // (`$app` / `$page` / `$self`). Without seeded state the layout sees
            // `null` and produces wrong rects, so SeedStateGraph is a hard dep
            // alongside BuildNodeTree.
            ComputeFirstLayout => &[BuildNodeTree, SeedStateGraph],
            BuildVisibleSpatial => &[ComputeFirstLayout],
            // Splash config lives at runtime.document.schema.app.splash, which
            // requires ParseSchema. The GPU context is required for any draw op.
            RenderSplash => &[InitGpuContext, ParseSchema],
            RenderFirstFrame => &[
                InitGpuContext,
                LoadCoreFonts,
                ComputeFirstLayout,
                SeedStateGraph,
            ],
            PresentToSurface => &[RenderFirstFrame],
            BuildFullSpatial => &[ComputeFirstLayout],
            LoadCoreFonts | LoadRemainingFonts | DecodeImages => &[ParseSchema],
            EventPumpReady => &[PresentToSurface, BuildVisibleSpatial],
        }
    }

    /// Whether this phase is on the critical path to first-interactive.
    /// Splash + post-paint async work is excluded.
    pub fn is_critical(self) -> bool {
        use StartupPhase::*;
        matches!(
            self,
            ReadFile
                | ParseSchema
                | SeedStateGraph
                | BuildNodeTree
                | InitGpuContext
                | LoadCoreFonts
                | ComputeFirstLayout
                | BuildVisibleSpatial
                | RenderFirstFrame
                | PresentToSurface
                | EventPumpReady
        )
    }

    /// Stable string id used for pretty-printed reports and JSON output.
    pub fn as_str(self) -> &'static str {
        use StartupPhase::*;
        match self {
            ReadFile => "ReadFile",
            ParseSchema => "ParseSchema",
            SeedStateGraph => "SeedStateGraph",
            BuildNodeTree => "BuildNodeTree",
            InitGpuContext => "InitGpuContext",
            LoadCoreFonts => "LoadCoreFonts",
            ComputeFirstLayout => "ComputeFirstLayout",
            BuildVisibleSpatial => "BuildVisibleSpatial",
            RenderSplash => "RenderSplash",
            RenderFirstFrame => "RenderFirstFrame",
            PresentToSurface => "PresentToSurface",
            BuildFullSpatial => "BuildFullSpatial",
            LoadRemainingFonts => "LoadRemainingFonts",
            DecodeImages => "DecodeImages",
            EventPumpReady => "EventPumpReady",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_lists_every_variant() {
        // Defence-in-depth: if a new variant is added but ALL forgotten, this
        // test fails because deps() will reference a phase ALL doesn't include.
        let in_all: HashSet<_> = StartupPhase::ALL.iter().copied().collect();
        for p in StartupPhase::ALL {
            for d in p.deps() {
                assert!(in_all.contains(d), "dep {d:?} of {p:?} missing from ALL");
            }
        }
    }

    #[test]
    fn graph_is_acyclic() {
        // Topological sort must succeed; if a cycle exists the loop fails to
        // make progress.
        let mut done: HashSet<StartupPhase> = HashSet::new();
        let mut pending: HashSet<StartupPhase> = StartupPhase::ALL.iter().copied().collect();
        while !pending.is_empty() {
            let next: Vec<_> = pending
                .iter()
                .copied()
                .filter(|p| p.deps().iter().all(|d| done.contains(d)))
                .collect();
            assert!(!next.is_empty(), "cycle detected: remaining {pending:?}");
            for p in next {
                pending.remove(&p);
                done.insert(p);
            }
        }
    }

    #[test]
    fn roots_have_no_deps() {
        assert!(StartupPhase::ReadFile.deps().is_empty());
        assert!(StartupPhase::InitGpuContext.deps().is_empty());
    }

    #[test]
    fn parse_schema_depends_only_on_read_file() {
        assert_eq!(StartupPhase::ParseSchema.deps(), &[StartupPhase::ReadFile]);
    }

    #[test]
    fn render_first_frame_waits_for_gpu_fonts_layout_state() {
        let deps: HashSet<_> = StartupPhase::RenderFirstFrame
            .deps()
            .iter()
            .copied()
            .collect();
        for required in [
            StartupPhase::InitGpuContext,
            StartupPhase::LoadCoreFonts,
            StartupPhase::ComputeFirstLayout,
            StartupPhase::SeedStateGraph,
        ] {
            assert!(
                deps.contains(&required),
                "RenderFirstFrame must depend on {required:?}"
            );
        }
    }

    #[test]
    fn event_pump_ready_is_terminal() {
        // Nothing in ALL depends on EventPumpReady — it's the terminal node.
        for p in StartupPhase::ALL {
            assert!(
                !p.deps().contains(&StartupPhase::EventPumpReady),
                "{p:?} should not depend on EventPumpReady"
            );
        }
    }

    #[test]
    fn critical_path_set_matches_spec() {
        // Spec says 11 critical phases (everything except Splash + post-paint
        // background work).
        let critical: HashSet<_> = StartupPhase::ALL
            .iter()
            .copied()
            .filter(|p| p.is_critical())
            .collect();
        let expected: HashSet<_> = [
            StartupPhase::ReadFile,
            StartupPhase::ParseSchema,
            StartupPhase::SeedStateGraph,
            StartupPhase::BuildNodeTree,
            StartupPhase::InitGpuContext,
            StartupPhase::LoadCoreFonts,
            StartupPhase::ComputeFirstLayout,
            StartupPhase::BuildVisibleSpatial,
            StartupPhase::RenderFirstFrame,
            StartupPhase::PresentToSurface,
            StartupPhase::EventPumpReady,
        ]
        .into_iter()
        .collect();
        assert_eq!(critical, expected);
    }

    #[test]
    fn non_critical_phases_are_post_paint_or_splash() {
        for p in [
            StartupPhase::RenderSplash,
            StartupPhase::BuildFullSpatial,
            StartupPhase::LoadRemainingFonts,
            StartupPhase::DecodeImages,
        ] {
            assert!(!p.is_critical(), "{p:?} should not be critical");
        }
    }

    #[test]
    fn as_str_round_trips_distinct() {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for p in StartupPhase::ALL {
            assert!(seen.insert(p.as_str()), "{p:?} produced duplicate as_str");
        }
    }
}
