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

### Building with `textlayout`

`skia-bindings` 0.78's bundled `depot_tools` invokes
`gclient_utils.py`, which `import pipes` — a stdlib module Python
3.13 removed. The depot_tools `ninja` wrapper (`ninja` on
POSIX, `ninja.bat` on Windows) `exec`s `python3 ninja.py`, so
whichever `python3` is first on PATH decides the outcome:
Python 3.10 / 3.11 / 3.12 still expose `pipes`; 3.13+ doesn't.

Easiest path is the helper script in `scripts/build-textlayout.sh`,
which auto-discovers a working interpreter (Homebrew keg first,
then `python3.{11,12,10}` on PATH) and prepends it for the cargo
invocation:

```bash
scripts/build-textlayout.sh build -p jian-skia
scripts/build-textlayout.sh test  -p jian-skia
```

The script always appends `--features textlayout`; pass any other
cargo args verbatim. Manual override works the same way:

```bash
PATH="/opt/homebrew/opt/python@3.11/libexec/bin:$PATH" \
  cargo build -p jian-skia --features textlayout
```

The `libexec/bin` directory exposes a generic `python3` symlink
(not just `python3.11`); on a Linux box install 3.11 via your
package manager (`apt install python3.11`) and either rely on the
helper's PATH-lookup path or prepend manually. Windows: same
principle — `setup-python` action in CI handles it; locally the
`py -3.11 -m` launcher or a 3.11 install in `%PATH%` works.

A future skia-bindings bump should drop the `pipes` dependency
upstream — track <https://github.com/rust-skia/rust-skia/issues>.
When that lands, delete `scripts/build-textlayout.sh` and the
`textlayout` CI matrix in `.github/workflows/ci.yml`.

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
