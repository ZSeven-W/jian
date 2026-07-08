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

use jian_core::layout::measure::{FontStyleKind, MeasureBackend, MeasureRequest, MeasureResult};
use skia_safe::{
    font_style::{Slant, Weight, Width},
    textlayout::{FontCollection, ParagraphBuilder, ParagraphStyle, TextStyle},
    FontMgr, FontStyle,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::font_resolve::FontResolver;

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

/// Canvas text paint advances literal-newline lines by `font_size * 1.2`
/// when no `lineHeight` is authored. Skia Paragraph's font-metrics default
/// is too tight for that native paint path, so measurement must use the
/// painter's line advance for explicit multiline content.
const DEFAULT_PAINT_LINE_HEIGHT: f32 = 1.2;

/// Measure backend that defers to skia's paragraph shaper.
///
/// Construct once at host startup, share the same `Rc` across the
/// whole runtime. The default `FontMgr` resolves system fonts on
/// every supported platform; hosts that bundle their own typefaces
/// can build via [`SkiaMeasure::with_font_manager`].
pub struct SkiaMeasure {
    font_collection: RefCell<Rc<FontCollection>>,
    font_resolver: FontResolver,
    built_generation: Cell<u64>,
}

impl SkiaMeasure {
    /// Use the platform's default `FontMgr` (system fonts).
    pub fn new() -> Self {
        // `FontMgr::default()` touches DirectWrite on Windows; serialize it
        // with all other font work (reentrant — `with_font_manager` re-locks
        // harmlessly).
        crate::font_lock::with_font_lock(|| Self::with_font_manager(FontMgr::default()))
    }

    /// Use a host-supplied `FontMgr` — for example one that wraps a
    /// `TypefaceFontProvider` populated with bundled `.ttf` blobs.
    pub fn with_font_manager(font_mgr: FontMgr) -> Self {
        crate::font_lock::with_font_lock(|| {
            // Read the generation BEFORE building the collection so an import
            // landing mid-construction is caught by the next `refresh_if_stale`
            // rather than recorded as already-applied against a stale
            // collection (see the same reasoning in `FontResolver`).
            let built_generation = crate::bundled_fonts::generation();
            let font_resolver = FontResolver::new(font_mgr);
            let fc = build_collection(&font_resolver);
            Self {
                font_collection: RefCell::new(Rc::new(fc)),
                font_resolver,
                built_generation: Cell::new(built_generation),
            }
        })
    }

    /// Rebuild the cached `FontCollection` when the font registry
    /// generation has advanced (a runtime import/removal), so an
    /// already-open document re-measures with the new faces instead of
    /// returning stale widths. Cheap atomic check on the common path.
    fn refresh_if_stale(&self) {
        let current = crate::bundled_fonts::generation();
        if current == self.built_generation.get() {
            return;
        }
        let fc = build_collection(&self.font_resolver);
        *self.font_collection.borrow_mut() = Rc::new(fc);
        self.built_generation.set(current);
    }

    fn resolved_natural_width(&self, req: &MeasureRequest<'_>) -> f32 {
        let mut max_line_width = 0.0_f32;
        let mut line_width = 0.0_f32;
        let mut line_chars = 0usize;
        let mut line_spacing = 0.0_f32;
        for run in req.runs {
            let italic = matches!(run.font_style, FontStyleKind::Italic);
            let mut parts = run.text.split('\n').peekable();
            while let Some(part) = parts.next() {
                line_width += self.font_resolver.measure_text(
                    part,
                    run.font_size,
                    run.font_family,
                    run.font_weight,
                    italic,
                );
                line_chars += part.chars().count();
                line_spacing = line_spacing.max(run.letter_spacing.max(0.0));
                if parts.peek().is_some() {
                    max_line_width = max_line_width
                        .max(line_width + letter_spacing_width(line_chars, line_spacing));
                    line_width = 0.0;
                    line_chars = 0;
                    line_spacing = 0.0;
                }
            }
        }
        max_line_width.max(line_width + letter_spacing_width(line_chars, line_spacing))
    }
}

/// Build a `FontCollection` whose default manager orders imported →
/// system → bundled → Roboto (via the resolver), with imported + bundled
/// faces also registered as an asset fallback so a system-missing family
/// still shapes with the right metrics here — otherwise `fit_content`
/// heights are computed from fallback glyphs and every text block lands
/// at the wrong height.
fn build_collection(font_resolver: &FontResolver) -> FontCollection {
    let mut fc = FontCollection::new();
    fc.set_default_font_manager(font_resolver.ordered_font_manager(), None);
    if let Some(provider) = crate::bundled_fonts::asset_provider() {
        fc.set_asset_font_manager(Some(provider.into()));
    }
    fc
}

fn letter_spacing_width(chars: usize, spacing: f32) -> f32 {
    if chars <= 1 {
        return 0.0;
    }
    chars.saturating_sub(1) as f32 * spacing
}

fn has_literal_newline(req: &MeasureRequest<'_>) -> bool {
    req.runs.iter().any(|run| run.text.contains('\n'))
}

fn painted_multiline_height(req: &MeasureRequest<'_>, line_count: u16) -> f32 {
    let max_size = req
        .runs
        .iter()
        .map(|run| run.font_size)
        .fold(0.0_f32, f32::max);
    let line_mult = if req.line_height > 0.0 {
        req.line_height
    } else {
        DEFAULT_PAINT_LINE_HEIGHT
    };
    f32::from(line_count) * max_size * line_mult
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
        // Serialize the whole shape/measure against every other DirectWrite
        // user (a paint pass, another thread's measure) — skia's Windows font
        // backend segfaults under concurrent access. Reentrant, so the
        // `refresh_if_stale` → `build_collection` → `asset_provider` nesting
        // below is fine.
        crate::font_lock::with_font_lock(|| self.measure_locked(req))
    }
}

