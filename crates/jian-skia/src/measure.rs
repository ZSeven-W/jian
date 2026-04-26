//! `SkiaMeasure` — `MeasureBackend` over `skia_safe::textlayout::Paragraph`.
//!
//! Each layout-time measure becomes one ParagraphBuilder cycle:
//! per-run `push_style + add_text + pop`, then `layout(max_width)`,
//! then read `longest_line` / `height` / `line_number`. This gets us
//! the same metrics the renderer uses for paint, so a node's
//! laid-out rect agrees with the painted glyphs to within Skia's
//! own rounding.
//!
//! The backend holds an `Rc<FontCollection>` populated from
//! `FontMgr::default()` (system fonts) and clones a fresh one into
//! every ParagraphBuilder. Building a brand-new collection per call
//! would re-init the font manager — cloning the Rc is essentially
//! free.
//!
//! Gated behind `cfg(feature = "textlayout")`. Default-feature
//! builds rely on `EstimateBackend` from `jian-core`.

use jian_core::layout::measure::{
    FontStyleKind, MeasureBackend, MeasureRequest, MeasureResult,
};
use skia_safe::{
    font_style::{Slant, Weight, Width},
    textlayout::{FontCollection, ParagraphBuilder, ParagraphStyle, TextStyle},
    FontMgr, FontStyle,
};
use std::rc::Rc;

/// Wrap budget passed to `paragraph.layout()` when the caller wants
/// the natural single-line extent. Skia requires a finite width
/// (NaN / infinity panic), so we use a value far larger than any
/// realistic UI surface — 1 million logical pixels — to guarantee
/// no wrap happens. `paragraph.max_intrinsic_width()` then reports
/// the unwrapped shaped width regardless of the budget.
const NATURAL_LAYOUT_BUDGET: f32 = 1.0e6;

/// Phase 1 hardcoded font width axis. `StyledRun` doesn't currently
/// carry a width (compressed / extended) — when it does, swap this
/// for `run.font_width` and bump the schema. Until then every shaped
/// run uses the typeface's `Normal` width.
const FONT_WIDTH: Width = Width::NORMAL;

/// Measure backend that defers to skia's paragraph shaper.
///
/// Construct once at host startup, share the same `Rc` across the
/// whole runtime. The default `FontMgr` resolves system fonts on
/// every supported platform; hosts that bundle their own typefaces
/// can build via [`SkiaMeasure::with_font_manager`].
pub struct SkiaMeasure {
    font_collection: Rc<FontCollection>,
}

impl SkiaMeasure {
    /// Use the platform's default `FontMgr` (system fonts).
    pub fn new() -> Self {
        Self::with_font_manager(FontMgr::default())
    }

    /// Use a host-supplied `FontMgr` — for example one that wraps a
    /// `TypefaceFontProvider` populated with bundled `.ttf` blobs.
    pub fn with_font_manager(font_mgr: FontMgr) -> Self {
        let mut fc = FontCollection::new();
        fc.set_default_font_manager(font_mgr, None);
        Self {
            font_collection: Rc::new(fc),
        }
    }
}

impl Default for SkiaMeasure {
    fn default() -> Self {
        Self::new()
    }
}

