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
