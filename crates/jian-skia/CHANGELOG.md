# Changelog

## [0.1.0] — Plan 7 — jian-skia MVP

### Added

- `SkiaBackend` implementing `jian_core::render::RenderBackend`:
  - `new_surface` / `begin_frame` / `end_frame` against a raster
    Skia `Surface`.
  - `draw_on(surface, op)` for `DrawOp::Rect`, `RoundedRect`, `Path`,
    `Image` (grey placeholder MVP), `Text` (single-line `draw_str`).
  - `apply_blur` / `apply_shadow` build `ImageFilter`s.
- `SkiaSurface`: CPU raster surface wrapper with `encode_png()` for
  test harnesses.
- Feature gates for per-platform GPU backends: `metal` / `d3d` / `gl` /
  `vulkan`. Optional `textlayout` for full `ParagraphBuilder` shaping.
- Conversions: `Color` → `Color4f`, `Rect` → `SkRect`, `Point` →
  `SkPoint`, `Affine2` → `Matrix`, `PathCommand[]` → `Path`.
- 13 unit tests + end-to-end test that loads a `.op`, runs `Runtime`
  layout, and renders through `SkiaBackend` to a valid PNG byte stream.

### Known limitations

- `push_clip` / `push_transform` / `push_layer` are no-ops. The
  Plan 2 trait is canvas-less so these need a trait-level revision
  (parked for Plan 8 once the desktop host lands and exercises them).
- Text uses single-line `canvas.draw_str` instead of `ParagraphBuilder`.
  Multi-line / CJK-aware shaping arrives when the `textlayout` feature
  is enabled + wired through (Plan 8+).
- `DrawOp::Image` paints a grey placeholder pending image cache
  (Task 8 lands with Plan 12 once a network-aware host provides bytes).