impl MeasureBackend for SkiaMeasure {
    fn measure(&self, req: &MeasureRequest<'_>) -> MeasureResult {
        if req.runs.is_empty() {
            return MeasureResult {
                width: 0.0,
                height: 0.0,
                line_count: 0,
                baseline: 0.0,
            };
        }

        // ParagraphStyle is a per-paragraph default container;
        // line_height is applied per TextStyle below so individual
        // runs can opt in/out independently.
        let style = ParagraphStyle::new();
        let mut builder =
            ParagraphBuilder::new(&style, (*self.font_collection).clone());

        for run in req.runs {
            let mut ts = TextStyle::new();
            ts.set_font_size(run.font_size);
            if let Some(family) = run.font_family {
                ts.set_font_families(&[family]);
            }
            let weight = Weight::from(run.font_weight as i32);
            let slant = match run.font_style {
                FontStyleKind::Italic => Slant::Italic,
                FontStyleKind::Normal => Slant::Upright,
            };
            ts.set_font_style(FontStyle::new(weight, FONT_WIDTH, slant));
            if run.letter_spacing != 0.0 {
                ts.set_letter_spacing(run.letter_spacing);
            }
            if req.line_height > 0.0 {
                ts.set_height(req.line_height);
                ts.set_height_override(true);
            }
            builder.push_style(&ts);
            builder.add_text(run.text);
            builder.pop();
        }

        let mut paragraph = builder.build();
        // `NATURAL_LAYOUT_BUDGET` keeps the wrap budget finite per
        // Skia's contract; `max_intrinsic_width` then reports the
        // shaped single-line width regardless of the budget value.
        let layout_width = req.max_width.unwrap_or(NATURAL_LAYOUT_BUDGET);
        paragraph.layout(layout_width);

        let width = match req.max_width {
            // Wrapping mode: `longest_line` is the rendered width
            // (always ≤ the budget). It matches what the renderer
            // would paint, which is the contract the plan promises.
            Some(_) => paragraph.longest_line(),
            // Natural extent: `max_intrinsic_width` is the unwrapped
            // single-line width even when the layout was given an
            // arbitrary huge budget.
            None => paragraph.max_intrinsic_width(),
        };

        MeasureResult {
            width,
            height: paragraph.height(),
            // line_number is 0-indexed in skia; layout always
            // produces at least one logical line for non-empty
            // text, so report `max(1, n)`.
            line_count: paragraph.line_number().max(1) as u16,
            baseline: paragraph.alphabetic_baseline(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::layout::measure::{MeasureBackend, StyledRun};

    fn run<'a>(text: &'a str, weight: u16, size: f32) -> StyledRun<'a> {
        StyledRun {
            text,
            font_family: None,
            font_size: size,
            font_weight: weight,
            font_style: FontStyleKind::Normal,
            letter_spacing: 0.0,
        }
    }

    #[test]
    fn measures_ascii_natural_extent() {
        let backend = SkiaMeasure::new();
        let runs = [run("Hello", 400, 16.0)];
        let res = backend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: None,
        });
        // Skia's default font on macOS / Linux / Windows produces
        // ~30-40px for "Hello" @ 16pt depending on the resolved
        // typeface. The estimator's same input is `5 * 16 * 0.58 =
        // 46.4` — a 5–10% overshoot on Latin. Pin a generous range
        // so the test doesn't trip on per-platform font swaps.
        assert!(
            res.width > 20.0 && res.width < 60.0,
            "ASCII natural extent out of expected band: {}",
            res.width,
        );
        assert_eq!(res.line_count, 1);
        assert!(res.height >= 16.0 && res.height < 32.0);
    }

    #[test]
    fn measures_cjk_wider_than_estimator_would() {
        // Estimator under-shoots CJK by ~50% (ratio 0.58 vs ≈1.0
        // square glyph). Skia's shaper resolves a CJK fallback and
        // reports the real glyph width. We assert the shaper width
        // is strictly *more* than what the estimator would have
        // produced for the same string — that's the bug the whole
        // backend exists to fix.
        let backend = SkiaMeasure::new();
        let runs = [run("你好", 400, 16.0)];
        let res = backend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: None,
        });
        let estimator = 2.0 * 16.0 * 0.58; // 18.56
        assert!(
            res.width > estimator * 1.2,
            "CJK shaped width should be >>20% wider than estimator's \
             {}, got {}",
            estimator,
            res.width,
        );
    }

    #[test]
    fn styled_mix_weighted_runs_widen_paragraph() {
        // A bold-tail run shapes wider than the same text at uniform
        // 400 weight. Catches a regression where push_style/pop
        // ordering would silently swallow a segment's style.
        let backend = SkiaMeasure::new();
        let uniform = backend.measure(&MeasureRequest {
            runs: &[run("Hello there friend", 400, 16.0)],
            line_height: 0.0,
            max_width: None,
        });
        let mixed = [
            run("Hello ", 400, 16.0),
            run("there ", 700, 16.0),
            run("friend", 400, 16.0),
        ];
        let mixed_res = backend.measure(&MeasureRequest {
            runs: &mixed,
            line_height: 0.0,
            max_width: None,
        });
        assert!(
            mixed_res.width >= uniform.width,
            "mixed-weight paragraph must not be narrower than uniform: \
             uniform={}, mixed={}",
            uniform.width,
            mixed_res.width,
        );
    }

    #[test]
    fn wraps_to_max_width() {
        let backend = SkiaMeasure::new();
        let runs = [run(
            "This sentence is intentionally long enough to wrap when the \
             budget is small.",
            400,
            16.0,
        )];
        let res = backend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: Some(80.0),
        });
        assert!(res.width <= 80.0 + 0.5, "wrap budget exceeded: {}", res.width);
        assert!(res.line_count >= 2, "expected ≥2 lines, got {}", res.line_count);
        assert!(res.height > 16.0, "wrapped text should be taller than one line");
    }

    #[test]
    fn empty_runs_return_zero_metrics() {
        let backend = SkiaMeasure::new();
        let res = backend.measure(&MeasureRequest {
            runs: &[],
            line_height: 0.0,
            max_width: None,
        });
        assert_eq!(res.width, 0.0);
        assert_eq!(res.height, 0.0);
        assert_eq!(res.line_count, 0);
    }
}
