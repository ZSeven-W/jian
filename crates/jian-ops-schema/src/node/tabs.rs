use super::base::PenNodeBase;
use super::container::CornerRadius;
use super::select::SelectOption;
use crate::sizing::SizingBehavior;
use crate::state_override::WidgetStates;
use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

/// Tabbed panel switcher. Unlike the leaf widgets this is a CONTAINER:
/// `children[i]` is the panel for `tabs[i]`. The active tab `value`
/// two-way binds via `bindings.bind:value`; only the active panel is
/// painted at runtime.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct TabsNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    /// Tab bar entries; `value` keys the active tab, `label` is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<SelectOption>>,
    /// Currently active tab `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Panel subtrees, one per tab (parallel to `tabs` by index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
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
