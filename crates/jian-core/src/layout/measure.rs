//! Text measurement backend.
//!
//! `LayoutEngine` calls `MeasureBackend::measure` once per text-leaf
//! during a layout pass. Backends can be either:
//!
//! - The default `EstimateBackend` — character-count heuristic tuned
//!   against Inter / Space Grotesk; good to within ~10% for Latin.
//!   Used by every unit test and the headless CI fast-path so no
//!   font engine is required.
//! - A host-supplied real-shaping backend (jian-skia's `SkiaMeasure`
//!   under the `textlayout` feature) installed via
//!   `Runtime::build_layout_with`.
//!
//! `&self` is intentional: layout invokes measure many times per
//! frame and Taffy's measure callback hands us an immutable context.
//! Implementations that cache typefaces or paragraph builders are
//! responsible for their own interior mutability (`RefCell` for
//! single-threaded, `parking_lot::Mutex` / `OnceLock` if a future
//! consumer needs `Send + Sync`).
//!
//! `MeasureRequest::runs` is non-empty: a `TextContent::Plain` node
//! arrives as a one-element vec inheriting the node-level font
//! fields; `TextContent::Styled` flattens to one run per segment.
//! Per-segment shaping matters because the renderer
//! (jian-skia ParagraphBuilder under `textlayout`) shapes each
//! segment with its own style — measuring with a single concatenated
//! string would diverge from rendered output the moment two
//! segments have different weights or sizes.

use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyleKind {
    Normal,
    Italic,
}

/// One styled run within a text node. Mirrors
/// `jian_ops_schema::style::StyledTextSegment` plus already-resolved
/// numeric weight + style. The `text` borrows from a node-local
/// store so backends can shape without copying.
#[derive(Debug, Clone)]
pub struct StyledRun<'a> {
    pub text: &'a str,
    pub font_family: Option<&'a str>,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_style: FontStyleKind,
    pub letter_spacing: f32,
}

/// Backend input for one text-leaf measurement.
#[derive(Debug, Clone)]
pub struct MeasureRequest<'a> {
    pub runs: &'a [StyledRun<'a>],
    /// Multiplier — 0.0 means "engine default" (1.3× the largest
    /// segment size). Backends honour authored values literally.
    pub line_height: f32,
    /// Wrap budget. `None` means "natural single-line extent";
    /// `Some(w)` means wrap to that width and report multi-line
    /// height.
    pub max_width: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct MeasureResult {
    pub width: f32,
    pub height: f32,
    pub line_count: u16,
    /// Distance from the top of the layout box to the first-line
    /// baseline. The renderer uses this to align text vertically;
    /// the estimator returns `font_size * 0.8` as a rough average.
    pub baseline: f32,
}

