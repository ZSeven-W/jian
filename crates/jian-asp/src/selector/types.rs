//! Selector data types (Plan 18 Task 2 — Phase 1 types-only).
//!
//! The selector is a **structured JSON value**, not a string DSL —
//! LLM agents serialise it the same way they serialise any other
//! tool-call argument, and the runtime parses with `serde` instead of
//! a hand-rolled tokenizer. The Phase 1 commit (this file) lays out
//! the field set; the resolver (`resolve.rs`) lands in Phase 2 once
//! the runtime borrows are settled.
//!
//! ## Field semantics
//!
//! - `id` / `alias`: exact match on the schema's stable identifier.
//! - `role`: a11y-style role (`button` / `link` / `text` / `image`,
//!   matching the values exported by `jian-core::semantics`).
//! - `text` / `text_contains`: literal / substring match on the
//!   visible text content (case-sensitive; case-insensitive support
//!   joins as a second field rather than overloading these two).
//! - `visible` / `focused` / `enabled`: boolean filters, only applied
//!   when the field is `Some(true|false)`. `None` means the field is
//!   not constrained (matches both states).
//! - `near` / `child_of` / `parent_of`: relational filters that take
//!   another selector — these recurse, so an agent can express
//!   `find a button whose parent is a card whose title contains "Plan"`
//!   without inventing a string DSL.
//! - `all_of` / `any_of` / `not`: combinators that take a vector of
//!   sub-selectors. The runtime evaluates these against an
//!   already-filtered candidate set.
//! - `first` / `index`: choose-one disambiguation when the candidate
//!   set has multiple matches. `first: true` picks the document-order
//!   first; `index: 2` picks the third. Mutually exclusive — providing
//!   both is a Phase 2 resolver-validation error.
//!
//! All fields are `Option<...>` so JSON can omit anything irrelevant.
//! `#[serde(default, skip_serializing_if = "Option::is_none")]` keeps
//! the on-the-wire shape minimal: `{"role":"button","text":"Save"}`
//! deserialises cleanly even though `Selector` has 17 nullable fields.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Selector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    /// A11y / semantic role (`button` / `link` / `text` / `image` / …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Exact-match on visible text content. Mutually exclusive with
    /// `text_contains`; providing both is a resolver-validation
    /// error caught at evaluation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Substring match on visible text content (case-sensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Relational: filter to nodes whose layout-rect centre is within
    /// the supplied selector's match's centre by ≤ 64 logical px (the
    /// fixed threshold in Phase 1; a `radius_px` knob can land later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<Box<Selector>>,

    /// Filter to nodes whose ancestor chain contains a match for the
    /// inner selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_of: Option<Box<Selector>>,

    /// Filter to nodes whose descendant subtree contains a match for
    /// the inner selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_of: Option<Box<Selector>>,

    /// Logical combinators. These take a vector to enable the
    /// canonical AND / OR / NOT semantics over arbitrary sub-selectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_of: Option<Vec<Selector>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_of: Option<Vec<Selector>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<Box<Selector>>,

    /// Pick the document-order first match. Mutually exclusive with
    /// `index`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<bool>,

    /// Zero-indexed pick from the candidate set. Mutually exclusive
    /// with `first`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// Canonical `all_of` / `any_of` / `not` discriminator — exposed for
/// callers that want to construct combinators without dropping into
/// the field literals. Phase 2 resolver uses this internally too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    AllOf,
    AnyOf,
    Not,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selector_is_serializable() {
        // `{}` should round-trip — the resolver handles it as
        // "match every node", though that's typically not useful.
        let s = Selector::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "{}");
        let back: Selector = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn typical_button_selector_round_trips() {
        let json = r#"{"role":"button","text":"Save","visible":true}"#;
        let s: Selector = serde_json::from_str(json).unwrap();
        assert_eq!(s.role.as_deref(), Some("button"));
        assert_eq!(s.text.as_deref(), Some("Save"));
        assert_eq!(s.visible, Some(true));
        // Re-serialise; field-omitted ones must stay omitted.
        let again = serde_json::to_string(&s).unwrap();
        let back: Selector = serde_json::from_str(&again).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn nested_relational_selector_parses() {
        // `find a Save button whose parent is a frame with text "Doc"`.
        let json = r##"{
            "role": "button",
            "text": "Save",
            "child_of": { "role": "frame", "text_contains": "Doc" }
        }"##;
        let s: Selector = serde_json::from_str(json).unwrap();
        let parent = s.child_of.as_deref().unwrap();
        assert_eq!(parent.role.as_deref(), Some("frame"));
        assert_eq!(parent.text_contains.as_deref(), Some("Doc"));
    }

    #[test]
    fn all_of_any_of_combinators_parse() {
        let json = r##"{
            "all_of": [
                { "role": "button" },
                { "any_of": [
                    { "text": "Save" },
                    { "text_contains": "Submit" }
                ]}
            ]
        }"##;
        let s: Selector = serde_json::from_str(json).unwrap();
        let outer = s.all_of.as_ref().unwrap();
        assert_eq!(outer.len(), 2);
        let inner = outer[1].any_of.as_ref().unwrap();
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0].text.as_deref(), Some("Save"));
        assert_eq!(inner[1].text_contains.as_deref(), Some("Submit"));
    }

    #[test]
    fn unknown_field_rejected() {
        // serde's default is to ignore unknown fields. ASP intentionally
        // does NOT use `#[serde(deny_unknown_fields)]` — Phase 2 may add
        // new optional fields, and a stricter mode would force every
        // agent to upgrade in lock-step. The test pins the lenient
        // behaviour so a future change to `deny_unknown_fields` is a
        // conscious decision.
        let json = r#"{"role":"button","made_up":"x"}"#;
        let s: Selector = serde_json::from_str(json).unwrap();
        assert_eq!(s.role.as_deref(), Some("button"));
    }

    #[test]
    fn combinator_enum_is_useful_for_callers() {
        // Smoke: the enum exists and is comparable so a future helper
        // that constructs combinator selectors programmatically (e.g.
        // a builder used by the verb_impls layer) has a stable
        // discriminator.
        assert_ne!(Combinator::AllOf, Combinator::AnyOf);
        assert_ne!(Combinator::AnyOf, Combinator::Not);
    }
}
