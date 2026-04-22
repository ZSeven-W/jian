# jian-skia

Skia-backed [`RenderBackend`][backend] for the Jian runtime. Converts
`jian-core` scene primitives (`DrawOp::Rect` / `RoundedRect` / `Path` /
`Image` / `Text`) into Skia canvas operations via
[`skia-safe`](https://crates.io/crates/skia-safe).

## Usage

```rust
use jian_core::geometry::{rect, size};
use jian_core::render::{DrawOp, Paint, RenderBackend};
use jian_core::scene::Color;
use jian_skia::SkiaBackend;

let mut backend = SkiaBackend::new();
let mut surface = backend.new_surface(size(400.0, 300.0));

backend.begin_frame(&mut surface, 0xffffffff); // white clear
backend.draw_on(
    &mut surface,
    &DrawOp::Rect {
        rect: rect(40.0, 40.0, 320.0, 220.0),
        paint: Paint::solid(Color::rgb(0x1e, 0x88, 0xe5)),
    },
);
backend.end_frame(&mut surface);

std::fs::write("out.png", surface.encode_png().unwrap()).unwrap();
```

## Features

| Feature      | Default | Effect                                                    |
|--------------|---------|-----------------------------------------------------------|
| `metal`      | off     | GPU surface on macOS / iOS via Metal                      |
| `d3d`        | off     | GPU surface on Windows via D3D12                          |
| `gl`         | off     | GPU surface on Linux / Android / WebGL via OpenGL ES      |
| `vulkan`     | off     | GPU surface on Linux / Android via Vulkan                 |
| `textlayout` | off     | Full `ParagraphBuilder` text shaping (ICU + HarfBuzz)     |

The **default** feature set is empty — raster surfaces only. This keeps
`cargo test` fast and free of platform-specific GPU context setup.
Host crates (jian-host-desktop in Plan 8, WASM integration in Plan 12)
opt into the per-platform GPU feature they need.

## Status

MVP (`v0.1.0-skia`):

- ✅ `RenderBackend` trait implementation on raster surfaces
- ✅ Rect / RoundedRect / Path / Text / Image draw ops
- ✅ `apply_blur` / `apply_shadow` → Skia `ImageFilter`
- ✅ End-to-end test loading `.op` → layout → render → PNG
- ⏳ Per-platform GPU surface factories (Plan 8)
- ⏳ Gradient / image fills from `PenFill` spec (current: solid only)
- ⏳ `textlayout` feature (current: single-line `canvas.draw_str`)
- ⏳ Golden PNG corpus with pixel-diff harness (Plan 10)

## Dependencies

- [`skia-safe`](https://crates.io/crates/skia-safe) 0.78 — bundles
  pre-built Skia binaries for the major triples; no system Skia install
  required on supported platforms.

## License

MIT

[backend]: https://docs.rs/jian-core/latest/jian_core/render/trait.RenderBackend.html