pub trait MeasureBackend {
    fn measure(&self, req: &MeasureRequest<'_>) -> MeasureResult;

    /// Monotonic process-global font registry generation observed by this
    /// backend. Estimating/headless backends have no registry and stay at 0.
    fn font_generation(&self) -> u64 {
        0
    }
}

/// Default backend: character-count heuristic. Sufficient for
/// unit tests + the headless CI path; *not* sufficient for
/// production rendering under the `textlayout` feature.
#[derive(Debug, Default, Clone, Copy)]
pub struct EstimateBackend;

impl MeasureBackend for EstimateBackend {
    fn measure(&self, req: &MeasureRequest<'_>) -> MeasureResult {
        if req.runs.is_empty() {
            return MeasureResult {
                width: 0.0,
                height: 0.0,
                line_count: 0,
                baseline: 0.0,
            };
        }
        // Concatenate run text and pick the largest size + heaviest
        // weight as the representative for the heuristic. A future
        // backend can do per-run shaping; here we approximate.
        let mut content = String::new();
        let mut max_size = 0.0_f32;
        let mut max_weight: u16 = 0;
        for run in req.runs {
            content.push_str(run.text);
            if run.font_size > max_size {
                max_size = run.font_size;
            }
            if run.font_weight > max_weight {
                max_weight = run.font_weight;
            }
        }
        let natural_w = estimate_runs_width(req.runs, max_size, max_weight);
        let line_mult = if req.line_height > 0.0 {
            req.line_height
        } else {
            1.3
        };

        // `str::lines` intentionally omits the final empty line. Text layout
        // cannot do that: an authored trailing newline still advances one
        // line. Keep the count as `usize` while calculating height so very
        // large documents do not wrap through the public `u16` count.
        let explicit_lines = content.split('\n').count().max(1);
        let reported_explicit_lines = explicit_lines.min(usize::from(u16::MAX)) as u16;

        let (width, height, line_count) = match req.max_width {
            Some(budget) if natural_w > budget + 0.5 && budget > 0.0 => {
                let lines = estimate_wrapped_line_count(req.runs, max_size, max_weight, budget);
                let reported_lines = lines.min(f32::from(u16::MAX)) as u16;
                (budget, max_size * line_mult * lines, reported_lines)
            }
            _ => {
                let height = max_size * line_mult * explicit_lines as f32;
                (natural_w, height, reported_explicit_lines)
            }
        };
        MeasureResult {
            width,
            height,
            line_count,
            baseline: max_size * 0.8,
        }
    }
}

/// Compatibility shim — older call sites that just want a `(w, h)`
/// for a plain string. Internally builds a single-run request
/// against `EstimateBackend`. Prefer `MeasureBackend` for new code.
pub fn estimate_text_size(content: &str, font_size: f32) -> (f32, f32) {
    estimate_text_size_weighted(content, font_size, 400)
}

/// Compatibility shim for weighted estimates. Same disclaimer as
/// `estimate_text_size`.
pub fn estimate_text_size_weighted(content: &str, font_size: f32, weight: u16) -> (f32, f32) {
    estimate_concat(content, font_size, weight)
}

/// Shared character-count heuristic used by the compatibility shims
/// and by `EstimateBackend`. Splits on `\n`, picks the widest line,
/// scales by `font_size * weight_ratio`.
fn estimate_concat(content: &str, font_size: f32, weight: u16) -> (f32, f32) {
    let lines: Vec<&str> = content.split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    // Regular weight ~ 0.55; semibold 600 ~ 0.60; bold 700+ ~ 0.64.
    let ratio = if weight >= 700 {
        0.64
    } else if weight >= 600 {
        0.60
    } else {
        0.58
    };
    let width = widest as f32 * font_size * ratio;
    let height = lines.len() as f32 * font_size * 1.3;
    (width, height)
}

/// Estimate the widest CSS line while preserving authored tracking. Intrinsic
/// inline width includes tracking after the final character as well as between
/// characters, so every character contributes its run's spacing.
fn estimate_runs_width(runs: &[StyledRun<'_>], font_size: f32, weight: u16) -> f32 {
    let ratio = if weight >= 700 {
        0.64
    } else if weight >= 600 {
        0.60
    } else {
        0.58
    };
    let char_width = font_size * ratio;
    let mut widest = 0.0_f32;
    let mut line_width = 0.0_f32;

    for run in runs {
        let spacing = if run.letter_spacing.is_finite() {
            run.letter_spacing
        } else {
            0.0
        };
        for ch in run.text.chars() {
            if ch == '\n' {
                widest = widest.max(line_width.max(0.0));
                line_width = 0.0;
                continue;
            }
            line_width += char_width + spacing;
        }
    }

    widest.max(line_width.max(0.0))
}

/// Estimate wrapping independently for every authored line. Dividing only the
/// widest line by the budget loses the other explicit lines, while dividing
/// the concatenated width would incorrectly let one line's spare space absorb
/// another. Empty lines (including a trailing one) always contribute once.
fn estimate_wrapped_line_count(
    runs: &[StyledRun<'_>],
    font_size: f32,
    weight: u16,
    budget: f32,
) -> f32 {
    let ratio = if weight >= 700 {
        0.64
    } else if weight >= 600 {
        0.60
    } else {
        0.58
    };
    let char_width = font_size * ratio;
    let mut total_lines = 0.0_f32;
    let mut line_width = 0.0_f32;

    let finish_line = |width: f32| (width.max(0.0) / budget).ceil().max(1.0);
    for run in runs {
        let spacing = if run.letter_spacing.is_finite() {
            run.letter_spacing
        } else {
            0.0
        };
        for ch in run.text.chars() {
            if ch == '\n' {
                total_lines += finish_line(line_width);
                line_width = 0.0;
            } else {
                line_width += char_width + spacing;
            }
        }
    }
    total_lines + finish_line(line_width)
}

/// Convenience constructor — most call sites want
/// `Rc::new(EstimateBackend)`.
pub fn default_backend() -> Rc<dyn MeasureBackend> {
    Rc::new(EstimateBackend)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<'a>(text: &'a str, weight: u16) -> StyledRun<'a> {
        StyledRun {
            text,
            font_family: None,
            font_size: 16.0,
            font_weight: weight,
            font_style: FontStyleKind::Normal,
            letter_spacing: 0.0,
        }
    }

    #[test]
    fn estimate_single_line() {
        let (w, h) = estimate_text_size("Hello", 16.0);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn estimate_multi_line() {
        let (w_one, h_one) = estimate_text_size("Hi", 16.0);
        let (w_two, h_two) = estimate_text_size("Hi\nThere", 16.0);
        assert!(w_two >= w_one);
        assert!(h_two > h_one);
    }

    #[test]
    fn backend_measures_single_run() {
        let runs = vec![run("Hello", 400)];
        let req = MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: None,
        };
        let res = EstimateBackend.measure(&req);
        assert!(res.width > 0.0);
        assert!(res.height > 0.0);
        assert_eq!(res.line_count, 1);
    }

    #[test]
    fn backend_picks_max_size_across_runs() {
        // A bold run should widen the bounding box: with two runs
        // of mixed weight the heuristic uses the heavier ratio.
        let runs_uniform = vec![run("Hi there", 400)];
        let mut runs_mixed = vec![run("Hi ", 400)];
        runs_mixed.push(run("there", 700));
        let res_uniform = EstimateBackend.measure(&MeasureRequest {
            runs: &runs_uniform,
            line_height: 0.0,
            max_width: None,
        });
        let res_mixed = EstimateBackend.measure(&MeasureRequest {
            runs: &runs_mixed,
            line_height: 0.0,
            max_width: None,
        });
        assert!(
            res_mixed.width >= res_uniform.width,
            "mixed-weight run should not measure narrower than uniform 400"
        );
    }

    #[test]
    fn backend_wraps_to_max_width() {
        let runs = vec![run(
            "This is a fairly long line that should wrap when budget is small",
            400,
        )];
        let res = EstimateBackend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 0.0,
            max_width: Some(40.0),
        });
        assert_eq!(res.width, 40.0);
        assert!(res.line_count > 1);
        assert!(res.height > 16.0 * 1.3);
    }

    #[test]
    fn backend_applies_positive_and_negative_letter_spacing() {
        let mut plain = run("NOVA.", 700);
        let base = EstimateBackend.measure(&MeasureRequest {
            runs: std::slice::from_ref(&plain),
            line_height: 0.0,
            max_width: None,
        });

        plain.letter_spacing = -1.5;
        let tightened = EstimateBackend.measure(&MeasureRequest {
            runs: std::slice::from_ref(&plain),
            line_height: 0.0,
            max_width: None,
        });
        assert!((base.width - tightened.width - 7.5).abs() < f32::EPSILON);

        plain.letter_spacing = 2.0;
        let expanded = EstimateBackend.measure(&MeasureRequest {
            runs: std::slice::from_ref(&plain),
            line_height: 0.0,
            max_width: None,
        });
        assert!((expanded.width - base.width - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn backend_applies_each_runs_tracking_to_its_characters() {
        let mut left = run("AB", 400);
        left.letter_spacing = -2.0;
        let mut right = run("CD", 400);
        right.letter_spacing = 3.0;
        let measured = EstimateBackend.measure(&MeasureRequest {
            runs: &[left, right],
            line_height: 0.0,
            max_width: None,
        });
        let base = 4.0 * 16.0 * 0.58;
        // Every character contributes its run's spacing: 2*(-2) + 2*(+3).
        assert!((measured.width - (base + 2.0)).abs() < 0.001);
    }

    #[test]
    fn backend_intrinsic_width_includes_trailing_tracking() {
        let mut heading = run("逛逛分类", 700);
        let base = EstimateBackend.measure(&MeasureRequest {
            runs: std::slice::from_ref(&heading),
            line_height: 0.0,
            max_width: None,
        });
        heading.letter_spacing = -1.0;
        let tightened = EstimateBackend.measure(&MeasureRequest {
            runs: std::slice::from_ref(&heading),
            line_height: 0.0,
            max_width: None,
        });
        assert!((base.width - tightened.width - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn backend_natural_height_honors_authored_line_height() {
        let runs = [StyledRun {
            text: "Navigation",
            font_family: None,
            font_size: 15.0,
            font_weight: 700,
            font_style: FontStyleKind::Normal,
            letter_spacing: 0.0,
        }];
        let measured = EstimateBackend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 1.5,
            max_width: None,
        });
        assert_eq!(measured.line_count, 1);
        assert!((measured.height - 22.5).abs() < f32::EPSILON);
    }

    #[test]
    fn backend_preserves_trailing_empty_line() {
        let runs = [run("text\n", 400)];
        let measured = EstimateBackend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 1.0,
            max_width: None,
        });
        assert_eq!(measured.line_count, 2);
        assert!((measured.height - 32.0).abs() < f32::EPSILON);
    }

    #[test]
    fn backend_clamps_reported_line_count_without_wrapping_height() {
        let content = "x\n".repeat(usize::from(u16::MAX));
        let runs = [run(&content, 400)];
        let measured = EstimateBackend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 1.0,
            max_width: None,
        });
        assert_eq!(measured.line_count, u16::MAX);
        assert_eq!(measured.height, 16.0 * (f32::from(u16::MAX) + 1.0));
    }

    #[test]
    fn backend_wraps_each_explicit_line_independently() {
        let runs = [run("a\nb\nc\n", 400)];
        let measured = EstimateBackend.measure(&MeasureRequest {
            runs: &runs,
            line_height: 1.0,
            max_width: Some(5.0),
        });
        // Each glyph wraps to two lines at this width, and the authored
        // trailing newline contributes one final empty line.
        assert_eq!(measured.line_count, 7);
        assert_eq!(measured.height, 16.0 * 7.0);
    }
}