impl SkiaMeasure {
    fn measure_locked(&self, req: &MeasureRequest<'_>) -> MeasureResult {
        self.refresh_if_stale();

        // ParagraphStyle is a per-paragraph default container;
        // line_height is applied per TextStyle below so individual
        // runs can opt in/out independently.
        let style = ParagraphStyle::new();
        let mut builder = ParagraphBuilder::new(&style, (**self.font_collection.borrow()).clone());

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
                // CSS / Pencil split the line-height leading half above +
                // half below the glyphs; skia's default proportional
                // distribution shifts each baseline, so measured text
                // height (and the baseline the renderer paints to) drifts
                // from Pencil. Keep measure + paint on the same rule.
                ts.set_half_leading(true);
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
            // single-line width, but the direct resolver path also
            // applies native synthetic-bold width compensation.
            None => self.resolved_natural_width(req),
        };
        let line_count = paragraph.line_number().max(1) as u16;
        let height = if has_literal_newline(req) {
            painted_multiline_height(req, line_count)
        } else {
            paragraph.height()
        };

        MeasureResult {
            width,
            height,
            // line_number is 0-indexed in skia; layout always
            // produces at least one logical line for non-empty
            // text, so report `max(1, n)`.
            line_count,
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
    fn measures_cjk_wider_than_estimator_when_font_available() {
        // Estimator under-shoots CJK by ~50% (ratio 0.58 vs ≈1.0
        // square glyph). Skia's shaper resolves a CJK fallback and
        // reports the real glyph width. We assert the shaper width
        // is strictly *more* than what the estimator would have
        // produced for the same string — that's the bug the whole
        // backend exists to fix.
        //
        // Linux CI images do not ship a Han font by default. When no
        // installed face covers the text, the direct resolver returns no
        // advance and this test only verifies the fallback is stable. On
        // machines with CJK coverage (macOS / Windows developer and CI
        // images), the test still catches the regression we care about:
        // the shaper agreeing with the Latin-biased estimator.
        let backend = SkiaMeasure::new();
        let runs = [run("你好", 400, 16.0)];
        let res = backend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: None,
        });
        let has_cjk_coverage = "你好".chars().all(|c| {
            backend
                .font_resolver
                .typeface_for_char(None, c, 400, false)
                .is_some()
        });
        if !has_cjk_coverage {
            assert!(
                res.width.is_finite() && res.width >= 0.0,
                "CJK measurement without a covering font should be stable, got {}",
                res.width,
            );
            return;
        }
        let estimator = 2.0 * 16.0 * 0.58; // 18.56
        assert!(
            res.width > estimator * 1.02,
            "CJK shaped width should exceed estimator's {}, got {}",
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
        assert!(
            res.width <= 80.0 + 0.5,
            "wrap budget exceeded: {}",
            res.width
        );
        assert!(
            res.line_count >= 2,
            "expected ≥2 lines, got {}",
            res.line_count
        );
        assert!(
            res.height > 16.0,
            "wrapped text should be taller than one line"
        );
    }

    #[test]
    fn multiline_height_matches_painted_line_count_times_line_height() {
        struct Case {
            text: &'static str,
            size: f32,
            line_height: f32,
            lines: u16,
        }

        let backend = SkiaMeasure::new();
        for case in [
            Case {
                text: "contact\nhello@darkbrew.cz",
                size: 16.0,
                line_height: 0.0,
                lines: 2,
            },
            Case {
                text: "first\nsecond\nthird",
                size: 18.0,
                line_height: 0.0,
                lines: 3,
            },
            Case {
                text: "contact\nhello@darkbrew.cz",
                size: 16.0,
                line_height: 1.5,
                lines: 2,
            },
            Case {
                text: "first\nsecond\nthird",
                size: 18.0,
                line_height: 1.5,
                lines: 3,
            },
        ] {
            let runs = [StyledRun {
                text: case.text,
                font_family: None,
                font_size: case.size,
                font_weight: 400,
                font_style: FontStyleKind::Normal,
                letter_spacing: 0.0,
            }];
            let measured = backend.measure(&MeasureRequest {
                runs: &runs,
                line_height: case.line_height,
                max_width: None,
            });
            let line_mult = if case.line_height > 0.0 {
                case.line_height
            } else {
                1.2
            };
            let expected = f32::from(case.lines) * case.size * line_mult;
            let rel = (measured.height - expected).abs() / measured.height.max(expected).max(1.0);
            println!(
                "multiline measure text={:?} size={} line_height={} measured={:.3} expected={:.3} lines={} diff={:.2}%",
                case.text,
                case.size,
                case.line_height,
                measured.height,
                expected,
                measured.line_count,
                rel * 100.0
            );
            assert_eq!(measured.line_count, case.lines);
            assert!(
                rel <= 0.02,
                "multiline height drift exceeded 2%: measured={:.3} expected={:.3} diff={:.2}%",
                measured.height,
                expected,
                rel * 100.0
            );
        }
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
