# Changelog

## [0.1.0] — Plan 9 — jian-cli MVP

### Added

- `jian check PATH [--json]`: load + parse via
  `jian_ops_schema::load_str`, emit human or NDJSON diagnostics.
  Exit 0 clean, 1 warnings, 2 parse error.
- `jian pack INPUT OUTPUT`: write a deflate-compressed `.op.pack`
  containing `manifest.json` (app id/name/version + declared
  capabilities) plus the original JSON as `app.op`.
- `jian unpack INPUT OUT_DIR`: extract every entry; zip-slip guard on
  entry names.
- `jian new NAME [--template counter|form] [--path DIR]`: scaffold
  a fresh project by writing the chosen embedded template with
  `{{APP_NAME}}` and `{{APP_ID}}` substitutions. `--template form`
  is also available.
- `slugify` helper: kebab-case `APP_ID` generation (lowercase,
  ASCII-alphanumeric + `-`, collapse runs, strip dashes).
- 7 integration tests via `assert_cmd`: clean/warning/malformed
  check paths, `--json` ndjson, new→check round-trip, pack/unpack
  byte-exact roundtrip.

### Not yet shipped

- `jian player PATH` — waiting on the winit event loop in
  `jian-host-desktop` (`run` feature, Plan 8 follow-up).
- `jian dev PATH` — hot-reload variant of `player`.
- Distribution configs (`cargo dist`, Homebrew, winget).
