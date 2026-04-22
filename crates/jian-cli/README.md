# jian (CLI)

Command-line toolchain for the Jian runtime (`.op` files).

```bash
cargo install --path crates/jian-cli
jian --help
```

## Commands

| Command | What it does |
|---------|--------------|
| `jian check PATH [--json]` | Parse + validate a `.op`; print diagnostics. Exit 0 clean, 1 warnings, 2 parse error. |
| `jian pack INPUT OUTPUT` | Zip a `.op` into a `.op.pack` with a generated `manifest.json`. |
| `jian unpack INPUT OUT_DIR` | Inverse of `pack`. Guards against zip-slip. |
| `jian new NAME [--template counter\|form] [--path DIR]` | Scaffold a new project from an embedded template with `{{APP_NAME}}` / `{{APP_ID}}` placeholders substituted. |

## Roadmap

- `jian player PATH` — open a `.op` in a desktop window (lands once
  `jian-host-desktop` ships its real event loop under the `run` feature).
- `jian dev PATH` — hot-reload variant of `player` backed by the
  `notify` crate.
- Distribution: Homebrew tap, `winget` manifest, Linux install script
  (`cargo dist` or a GitHub Actions matrix).

## License

MIT
