# jian-host-desktop

winit-based desktop host for the Jian runtime. Translates OS window
events into `jian_core::gesture::PointerEvent`s / keyboard events, wires
native platform services (clipboard / storage / router) into the
`ActionContext`, and (behind the `run` feature) drives a `SkiaBackend`
over a real window surface.

## Pieces

| Module     | What it does                                                      |
|------------|-------------------------------------------------------------------|
| `host`     | `DesktopHost` — owns `Runtime` + `SkiaBackend` + `HostConfig`     |
| `pointer`  | `PointerTranslator`: winit → `PointerEvent` with cursor state     |
| `keyboard` | `translate_key` + `modifiers_from_winit` — winit key → jian key   |
| `services` | In-memory `HistoryRouter` + `InMemoryStorage` (MVP)               |
| `bin/`     | `jian-player PATH` — loads a `.op`, prints node count             |

## Feature flags

| Feature     | Default | Effect                                                             |
|-------------|---------|--------------------------------------------------------------------|
| `run`       | off     | Enables `DesktopHost::run` — winit event loop + softbuffer presenter |
| `clipboard` | off     | Pulls in `arboard` and exposes `DesktopClipboard`                  |

Both are off by default so `cargo test -p jian-host-desktop` stays
headless and portable across the CI matrix. `jian-cli`'s `player`
feature (on by default) activates `jian-host-desktop/run`
transitively.

## Real font metrics — backend hook

Default builds use the in-core character-count estimator (good to
~10% on Latin, undershoots ~50% on CJK). To get glyph-accurate
measurement that agrees with what jian-skia paints, hosts install
a `MeasureBackend` once at startup and call
`Runtime::build_layout_with` instead of `build_layout`. The
runtime then mutates the engine's backend slot in place; every
subsequent `build_layout(size)` reuses the installed backend until
it's swapped again.

The intended production backend is `SkiaMeasure` over
`skia_safe::textlayout::Paragraph`, gated by `jian-skia`'s
`textlayout` cargo feature. **It is not yet implemented** — Tasks
2 and 3 of the font-metrics plan still wait on the textlayout
build environment. Headless tests and the CI fast-path keep the
default `EstimateBackend` so neither the skia-bindings build nor
a system-font scan is required.

When `SkiaMeasure` lands, the install pattern will look like:

```rust
// future — requires `jian-skia` built with `--features textlayout`,
// expected once font-metrics plan Task 2 ships:
//
// let measure = std::rc::Rc::new(jian_skia::measure::SkiaMeasure::new());
// runtime.build_layout_with(measure, (w, h))?;
```

`text_growth` semantics in the schema are already honoured by
every backend (Task 4 shipped):

- `auto` — wrap to the container's available width.
- `fixed-width` — wrap to the node's authored numeric width;
  falls back to `auto` semantics when the node was authored as
  `width: auto` (no fixed budget to honour).
- `fixed-width-height` — no wrap; report natural extent and let
  the renderer clip.

## Status

MVP (`v0.1.0-desktop`):

- ✅ Pure-function winit → pointer / keyboard translators (16 unit tests)
- ✅ `HistoryRouter` + `InMemoryStorage` service stubs
- ✅ `DesktopHost` composition root + `HostConfig`
- ✅ `jian-player PATH` loads a `.op` via `Runtime::new_from_document`
- ✅ `DesktopHost::run` winit event loop + softbuffer CPU presenter
  (Plan 8 T5, under the `run` feature)
- ✅ Scene walker `scene::collect_draws` — solid-fill rectangles
- ⏳ Platform-specific Skia surface factories (Plan 8 T2)
- ⏳ Native menus / deep links / auto-updater (Plan 8 T7-T9)
- ⏳ `reqwest`-backed `NetworkClient` and `rusqlite` storage

## License

MIT
