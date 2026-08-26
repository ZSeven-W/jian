use crate::events::ExtraJson;
use crate::expression::Expression;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum ScrollBehavior {
    Auto,
    Contain,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub enum AxisLock {
    /// No axis constraint — the swipe direction is judged freely.
    Auto,
    /// Horizontal movement locks the vertical axis.
    Horizontal,
    /// Vertical movement locks the horizontal axis.
    Vertical,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct GestureOverrides {
    /// When true, this node and its subtree bypass the Arena and receive raw pointer events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_pointer: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Expression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_behavior: Option<ScrollBehavior>,
    /// Override drag threshold in logical pixels (default 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag_threshold: Option<f64>,
    /// Override long-press duration in ms (default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_press_duration: Option<u32>,
    /// Author-explicit Tab-traversal opt-in.
    ///
    /// `Some(true)` — node enters the focus chain regardless of its
    /// semantic role.
    /// `Some(false)` — node is excluded even if its `semantics.role`
    /// would otherwise auto-include it (e.g. a decorative `Input`).
    /// `None` — falls back to the role heuristic (`Button` / `Link`
    /// / `Input` are auto-included; everything else is opt-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focusable: Option<bool>,

    /// Double-tap detection window in ms (default 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_tap_timeout: Option<u32>,
    /// Max distance between two taps to still count as a double-tap (px).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_tap_slop: Option<f64>,
    /// Minimum travel distance for a swipe to claim (px).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swipe_min_distance: Option<f64>,
    /// Minimum velocity for a swipe to claim (px/s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swipe_min_velocity: Option<f64>,
    /// Axis constraint applied when judging a swipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_lock: Option<AxisLock>,
    /// Event hooks disabled on this node (`["onHoverEnter", ...]`).
    /// Order is preserved exactly as authored; the schema layer never
    /// dedups or rewrites the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_events: Option<Vec<String>>,
    /// Explicit handler evaluation order (`["onSwipe", "onTap", ...]`).
    /// Order is preserved exactly as authored; the schema layer never
    /// dedups or rewrites the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_order: Option<Vec<String>>,
    /// Future/unknown override keys. Preserved verbatim on round-trip.
    /// Exported to TypeScript as `{ [key in string]?: JsonValue }`
    /// alongside the known fields.
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_pointer_override() {
        let g: GestureOverrides = serde_json::from_str(r#"{"rawPointer":true}"#).unwrap();
        assert_eq!(g.raw_pointer, Some(true));
    }

    #[test]
    fn scroll_and_thresholds() {
        let json = r#"{"scrollBehavior":"contain","dragThreshold":16,"longPressDuration":300}"#;
        let g: GestureOverrides = serde_json::from_str(json).unwrap();
        assert!(matches!(g.scroll_behavior, Some(ScrollBehavior::Contain)));
        assert_eq!(g.drag_threshold, Some(16.0));
        assert_eq!(g.long_press_duration, Some(300));
    }

    #[test]
    fn focusable_round_trips() {
        let yes: GestureOverrides = serde_json::from_str(r#"{"focusable":true}"#).unwrap();
        assert_eq!(yes.focusable, Some(true));
        let no: GestureOverrides = serde_json::from_str(r#"{"focusable":false}"#).unwrap();
        assert_eq!(no.focusable, Some(false));
        let unset: GestureOverrides = serde_json::from_str("{}").unwrap();
        assert_eq!(unset.focusable, None);
        // Round-trip preserves the explicit `false` (so an author can
        // opt a default-focusable role *out*).
        let ser = serde_json::to_string(&no).unwrap();
        assert!(ser.contains("\"focusable\":false"));
    }

    #[test]
    fn rich_gesture_overrides_round_trip() {
        let input = serde_json::json!({
            "doubleTapTimeout": 280,
            "doubleTapSlop": 12,
            "swipeMinDistance": 48,
            "swipeMinVelocity": 320,
            "axisLock": "horizontal",
            "disabledEvents": ["onHoverEnter"],
            "interactionOrder": ["onSwipe", "onTap"],
            "futureThreshold": 7
        });
        let decoded: GestureOverrides = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(&decoded).unwrap();
        // `doubleTapSlop`/`swipeMinDistance`/`swipeMinVelocity` are
        // typed as f64, so an integral authored value re-serializes as
        // `12.0` instead of `12` — numerically identical, never lossy.
        // Compare with numbers normalized to f64.
        assert_eq!(normalize_numbers(output.clone()), normalize_numbers(input));
        // Second decode of the output equals the first decode: the
        // round-trip is stable.
        let again: GestureOverrides = serde_json::from_value(output).unwrap();
        assert_eq!(again, decoded);
        // The unknown key lands in `extra` and is never shadowed by a
        // known field.
        assert_eq!(
            decoded
                .extra
                .get("futureThreshold")
                .and_then(|v| v.as_i64()),
            Some(7)
        );
    }

    #[test]
    fn axis_lock_wire_names_are_auto_horizontal_vertical() {
        // Camel wire: `Auto` -> "auto", `Horizontal` -> "horizontal",
        // `Vertical` -> "vertical".
        assert_eq!(
            serde_json::to_value(AxisLock::Auto).unwrap(),
            serde_json::json!("auto")
        );
        assert_eq!(
            serde_json::to_value(AxisLock::Horizontal).unwrap(),
            serde_json::json!("horizontal")
        );
        assert_eq!(
            serde_json::to_value(AxisLock::Vertical).unwrap(),
            serde_json::json!("vertical")
        );
        assert_eq!(
            serde_json::from_value::<AxisLock>(serde_json::json!("auto")).unwrap(),
            AxisLock::Auto
        );
        assert_eq!(
            serde_json::from_value::<AxisLock>(serde_json::json!("horizontal")).unwrap(),
            AxisLock::Horizontal
        );
        assert_eq!(
            serde_json::from_value::<AxisLock>(serde_json::json!("vertical")).unwrap(),
            AxisLock::Vertical
        );
    }

    #[test]
    fn disabled_and_interaction_order_preserve_vector_order() {
        // Vec<String> order is preserved exactly; the schema layer does
        // NOT dedupe or rewrite (duplicates are an authoring concern).
        let json = r#"{"disabledEvents":["onSwipe","onTap","onSwipe"],"interactionOrder":["onSwipe","onTap","onSwipe"]}"#;
        let g: GestureOverrides = serde_json::from_str(json).unwrap();
        let ordered = vec![
            "onSwipe".to_owned(),
            "onTap".to_owned(),
            "onSwipe".to_owned(),
        ];
        assert_eq!(g.disabled_events.as_deref(), Some(ordered.as_slice()));
        assert_eq!(g.interaction_order.as_deref(), Some(ordered.as_slice()));
        // Round-trip keeps the same order with duplicates intact.
        let output = serde_json::to_value(&g).unwrap();
        assert_eq!(
            output["disabledEvents"],
            serde_json::json!(["onSwipe", "onTap", "onSwipe"])
        );
    }

    #[test]
    fn legacy_gesture_overrides_serialize_unchanged() {
        // Old empty object: no new fields may leak into the output.
        assert_eq!(
            serde_json::to_value(GestureOverrides::default()).unwrap(),
            serde_json::json!({})
        );
        // Old fixture: only the authored keys survive (the new optional
        // fields stay absent when unset). `dragThreshold` is f64-typed,
        // so compare numerically.
        let input = serde_json::json!({"rawPointer": true, "dragThreshold": 8});
        let decoded: GestureOverrides = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(&decoded).unwrap();
        assert_eq!(normalize_numbers(output.clone()), normalize_numbers(input));
        let keys: Vec<_> = output.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec!["rawPointer".to_owned(), "dragThreshold".to_owned()]
        );
    }

    /// Walk a JSON value and convert every number to its f64 form so
    /// integral inputs (`12`) compare equal to f64-typed fields that
    /// re-serialize as `12.0`.
    fn normalize_numbers(mut v: serde_json::Value) -> serde_json::Value {
        fn walk(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item);
                    }
                }
                serde_json::Value::Object(entries) => {
                    for entry in entries.values_mut() {
                        walk(entry);
                    }
                }
                serde_json::Value::Number(n) => {
                    if let Some(f) = n.as_f64() {
                        *n = serde_json::Number::from_f64(f).expect("finite f64 number");
                    }
                }
                _ => {}
            }
        }
        walk(&mut v);
        v
    }
}
