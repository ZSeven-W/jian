//! Drift gate — under the `textlayout` feature, the layout engine's
//! laid-out rect for a text leaf must agree with what
//! `skia_safe::textlayout::Paragraph` reports as the natural width.
//!
//! Default-feature builds rely on the character-count estimator,
//! which deliberately disagrees with Skia (Latin ~10% off, CJK ~50%
//! off). The whole point of the `MeasureBackend` trait is to close
//! that gap when shaping is available. This test confirms it
//! actually does — without it, a regression in the trait wiring
//! could silently route layout through the estimator while the
//! renderer keeps shaping, reintroducing the drift the SkiaMeasure
//! backend was meant to eliminate.
//!
//! Compiles + runs only under `--features textlayout`. Default
//! `cargo test -p jian-skia` skips this file via `#[cfg(...)]`.

#![cfg(feature = "textlayout")]

use jian_core::layout::measure::{
    EstimateBackend, MeasureBackend, MeasureRequest, StyledRun,
};
use jian_core::Runtime;
use jian_ops_schema::load_str;
use jian_skia::SkiaMeasure;
use std::rc::Rc;

const TEXT_DOC: &str = r##"{
  "formatVersion":"1.0", "version":"1.0.0", "id":"x",
  "app": { "name":"x", "version":"1", "id":"x" },
  "children": [
    { "type":"frame", "id":"root", "width":600, "height":80,
      "children": [
        { "type":"text", "id":"label",
          "content":"Hello world",
          "fontSize":18, "fontWeight":400 }
      ]
    }
  ]
}"##;

fn rt() -> Runtime {
    let schema = load_str(TEXT_DOC).unwrap().value;
    Runtime::new_from_document(schema).unwrap()
}

#[test]
fn skia_measure_layout_agrees_with_paragraph_intrinsic_width() {
    let measure: Rc<dyn MeasureBackend> = Rc::new(SkiaMeasure::new());
    let mut runtime = rt();
    runtime
        .build_layout_with(measure.clone(), (600.0, 80.0))
        .unwrap();
    runtime.rebuild_spatial();

    let key = runtime.document.as_ref().unwrap().tree.get("label").unwrap();
    let laid_out = runtime.layout.node_rect(key).expect("text rect");

    // Independently ask the same backend for the same paragraph's
    // natural width. The layout pipeline should produce the same
    // value to within Skia's float rounding.
    let runs = [StyledRun {
        text: "Hello world",
        font_family: None,
        font_size: 18.0,
        font_weight: 400,
        font_style: jian_core::layout::measure::FontStyleKind::Normal,
        letter_spacing: 0.0,
    }];
    let direct = measure.measure(&MeasureRequest {
        runs: &runs,
        line_height: 0.0,
        max_width: None,
    });

    assert!(
        (laid_out.size.width - direct.width).abs() < 1.0,
        "laid-out width ({}) drifted from paragraph natural width ({}) by > 1px",
        laid_out.size.width,
        direct.width,
    );
    assert!(
        (laid_out.size.height - direct.height).abs() < 1.0,
        "laid-out height ({}) drifted from paragraph natural height ({}) by > 1px",
        laid_out.size.height,
        direct.height,
    );
}

#[test]
fn skia_measure_disagrees_with_estimator_on_cjk() {
    // Sanity: confirm the *estimator* materially under-shoots for
    // CJK (the bug the whole backend exists to fix). If a future
    // estimator change closes the gap on its own this test will
    // false-fire — that's a useful canary, not a regression.
    let cjk_runs = [StyledRun {
        text: "你好世界",
        font_family: None,
        font_size: 18.0,
        font_weight: 400,
        font_style: jian_core::layout::measure::FontStyleKind::Normal,
        letter_spacing: 0.0,
    }];
    let req = MeasureRequest {
        runs: &cjk_runs,
        line_height: 0.0,
        max_width: None,
    };
    let estimator = EstimateBackend.measure(&req);
    let skia = SkiaMeasure::new().measure(&req);
    assert!(
        skia.width > estimator.width * 1.2,
        "CJK shaper width should be > 20% wider than estimator's; \
         estimator={}, skia={}",
        estimator.width,
        skia.width,
    );
}
