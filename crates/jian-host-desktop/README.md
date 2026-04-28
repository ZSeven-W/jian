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

| Feature       | Default | Effect                                                             |
|---------------|---------|--------------------------------------------------------------------|
| `run`         | off     | Enables `DesktopHost::run` — winit event loop + softbuffer presenter |
| `clipboard`   | off     | Pulls in `arboard` and exposes `DesktopClipboard`                  |
| `menus`       | off     | Pulls in `muda` for the native menu bar                            |
| `textlayout`  | off     | Forwards to `jian-skia/textlayout`; activates `with_skia_measure()` |
| `dev-asp`     | off     | Dev-only Agent Shell Protocol (Plan 18)                             |
| `mcp`         | off     | Wires `jian-action-surface` MCP drain into the run loop             |

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

The production backend is `SkiaMeasure` over
`skia_safe::textlayout::Paragraph`, gated by `jian-skia`'s
`textlayout` cargo feature. As of font-metrics plan T2 (2026-04-28)
the host crate exposes a `textlayout` feature that pulls the same
flag through transitively, plus convenience builders on
`DesktopHost`:

```rust
// jian-host-desktop = { ..., features = ["run", "textlayout"] }
//
// Builder form — common case, install once when the host is built.
let host = DesktopHost::new(runtime, "MyApp")
    .with_default_menu()
    .with_skia_measure();    // available under feature = "textlayout"
host.run()?;

// In-place form — for hosts that already own the DesktopHost and
// want to swap shaping in / out (e.g. after a hot-reload).
host.install_skia_measure();

// Manual form — when the host wants a custom FontMgr (bundled
// fonts, sandboxed font dir, etc.):
let measure = std::rc::Rc::new(
    jian_skia::measure::SkiaMeasure::with_font_manager(custom_fm)
);
runtime.build_layout_with(measure, (w, h))?;
```

The `textlayout` build pulls in skia-safe's ICU + HarfBuzz layer
(~15 MB binary growth + a Python 3.10 / 3.11 / 3.12 toolchain for
skia-bindings' depot_tools — see
`scripts/build-textlayout.sh`). Headless tests and the CI
fast-path keep the default `EstimateBackend` so the heavier deps
aren't pulled into ordinary `cargo test`.

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
