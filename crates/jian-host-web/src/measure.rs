//! CanvasKit Paragraph-backed layout measurement.

use crate::canvaskit::CkRuntime;
use jian_core::layout::measure::{FontStyleKind, MeasureBackend, MeasureRequest, MeasureResult};
use js_sys::Array;
use wasm_bindgen::JsValue;

pub struct CkMeasure {
    runtime: CkRuntime,
}

impl CkMeasure {
    pub(crate) fn new(runtime: CkRuntime) -> Self {
        Self { runtime }
    }
}

impl MeasureBackend for CkMeasure {
    fn measure(&self, request: &MeasureRequest<'_>) -> MeasureResult {
        if request.runs.is_empty() {
            return MeasureResult {
                width: 0.0,
                height: 0.0,
                line_count: 0,
                baseline: 0.0,
            };
        }
        let texts = Array::new();
        let families = Array::new();
        let mut sizes = Vec::with_capacity(request.runs.len());
        let mut weights = Vec::with_capacity(request.runs.len());
        let mut italics = Vec::with_capacity(request.runs.len());
        let mut spacing = Vec::with_capacity(request.runs.len());
        for run in request.runs {
            texts.push(&JsValue::from_str(run.text));
            families.push(&JsValue::from_str(run.font_family.unwrap_or("")));
            sizes.push(run.font_size);
            weights.push(run.font_weight);
            italics.push(u8::from(run.font_style == FontStyleKind::Italic));
            spacing.push(run.letter_spacing);
        }
        let values = self.runtime.measure_paragraph(
            &texts,
            &families,
            &sizes,
            &weights,
            &italics,
            &spacing,
            request.max_width.unwrap_or(-1.0),
            request.line_height,
        );
        MeasureResult {
            width: values.first().copied().unwrap_or(0.0),
            height: values.get(1).copied().unwrap_or(0.0),
            line_count: values.get(2).copied().unwrap_or(0.0).round() as u16,
            baseline: values.get(3).copied().unwrap_or(0.0),
        }
    }
}
