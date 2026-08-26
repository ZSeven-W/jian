use crate::expression::Expression;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

/// Flattened bag of unknown/future interaction keys.
///
/// serde treats it like the map it wraps (`#[serde(transparent)]`), so
/// the enclosing struct's `#[serde(flatten)]` field hoists every key
/// into the parent object and unknown keys survive round-trip verbatim.
///
/// ts-rs refuses to flatten a plain map (`"{ [key in string]?: JsonValue }
/// cannot be flattened"`), so the manual `TS` impl below reuses ts-rs's
/// own map rendering as `inline_flattened`. The generated TypeScript for
/// the enclosing struct therefore becomes
/// `{ <known fields> } & ({ [key in string]?: JsonValue })` — known
/// fields stay precise AND arbitrary future keys stay expressible.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ExtraJson(pub BTreeMap<String, serde_json::Value>);

impl Deref for ExtraJson {
    type Target = BTreeMap<String, serde_json::Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ExtraJson {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// ts-rs's derived `TS` cannot be used on a transparent newtype in a
// flattened position (the map impl panics on `inline_flattened`), so the
// trait is implemented by hand, reusing the map's own TypeScript
// rendering `{ [key in string]?: JsonValue }`.
//
// The rendering is wrapped in parens: ts-rs merges `{ A } & { B }` into a
// single object literal, which is invalid for a *mapped* type member
// (`{ x: T, [key in string]?: V }` does not parse). The parentheses keep
// the intersection shape `{ known: ... } & ({ [key in string]?: JsonValue })`,
// which is valid TypeScript and keeps known fields independent of the
// index signature.
#[cfg(feature = "export-ts")]
impl ts_rs::TS for ExtraJson {
    type WithoutGenerics = Self;

    fn name() -> String {
        <BTreeMap<String, serde_json::Value> as ts_rs::TS>::name()
    }

    fn inline() -> String {
        <Self as ts_rs::TS>::name()
    }

    fn inline_flattened() -> String {
        format!("({})", <Self as ts_rs::TS>::name())
    }

    fn decl() -> String {
        panic!("ExtraJson cannot be declared")
    }

    fn decl_concrete() -> String {
        panic!("ExtraJson cannot be declared")
    }

    fn visit_dependencies(v: &mut impl ts_rs::TypeVisitor)
    where
        Self: 'static,
    {
        v.visit::<serde_json::Value>();
    }
}

/// A single Action is a 1-key object: `{ "<action_name>": <body> }`.
///
/// Examples (all are valid JSON `Action`s):
/// - `{ "set": { "$state.count": "$state.count + 1" } }`
/// - `{ "fetch": { "url": "/api/x", "into": "$state.u" } }`
/// - `{ "push": "/detail/42" }`
///
/// The body shape per action is NOT validated here — see `jian-core::action`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(transparent)]
pub struct Action(pub BTreeMap<String, serde_json::Value>);

impl Action {
    /// Returns (action_name, body) if this Action has exactly one key.
    pub fn single(&self) -> Option<(&str, &serde_json::Value)> {
        let mut iter = self.0.iter();
        let first = iter.next()?;
        if iter.next().is_some() {
            return None;
        }
        Some((first.0.as_str(), first.1))
    }
}

pub type ActionList = Vec<Action>;

/// All supported event hook keys. Note: input events (`onChange`, `onSubmit`, `onFocus`,
/// `onBlur`) apply only to input-kind nodes. `on_key` is keyboard, `on_reach_end`
/// is list-scroll-end, etc.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct EventHandlers {
    // Gesture-originated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_tap: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_double_tap: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_long_press: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_hover_enter: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_hover_leave: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_press_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_press_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_press_cancel: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_swipe: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_context_menu: Option<ActionList>,
    /// Raw pointer escape-hatch: fired for pointer Down/Move/Up when the
    /// node (or an ancestor) declares `gestures.rawPointer`.
    /// `SemanticEvent::RawPointer` maps here via
    /// `gesture::semantic::handler_key`; the runtime was already able to
    /// execute it dynamically (it survives round-trip through `extra`),
    /// this field makes it typed so AOT covers it too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_raw_pointer: Option<ActionList>,

    // Input-node events
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_submit: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_focus: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_blur: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_key: Option<ActionList>,

    // Scroll / list
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scroll: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_reach_end: Option<ActionList>,

    /// Future/unknown hook keys (e.g. `onFutureGesture`). Preserved
    /// verbatim on round-trip; known hooks win on key collision. The
    /// runtime never executes unknown hooks — an older runtime passes
    /// them through untouched. Exported to TypeScript as
    /// `{ [key in string]?: JsonValue }` alongside the known fields.
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

/// `bindings` is a map from property-name (with optional `bind:` prefix for two-way)
/// to a Tier-1 expression. E.g. `{ "content": "`Count: ${$state.count}`" }` or
/// `{ "bind:value": "$state.email" }`.
pub type Bindings = BTreeMap<String, Expression>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_set_action() {
        let json = r#"{"set":{"$state.count":"$state.count + 1"}}"#;
        let a: Action = serde_json::from_str(json).unwrap();
        let (name, body) = a.single().unwrap();
        assert_eq!(name, "set");
        assert!(body.is_object());
    }

    #[test]
    fn push_action_with_string_body() {
        let json = r#"{"push":"/detail/42"}"#;
        let a: Action = serde_json::from_str(json).unwrap();
        let (name, body) = a.single().unwrap();
        assert_eq!(name, "push");
        assert_eq!(body.as_str(), Some("/detail/42"));
    }

    #[test]
    fn event_handlers_partial() {
        let json = r#"{
          "onTap": [{"set":{"$state.count":"$state.count+1"}}],
          "onLongPress": [{"open_menu":"context"}]
        }"#;
        let e: EventHandlers = serde_json::from_str(json).unwrap();
        assert_eq!(e.on_tap.unwrap().len(), 1);
        assert_eq!(e.on_long_press.unwrap().len(), 1);
    }

