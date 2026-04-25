//! Public types for the AI Action Surface (Phase 1).
//!
//! Mirrors the data model from `2026-04-24-ai-action-surface.md` §3.
//! These types are pure data + builders — no IO, no MCP plumbing.
//! Production hosts wrap them with the `jian-action-surface` crate
//! (Phase 2) which exposes MCP tools `list_available_actions` /
//! `execute_action`. Until that lands, downstream code can still
//! consume `derive_actions` for previewing or testing.

use serde::{Deserialize, Serialize};

/// Author-visible group of an action. Plain string newtype — spec
/// §3.4 explicitly states "scope is a string, **not** a fixed enum".
/// Three pattern recipes:
///
/// - `Scope::modal(dialog_id)`  → `"modal.<dialog_id>"`
/// - `Scope::page(page_id)`    → `"<page_id>"` (raw page id literal)
/// - `Scope::global()`          → `"global"` (literal)
///
/// UI groupings (B.10 panel) classify by string prefix: anything
/// starting with `"modal."` → Modal; literal `"global"` → Global; else
/// → Page (using the literal as the group key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope(pub String);

impl Scope {
    pub fn modal(dialog_id: &str) -> Self {
        Self(format!("modal.{}", dialog_id))
    }
    pub fn page(page_id: &str) -> Self {
        Self(page_id.to_owned())
    }
    pub fn global() -> Self {
        Self("global".to_owned())
    }

    /// One of `"modal"` / `"global"` / `<page_id>` — used by UI panels
    /// to bucket actions without re-parsing the full string.
    pub fn group(&self) -> &str {
        if self.0.starts_with("modal.") {
            "modal"
        } else if self.0 == "global" {
            "global"
        } else {
            &self.0
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fully-qualified action name (`<scope>.<slug>` with optional
/// `_<hash4>` suffix when `aiName` was *not* used).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionName {
    pub scope: Scope,
    pub slug: String,
}

impl ActionName {
    pub fn full(&self) -> String {
        format!("{}.{}", self.scope.as_str(), self.slug)
    }
}

/// Why this action exists — used for downstream synthesis (the
/// MCP-side `execute_action` builds a `PointerEvent` keyed off
/// `SourceKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Tap,
    DoubleTap,
    LongPress,
    Submit,
    SetValue,
    OpenRoute,
    SwipeLeft,
    SwipeRight,
    SwipeUp,
    SwipeDown,
    Scroll,
    Confirm,
    Dismiss,
}

/// Compile-time-derived availability state. `StateGated` is dynamic
/// (re-evaluated each `execute_action`) and isn't stored here; only
/// the static portion lives in `ActionDefinition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AvailabilityStatic {
    Available,
    /// `semantics.aiHidden = true` — never visible to the agent.
    StaticHidden,
    /// Handler contains a destructive signal (confirm:, fetch DELETE/
    /// POST, storage_clear, storage_wipe). Hidden by default; author
    /// can flip with `aiHidden: false`.
    ConfirmGated,
}

/// Parameter declaration for `set_<slug>(value)` and `open_<slug>(p)`.
/// Phase 1 only carries a coarse JSON-Schema-friendly type tag — the
/// MCP server (Phase 2) maps it to the wire-level JsonSchema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: String,
    pub ty: ParamTy,
}

/// Atomic parameter types. Mirrors the subset of `state::StateType`
/// the surface emits in Phase 1 — no `oneOf` / nested object/array
/// expansion until Phase 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamTy {
    Int,
    Float,
    Number,
    String,
    Bool,
    Date,
    /// Type couldn't be statically inferred (e.g. expression depended
    /// on runtime state). Runtime accepts any JSON value and validates
    /// at execute-time.
    Unknown,
}

/// One entry in the action surface. Pure data — `derive_actions`
/// returns a deterministic `Vec<ActionDefinition>` for any given
/// `(document, build_salt)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub name: ActionName,
    /// Source PenNode id — agents never see this; runtime uses it
    /// internally to dispatch the synthesised event.
    pub source_node_id: String,
    pub source_kind: SourceKind,
    /// Author-supplied `aiDescription` if present, else an
    /// auto-generated short blurb (see `derive`).
    pub description: String,
    /// Static availability decided at derive time.
    pub status: AvailabilityStatic,
    /// Historical names still accepted by `execute_action` for
    /// transparent migration after a rename. See spec §9.
    pub aliases: Vec<ActionName>,
    /// Declared parameters (empty for verb-style actions like Tap).
    /// `set_<slug>` carries `(value: <state-type>)`; `open_<slug>`
    /// carries one entry per `:param` placeholder in the route path.
    pub params: Vec<ParamSpec>,
}

impl ActionDefinition {
    pub fn full_name(&self) -> String {
        self.name.full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_grouping() {
        assert_eq!(Scope::modal("checkout").group(), "modal");
        assert_eq!(Scope::page("home").group(), "home");
        assert_eq!(Scope::global().group(), "global");
    }

    #[test]
    fn scope_string_form() {
        assert_eq!(Scope::modal("checkout").as_str(), "modal.checkout");
        assert_eq!(Scope::page("home").as_str(), "home");
        assert_eq!(Scope::global().as_str(), "global");
    }

    #[test]
    fn action_full_name() {
        let n = ActionName {
            scope: Scope::page("home"),
            slug: "sign_in_a3f7".into(),
        };
        assert_eq!(n.full(), "home.sign_in_a3f7");
    }
}
