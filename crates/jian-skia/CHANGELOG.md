# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- `SkiaBackend` implementing `jian_core::render::RenderBackend`:
  `new_surface` / `begin_frame` / `end_frame` against a raster Skia
  `Surface`; `draw_on(surface, op)` for `DrawOp::Rect`, `RoundedRect`,
  `Path`, `Image` (grey placeholder MVP), `Text` (single-line
  `draw_str`); `apply_blur` / `apply_shadow` build `ImageFilter`s.
- `SkiaSurface` CPU raster surface wrapper with `encode_png()` for
  test harnesses.
- Feature gates for per-platform GPU backends: `metal` / `d3d` / `gl`
  / `vulkan`. Each ships as a feature-gated skeleton in
  `surface/{metal,d3d,gl}.rs` returning a typed `Err("not yet
  implemented")` with full implementation outlines in the source —
  real CAMetalLayer / IDXGI swapchain / GL context lifecycles land in
  per-backend follow-up sessions against real hardware.
- Optional `textlayout` feature for full `ParagraphBuilder` shaping.
  Without it, `DrawOp::Text` falls back to the canvas' built-in
  single-line `draw_str` (sufficient for Stage A).
- Type conversions: `Color` → `Color4f`, `Rect` → `SkRect`, `Point` →
  `SkPoint`, `Affine2` → `Matrix`, `PathCommand[]` → `Path`.
- 13 unit tests + an end-to-end test that loads a `.op`, runs
  `Runtime` layout, and renders through `SkiaBackend` to a valid PNG
  byte stream.
- `push_clip` / `push_transform` / `push_layer` are no-ops pending a
  trait revision; `DrawOp::Image` paints a grey placeholder pending
  the image cache that lands with the network-aware host.
