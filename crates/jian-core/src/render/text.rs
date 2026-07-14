//! Backend-neutral text styling shared by scene collection and rich hosts.
//!
//! `DrawOp::Text` remains the stable native-facing command. Hosts that can
//! shape styled paragraphs consume the parallel [`TextSpan`] metadata emitted
//! by the rich scene collector, while every existing backend continues to see
//! the same flat text command.

use super::{DrawOp, TextAlign, TextRun};
use crate::geometry::{point, Rect};
use crate::scene::Color;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub content: String,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub letter_spacing: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Default)]
pub struct RichDrawList {
    pub ops: Vec<DrawOp>,
    /// `(DrawOp index, exact styled paragraph runs)`.
    pub text_runs: Vec<(usize, Vec<TextSpan>)>,
}

/// Explicit paragraph layout metadata for rich hosts. This stays beside the
/// flat `DrawOp` so native backends continue to receive the legacy command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichTextGrowth {
    /// Non-responsive documents retain the pre-responsive flat DrawOp contract.
    Legacy,
    Auto,
    FixedWidth,
    FixedWidthHeight,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RichTextPlan {
    pub op_index: usize,
    pub spans: Vec<TextSpan>,
    pub bounds: Rect,
    pub growth: RichTextGrowth,
}

pub(super) fn resolve_text(
    json: &Value,
    rect: Rect,
    responsive: bool,
) -> Option<(DrawOp, Vec<TextSpan>, RichTextGrowth)> {
    if json.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }

    let node_family = json
        .get("fontFamily")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let node_size = json.get("fontSize").and_then(Value::as_f64).unwrap_or(14.0) as f32;
    let node_weight = weight(json.get("fontWeight"), 400);
    let node_italic = italic(json.get("fontStyle"), false);
    let node_spacing = json
        .get("letterSpacing")
        .and_then(Value::as_f64)
        .unwrap_or(0.0) as f32;
    let node_color = solid_fill(json.get("fill")).unwrap_or(Color::rgb(0, 0, 0));

    let spans = match json.get("content")? {
        Value::String(content) if !content.is_empty() => vec![TextSpan {
            content: content.clone(),
            font_family: node_family.clone(),
            font_size: node_size,
            font_weight: node_weight,
            italic: node_italic,
            letter_spacing: node_spacing,
            color: node_color,
        }],
        Value::Array(segments) => segments
            .iter()
            .filter_map(|segment| {
                let object = segment.as_object()?;
                let content = object.get("text")?.as_str()?;
                if content.is_empty() {
                    return None;
                }
                Some(TextSpan {
                    content: content.to_owned(),
                    font_family: object
                        .get("fontFamily")
                        .and_then(Value::as_str)
                        .unwrap_or(&node_family)
                        .to_owned(),
                    font_size: object
                        .get("fontSize")
                        .and_then(Value::as_f64)
                        .map_or(node_size, |value| value as f32),
                    font_weight: weight(object.get("fontWeight"), node_weight),
                    italic: italic(object.get("fontStyle"), node_italic),
                    // StyledTextSegment has no per-run spacing; layout
                    // inherits the node value for every segment.
                    letter_spacing: node_spacing,
                    color: object
                        .get("fill")
                        .and_then(Value::as_str)
                        .and_then(Color::from_hex)
                        .unwrap_or(node_color),
                })
            })
            .collect(),
        _ => return None,
    };
    if spans.is_empty() {
        return None;
    }

    let content = spans.iter().map(|span| span.content.as_str()).collect();
    let align = match json.get("textAlign").and_then(Value::as_str) {
        Some("center") => TextAlign::Center,
        Some("right" | "end") => TextAlign::End,
        _ => TextAlign::Start,
    };
    // Unitless multipliers only, per main's line-height normalization: a
    // pixel-like 17 must not be taken as a 17x multiplier. The check used to
    // live in `try_text`, which this branch removed, so it travels with it.
    let line_height = jian_ops_schema::node::text::canonical_line_height_multiplier(
        json.get("lineHeight").and_then(Value::as_f64),
    )
    .unwrap_or(0.0) as f32;
    let growth = if responsive {
        match json.get("textGrowth").and_then(Value::as_str) {
            Some("fixed-width") => RichTextGrowth::FixedWidth,
            Some("fixed-width-height") => RichTextGrowth::FixedWidthHeight,
            Some("auto") | None | Some(_) => RichTextGrowth::Auto,
        }
    } else {
        RichTextGrowth::Legacy
    };
    // The zero sentinel is rich-host-only. Flat DrawOps from non-responsive
    // documents always retain the authored width used before responsive text
    // growth existed, keeping native output unchanged.
    let max_width = match growth {
        RichTextGrowth::FixedWidth | RichTextGrowth::Legacy => rect.size.width,
        RichTextGrowth::FixedWidthHeight | RichTextGrowth::Auto => 0.0,
    };
    // Rich hosts consume `spans`; the flat fallback retains node-level style,
    // the legacy 16px default, and numeric-only weight.
    let legacy_size = json.get("fontSize").and_then(Value::as_f64).unwrap_or(16.0) as f32;
    let legacy_weight = json
        .get("fontWeight")
        .and_then(Value::as_u64)
        .map_or(400, |value| value as u16);
    Some((
        DrawOp::Text(TextRun {
            content,
            font_family: node_family,
            font_size: legacy_size,
            font_weight: legacy_weight,
            color: node_color,
            origin: point(rect.min_x(), rect.min_y()),
            max_width,
            align,
            line_height,
        }),
        spans,
        growth,
    ))
}

