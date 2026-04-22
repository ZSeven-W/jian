# Changelog

## [0.1.0] — Plan 8 — jian-host-desktop MVP

### Added

- `DesktopHost` composition root: owns `Runtime` + `SkiaBackend` +
  `HostConfig` (title + initial size).
- `pointer::PointerTranslator`: stateful winit → `PointerEvent`
  translator. Caches cursor position between events so that
  `MouseInput` (which carries no position) can fire a complete Down /
  Up. `CursorMoved` emits `Hover` when no button is held, `Move`
  otherwise. `Touch` events pass through with phase + finger id. 6
  unit tests covering phase transitions + modifier propagation.
- `keyboard::translate_key` + `modifiers_from_winit`: winit key →
  `(key_string, Modifiers)` with the web-ish naming convention
  (`Enter`, `ArrowLeft`, `Space`, …).
- `services::HistoryRouter` — in-process route stack implementing
  `jian_core::action::services::Router`.
- `services::InMemoryStorage` — BTreeMap-backed `StorageBackend` good
  enough for the MVP; real `rusqlite` lands under a future flag.
- Feature-gated `services::clipboard::DesktopClipboard` — `arboard`
  wrapper. Opt-in via the `clipboard` feature so headless CI skips it.
- `bin/jian-player PATH` — loads a `.op`, runs `Runtime::new_from_document`,
  builds layout, prints node count + initial size.

### Not yet shipped (see `README.md` → Status)

- winit event loop + redraw scheduling (Plan 8 T5).
- Per-platform Skia surface factories for Metal / D3D12 / OpenGL
  (Plan 8 T2).
- Native menus (muda), deep links, auto-updater, packaging (T7–T10).
- `reqwest` network client + SQLite storage.
