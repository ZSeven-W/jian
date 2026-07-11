use super::base::PenNodeBase;
use super::container::CornerRadius;
use crate::sizing::SizingBehavior;
use crate::state_override::WidgetStates;
use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

/// Single-line text input. Forms / counters need a writable input
/// surface that two-way binds via `bindings.bind:value`. The walker
/// renders a styled rectangle + caret placeholder; full IME and
/// selection-painter wiring lands in the desktop host (Plan 8) once
/// the gesture arena gains `Focus` recognizers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct TextInputNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(flatten)]
    pub limits: crate::sizing::SizeLimits,
    /// Placeholder shown when `value` is empty. Static text — author
    /// `bindings.placeholder` if it needs to react to state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Initial value. Two-way binding lives on `bindings.bind:value`,
    /// which derive lifts into a `set_*` action and the runtime keeps
    /// in sync with the state graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Lucide glyph drawn at the left content edge (e.g. `mail`, `lock`).
    /// The painter insets the text/caret past it so the whole box stays
    /// one interactive node. `None` = no leading icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading_icon: Option<String>,
    /// Lucide glyph drawn at the right content edge (e.g. `eye` for a
    /// password reveal). Decorative in Phase 1 (no toggle behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<CornerRadius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub states: Option<WidgetStates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}