    #[test]
    fn bindings_with_two_way() {
        let json = r#"{
          "content": "\"Count: \" + $state.count",
          "bind:value": "$state.email"
        }"#;
        let b: Bindings = serde_json::from_str(json).unwrap();
        assert_eq!(b.len(), 2);
        assert!(b.contains_key("bind:value"));
    }

    #[test]
    fn rich_event_hooks_and_future_fields_round_trip() {
        let input = serde_json::json!({
            "onPressStart": [{"set":{"$app.down":"true"}}],
            "onPressEnd": [{"set":{"$app.down":"false"}}],
            "onPressCancel": [{"set":{"$app.cancelled":"true"}}],
            "onSwipe": [{"set":{"$app.direction":"$event.direction"}}],
            "onContextMenu": [{"toast":"`Context`"}],
            "onFutureGesture": [{"futureAction":{"value":1}}]
        });
        let decoded: EventHandlers = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(&decoded).unwrap();
        assert_eq!(output, input);
        // The unknown hook lands in `extra` and is never shadowed by
        // a known field.
        let future = decoded
            .extra
            .get("onFutureGesture")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("futureAction"))
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_i64());
        assert_eq!(future, Some(1));
    }

    #[test]
    fn known_action_future_body_round_trip() {
        // `Action` is a raw 1-key body map; a future field appended to a
        // known action's body must survive deserialize/serialize.
        let input = serde_json::json!({
            "onTap": [{"set":{"$app.down":"true","futureCool":{"x":1}}}],
            "onPressCancel": [{"delay":{"ms":10,"futureMs":"$app.t"}}]
        });
        let decoded: EventHandlers = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), input);
    }

    #[test]
    fn legacy_event_handlers_serialize_unchanged() {
        // Old empty object: no new fields may leak into the output.
        assert_eq!(
            serde_json::to_value(EventHandlers::default()).unwrap(),
            serde_json::json!({})
        );
        // Old fixture: only the authored key survives.
        let input = serde_json::json!({
            "onTap": [{"set":{"$state.count":"$state.count + 1"}}],
            "onLongPress": [{"openMenu":"context"}]
        });
        let decoded: EventHandlers = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), input);
    }

    #[test]
    fn raw_pointer_handler_round_trips() {
        // R1 Blocker 1: `onRawPointer` is a first-class typed hook now —
        // it must deserialize into the `on_raw_pointer` field (not linger
        // in `extra`) and serialize back verbatim.
        let input = serde_json::json!({
            "onRawPointer": [ { "set": { "$app.raws": "$state.raws + 1" } } ]
        });
        let decoded: EventHandlers = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(
            decoded.on_raw_pointer.as_ref().map(ActionList::len),
            Some(1)
        );
        assert!(decoded.extra.is_empty(), "known hook leaked into extra");
        assert_eq!(serde_json::to_value(&decoded).unwrap(), input);
    }

    #[test]
    fn extra_json_is_transparent_on_the_wire() {
        // `ExtraJson` is a serde-transparent newtype over the map, so
        // it serializes as the bare object (no `0` key, no wrapping).
        let input = serde_json::json!({ "onFutureGesture": [{ "futureAction": { "v": 1 } }] });
        let decoded: ExtraJson = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), input);
        // And it stays a plain map from Rust through `Deref`.
        assert_eq!(
            decoded
                .get("onFutureGesture")
                .and_then(|v| v.get(0))
                .and_then(|v| v.get("futureAction")),
            Some(&serde_json::json!({ "v": 1 }))
        );
    }
}