fn solid_fill(value: Option<&Value>) -> Option<Color> {
    value?
        .as_array()?
        .iter()
        .filter_map(Value::as_object)
        .find(|fill| fill.get("type").and_then(Value::as_str) == Some("solid"))?
        .get("color")?
        .as_str()
        .and_then(Color::from_hex)
}

fn weight(value: Option<&Value>, inherited: u16) -> u16 {
    let Some(value) = value else { return inherited };
    if let Some(number) = value.as_u64() {
        return number as u16;
    }
    let Some(text) = value.as_str() else {
        return inherited;
    };
    text.parse().unwrap_or(match text {
        "bold" => 700,
        "semibold" | "semi-bold" | "demibold" => 600,
        "medium" => 500,
        "normal" | "regular" => 400,
        "light" => 300,
        "extralight" | "extra-light" | "ultralight" | "ultra-light" => 200,
        "thin" | "hairline" => 100,
        "black" | "heavy" => 900,
        "extrabold" | "extra-bold" | "ultrabold" | "ultra-bold" => 800,
        _ => inherited,
    })
}

fn italic(value: Option<&Value>, inherited: bool) -> bool {
    value
        .and_then(Value::as_str)
        .map_or(inherited, |value| value == "italic")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::rect;

    #[test]
    fn styled_runs_match_layout_defaults_and_inheritance() {
        let json = serde_json::json!({
            "type": "text",
            "content": [
                {"text": "A", "fontSize": 20, "fill": "#ff0000"},
                {"text": "B", "fontWeight": 700, "fontStyle": "italic"}
            ],
            "fontFamily": "Family",
            "letterSpacing": 2,
            "fill": [{"type": "solid", "color": "#0000ff"}]
        });
        let (_, spans, _) = resolve_text(&json, rect(0.0, 0.0, 80.0, 40.0), true).unwrap();
        assert_eq!(spans[0].font_size, 20.0);
        assert_eq!(spans[0].font_weight, 400);
        assert_eq!(spans[0].letter_spacing, 2.0);
        assert_eq!(spans[0].color, Color::rgb(255, 0, 0));
        assert_eq!(spans[1].font_size, 14.0);
        assert_eq!(spans[1].font_weight, 700);
        assert!(spans[1].italic);
        assert_eq!(spans[1].color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn flat_draw_op_preserves_legacy_node_only_style() {
        let json = serde_json::json!({
            "type": "text",
            "content": [{"text": "A", "fontSize": 24, "fill": "#ff0000"}],
            "fontFamily": "NodeFamily",
            "fontWeight": "heavy",
            "fill": [{"type": "solid", "color": "#0000ff"}]
        });
        let (DrawOp::Text(run), spans, _) =
            resolve_text(&json, rect(1.0, 2.0, 80.0, 40.0), true).unwrap()
        else {
            panic!("expected text")
        };
        assert_eq!(run.content, "A");
        assert_eq!(run.font_family, "NodeFamily");
        assert_eq!(run.font_size, 16.0);
        assert_eq!(run.font_weight, 400);
        assert_eq!(run.color, Color::rgb(0, 0, 255));
        assert_eq!(spans[0].font_weight, 900);
    }

    #[test]
    fn rich_draw_op_wrap_budget_matches_text_growth() {
        for (growth, expected_width) in [
            ("fixed-width", 80.0),
            ("fixed-width-height", 0.0),
            ("auto", 0.0),
        ] {
            let json = serde_json::json!({
                "type": "text",
                "content": "a deliberately long line",
                "textGrowth": growth,
            });
            let (DrawOp::Text(run), _, _) =
                resolve_text(&json, rect(0.0, 0.0, 80.0, 40.0), true).unwrap()
            else {
                panic!("expected text")
            };
            assert_eq!(run.max_width, expected_width, "growth={growth}");
        }
    }

    #[test]
    fn flat_nonresponsive_text_draw_op_keeps_the_authored_width() {
        for growth in [None, Some("auto"), Some("fixed-width-height")] {
            let mut json = serde_json::json!({
                "type": "text",
                "content": "legacy flat text",
            });
            if let Some(growth) = growth {
                json["textGrowth"] = serde_json::json!(growth);
            }
            let (DrawOp::Text(run), _, growth_mode) =
                resolve_text(&json, rect(0.0, 0.0, 80.0, 40.0), false).unwrap()
            else {
                panic!("expected text")
            };
            assert_eq!(run.max_width, 80.0, "growth={growth:?}");
            assert_eq!(growth_mode, RichTextGrowth::Legacy);
        }
    }
}
