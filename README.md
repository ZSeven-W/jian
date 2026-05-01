<div align="center">

# Jian · 简

### *One `.op` file. A native window. AI in the loop. Pure Rust.*

Jian loads a single declarative document and turns it into a real, interactive,
hot-reloading, AI-controllable application — without a JS runtime, without a
DOM, without an Electron tax.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)
![Tests](https://img.shields.io/badge/tests-1014%20passing-brightgreen.svg)
![Workspace](https://img.shields.io/badge/version-0.0.1-purple.svg)
![Platforms](https://img.shields.io/badge/macOS%20%7C%20Linux%20%7C%20Windows-supported-success)
![MCP](https://img.shields.io/badge/MCP-rmcp%20stdio-9cf)
![Cold start](https://img.shields.io/badge/cold%20start-typed%20stages-success)
![Status](https://img.shields.io/badge/status-pre--release-yellow.svg)

[**Quick start**](#-quick-start) ·
[**Why Jian?**](#-why-jian) ·
[**Architecture**](#-architecture) ·
[**AI surface**](#-ai-action-surface) ·
[**Roadmap**](#-roadmap)

</div>

---

## ✨ Highlights

- 🦀 **Pure-Rust runtime** — `winit` · `skia-safe` · `taffy` · `rstar`. No JS, no DOM, no V8.
- 📄 **One file is the app** — state, layout, bindings, events, AI capabilities — all in `.op`.
- ⚡ **Solid-style reactivity** — `Signal<T>` + `Effect`, fine-grained, single-threaded, allocation-light.
- 🎯 **Real gesture arena** — Tap · LongPress · Swipe · Scroll · multi-pointer **Pinch & Rotate**, with bubble-style hit dispatch.
- 🎨 **Pixel-stable rendering** — Skia raster, per-corner radii, gradients, shadows, image cache, Lucide icons; opt-in `textlayout` for full Paragraph shaping.
- 🤖 **AI-native by design** — every interactive node derives a `<scope>.<slug>_<hash4>` action (Tap stays unprefixed; DoubleTap / LongPress / Submit / Set / Open / LoadMore / Swipe* / Confirm / Dismiss carry verb prefixes); `jian dev --mcp` exposes them over a real MCP stdio server.
- 🚀 **Typed cold-start pipeline** — three-stage `StartupDriver` (`DataPath` pre-window · `Visual` first-redraw · `Background` post-paint), per-platform budget tests, `jian perf startup` + `jian perf compare` regression gate, and AOT initial-layout pre-bake via `jian pack --aot`.
- 🔁 **Hot reload that keeps state** — `jian dev app.op` reparses on save without losing your counter / form / scroll position.
- 🛡️ **Capability-gated I/O** — declared up-front in `app.capabilities`; the gate refuses anything the document didn't ask for.
- 🤝 **Agent Shell Protocol (dev-only)** — `jian-asp` ships verbs (Tap / Type / Scroll / Swipe / Snapshot / Inspect ax_tree) for AI-driven UI testing; physically isolated from release builds via the `dev-asp` cargo feature.
- 🧪 **1014 tests, 0 failures** — `cargo test --workspace` is the source of truth; CI matrix covers macOS / Linux / Windows + the `textlayout` build path + a per-platform `startup-budget` regression gate (15% threshold, 1 ms noise floor).

## 🎬 Hello, Counter

Jian apps are JSON. This one renders, lays out, hit-tests, and is hot-reloadable
the moment you save:

```jsonc
{
  "formatVersion": "1.0",
  "id": "demo.counter",
  "app": { "name": "Counter", "version": "0.1.0", "id": "demo.counter" },
  "state":    { "count": { "type": "int", "default": 0 } },
  "children": [{
    "type": "frame", "id": "root", "width": 480, "height": 320,
    "fill": [{ "type": "solid", "color": "#f5f7fa" }],
    "children": [
      { "type": "text", "id": "label", "x": 40, "y": 60,
        "width": 400, "height": 60, "fontSize": 40, "fontWeight": 700,
        "content": "Count: 0",
        "bindings": { "content": "`Count: ${$app.count}`" } },

      { "type": "rectangle", "id": "btn", "x": 140, "y": 180,
        "width": 200, "height": 64, "cornerRadius": 12,
        "fill": [{ "type": "solid", "color": "#1e88e5" }],
        "events": { "onTap": [{ "set": { "$app.count": "$app.count + 1" } }] }
      }
    ]
  }]
}
```

```bash
cargo run -p jian -- dev counter.op             # live-reload window
cargo run -p jian --features mcp -- dev counter.op --mcp   # + AI over MCP
```

> No build step. No bundler. No transpiler. The CLI parses the file, builds
> the runtime, hands it to a real `winit` window, and starts watching the
> file for saves.

## 🤔 Why Jian?

|                   | Electron / Tauri + React | Flutter / Compose   | **Jian**                                |
| ----------------- | ------------------------ | ------------------- | --------------------------------------- |
| Distribution unit | bundled JS / wasm        | compiled binary     | **single `.op` JSON**                   |
| Reactive model    | VDOM diff                | rebuild + keys      | **fine-grained signals**                |
| Authoring loop    | bundle → reload          | rebuild → restart   | **save → reparse, state preserved**     |
| AI integration    | bolt-on per app          | bolt-on per app     | **derived MCP surface, every release**  |
| Renderer          | webview / Impeller       | Skia / Skia         | **Skia (raster today, GPU staged)**     |
| Runtime cost      | JS engine + DOM          | Dart VM + framework | **Rust binary, ~1 process**             |

Jian sits where "fast tools" meet "fast UIs": you ship the runtime once, then
distribute applications as **content** (`.op` files), not as bundles. The same
runtime is staged to embed into the OpenPencil canvas (via `napi-rs`) and
into the browser (via WASM) — the document is portable, the host is not.

## 🏗 Architecture

```text
                    ┌───────────────────────┐
                    │   .op   (Pen Schema)  │
                    └───────────┬───────────┘
                                │ parse
                                ▼
   ┌──────────────────────────────────────────────────────────┐
   │ jian-ops-schema  ── Pen Schema v1, JSON Schema + ops.ts  │
   └──────────────────────────────────────────────────────────┘
                                │
                                ▼
   ┌──────────────────────────────────────────────────────────┐
   │ jian-core                                                │
   │  • Runtime  • Signal/Effect  • StateGraph (6 scopes)     │
   │  • Tier-1 expressions  • Tier-2 Action DSL               │
   │  • Capability gate  • R-tree spatial  • Gesture arena    │
   │  • taffy flexbox  • MeasureBackend (estimate | Skia)     │
   └──────┬───────────────────────────────┬───────────────────┘
          │                               │
          ▼                               ▼
   ┌────────────────┐            ┌──────────────────────────┐
   │ jian-skia      │            │ jian-action-surface      │
   │  raster + GPU* │            │  derive · list · execute │
   │  paragraph⁺    │            │  state-gate · audit      │
   └────────┬───────┘            │  rate-limit · MCP (rmcp) │
            │                    └────────────┬─────────────┘
            ▼                                 ▼
   ┌──────────────────────────────────────────────────────────┐
   │ jian-host-desktop  (winit · softbuffer · muda · arboard) │
   └────────────────────────────┬─────────────────────────────┘
                                │
                                ▼
   ┌──────────────┐    ┌────────────────┐    ┌──────────────┐
   │  jian player │    │  jian dev      │    │ jian dev     │
   │  (window)    │    │  (+hot reload) │    │  --mcp (+AI) │
   └──────────────┘    └────────────────┘    └──────────────┘
```

\* GPU surface factories (Metal / D3D12 / GL / Vulkan) are scaffolded
behind feature flags — raster + softbuffer covers all three desktop OSes
today.
⁺ Real `Paragraph` shaping is opt-in via the `textlayout` feature; a 1 px
drift gate pins the estimate path against it on every CI run.

## 🚀 Quick Start

```bash
# 1. clone + build
git clone https://github.com/zseven-w/jian && cd jian
cargo build --release

# 2. scaffold
cargo run -p jian -- new my-app && cd my-app

# 3. validate a document
cargo run -p jian -- check app.op

# 4. run it in a real window
cargo run -p jian -- player app.op

# 5. author loop — hot-reload on every save, runtime state survives
cargo run -p jian -- dev app.op

# 6. AI in the loop — same as `dev`, plus stdio MCP for tools/list + tools/call
cargo run -p jian --features mcp -- dev app.op --mcp
```

Embedding the runtime directly:

```rust
use jian_core::Runtime;

let mut rt = Runtime::new();
rt.load_str(&std::fs::read_to_string("app.op")?)?;
rt.build_layout((800.0, 600.0))?;
rt.rebuild_spatial();

let hits = rt.spatial.hit(jian_core::geometry::point(50.0, 50.0));
```

Plug real font metrics in:

```rust
use jian_skia::SkiaMeasure;
rt.build_layout_with(SkiaMeasure::new(), (800.0, 600.0))?;
```

## 📦 Crates

| Crate                     | What it does |
| ------------------------- | ------------ |
| **`jian-ops-schema`**     | Canonical Pen Schema for `.op` / `.op.pack`. Round-trips legacy v0.x byte-for-byte; ships v1.0 additive Jian extensions. Generates `bindings/ops.schema.json` (Draft 2020-12) + `bindings/ops.ts` consumed by OpenPencil and other editors. Hosts the `pack::initial_layout` AOT format (Plan 19 D1) + `font_plan` codepoint scanner (D2). |
| **`jian-core`** | Runtime kernel. `Runtime` composition root, `Signal<T>` + `Effect`, `StateGraph` (`$app` / `$page` / `$self` / `$route` / `$storage` / `$vars`), Tier-1 expressions, Tier-2 Action DSL, capability gate, gesture arena, taffy flexbox, R-tree spatial index, `MeasureBackend` trait, **typed three-stage `StartupDriver`** + `HostAgnosticBootstrap` for the DataPath cold-start phases. |
| **`jian-skia`** | `RenderBackend` over `skia-safe`. Raster + per-corner radii + linear gradients + shadows + image cache + Lucide icons. Optional `textlayout` feature wires real `Paragraph` shaping pinned by a 1 px drift gate. |
| **`jian-host-desktop`** | `winit` 0.30 + `softbuffer` 0.4 host. Scale-factor-aware pointer / key translators, in-memory router + storage, `arboard` clipboard, `muda` native menus, binding-aware scene walker. **Visual-stage bootstrap** runs Splash / FirstFrame / Present / EventPumpReady inside the first `RedrawRequested` after `resumed()` (Plan 19 capstone B2.2 / B4). Per-platform deep-link trait-routing seams ship in `app_delegate` (macOS) / `win_deeplink` (Windows). |
| **`jian-asp`** *(dev-only)* | Agent Shell Protocol verb registry: Tap / Type / Scroll / Swipe / Snapshot / Inspect `ax_tree`. Physically isolated from release builds via the `dev-asp` cargo feature (CI assertion enforces no `jian-asp` in release `cargo tree`). Plan 18 Phase 3. |
| **`jian-action-surface`** | AI Action Surface (spec §3–§10). Derives stable `<scope>.<verb>_<slug>` actions, evaluates `RuntimeStateGate` against live bindings, dispatches synthesised pointer events, and serves `list_available_actions` / `execute_action` over an `rmcp` stdio bridge. Audit + rate limit + concurrency caps + swipe throttle baked in. |
| **`jian` (CLI)** | `check` · `pack [--aot] [--aot-viewport WxH]` · `unpack` · `new` · `player` · `dev` (+`--mcp`) · `perf startup [--runs N] [--format json]` · `perf compare BASELINE CURRENT [--threshold 0.15] [--noise-floor-ms 1.0] [--format markdown]`. |

## 🤖 AI Action Surface

Jian advertises a **derived, gated, audited** action surface to any AI client
that speaks MCP:

```bash
cargo run -p jian --features mcp -- dev app.op --mcp
```

Under the hood, every interactive node becomes a tool. Names follow
`<scope>.<slug>_<hash4>` — `<hash4>` is four hex chars derived from
`(node.id, BUILD_SALT)` and is dropped when the author sets
`semantics.aiName`:

| Node                                                       | Action name shape                          |
|------------------------------------------------------------|--------------------------------------------|
| `events.onTap` on `<button id="signup-cta">`               | `home.signup_cta_a3f7`                     |
| `bindings["bind:value"] = $state.email` on `<text-input>`  | `home.set_email_b012`                      |
| `events.onScroll` / `onReachEnd` on `<list>`               | `home.load_more_c4d5`                      |
| `events.onPanStart` + `onPanEnd` on a card                 | `home.swipe_{left,right,up,down}_e6f7` (4) |
| `events.onSubmit` on a `<form>`                            | `home.submit_<slug>_<hash4>`               |
| `route = { push: "/checkout" }` on a link                  | `home.open_checkout_<hash4>`               |
| `semantics.aiName = "checkout"` on the same link           | `home.checkout` (no `_<hash4>`)            |

- **Static availability** comes from the schema (`AvailabilityStatic::Available`
  / `ConfirmGated` / `StaticHidden`). `confirm:` / `fetch DELETE|POST` /
  `storage_clear` / `storage_wipe` flip a node to `ConfirmGated` until the
  author opts back in with `semantics.aiHidden: false`.
- **Live state-gate** (`RuntimeStateGate`) drops actions whose source node or
  any ancestor evaluates `bindings.visible == false` or
  `bindings.disabled == true` against the live `StateGraph` — so the AI
  never sees an action it would then bounce off `state_gated` on execute.
- **Authorship overrides** (`aiAliases`) survive both `list` and `execute`;
  alias hits are audited with `alias_used: true` and stay transparent to
  the client.
- **Phase 1 dispatch coverage**: Tap / Confirm / Dismiss / SetValue / OpenRoute
  are wired against the live `Runtime`. DoubleTap / LongPress / Submit /
  Swipe* / LoadMore are listed in `list_available_actions` but currently
  return `ExecutionFailed(handler_error)` until their host-driver paths
  land.
- **Pointer dispatch** synthesises a real `PointerDown` + `PointerUp` at the
  layout centre of the source node — the same code path a human cursor takes.
- **Audit log + rate limit (10 calls/sec/session) + same-action concurrency
  cap + swipe 400 ms same-direction throttle** are first-class.
- The `RuntimeDispatcher` wraps `&mut Runtime`, while a `SinkDispatcher` keeps
  unit tests free of `winit`.

> Build-time stable names: the `_<hash4>` suffix comes from `BUILD_SALT`
> baked in by `crates/jian-core/build.rs` (priority: `JIAN_BUILD_SALT`
> env override → `<git-short-16>-<cargo-semver>` (e.g.
> `1a2b3c4d5e6f7890-0.0.1`) → `<cargo-semver>` alone). `jian dev --mcp`
> prints the resolved source string on attach so prompt caches can key
> on it.

See [`crates/jian-action-surface/README.md`](crates/jian-action-surface/README.md)
and [`openpencil-docs/superpowers/notes/2026-04-24-ai-action-surface-client-guide.md`](../openpencil-docs/superpowers/notes/2026-04-24-ai-action-surface-client-guide.md).

## 🛠 Development

```bash
cargo test --workspace                  # 1014 tests, 0 failures
cargo test --workspace --all-features   # adds the mcp + textlayout + dev-asp paths
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo run -p jian-ops-schema --bin export_schema
cargo run -p jian-ops-schema --features export-ts --bin export_ts

# Cold-start instrumentation
cargo run --release -p jian -- perf startup app.op --runs 20
cargo run --release -p jian -- perf startup app.op --runs 20 --format json > current.json
cargo run --release -p jian -- perf compare baseline.json current.json \
    --threshold 0.15 --noise-floor-ms 1.0 --label macos-aarch64

# AOT pre-bake (writes aot/initial_layout.bin into the .op.pack)
cargo run --release -p jian -- pack --aot --aot-viewport 800x600 app.op out.op.pack
```

CI runs on macOS / Linux / Windows; the `textlayout` job needs Python 3.11
because Skia's text shaping build pulls it in. The `startup-budget` workflow
collects per-platform `jian perf startup` JSON on every push and gates PRs
against `main`'s most recent green baseline (15% regression threshold,
1 ms noise floor, single rolling PR comment with the diff table).

## 🗺 Roadmap

**Shipped in `v0.0.1`:**

- ✅ Plan 1 ops-schema · Plan 2 core · Plan 3 Tier-1 expressions · Plan 4 Tier-2 Action DSL
- ✅ Plan 5 gesture arena (Tap / LongPress / Swipe / Scroll + multi-pointer Pinch & Rotate)
- ✅ Plan 6 capability gate (dev-asp gated)
- ✅ Plan 7 `jian-skia` (raster + gradients + image cache + `ParagraphBuilder` feature)
- ✅ Plan 8 desktop host (winit / softbuffer / muda menu spec)
- ✅ Plan 9 CLI (eight subcommands incl. `dev` hot-reload + `perf startup` / `perf compare`)
- ✅ Plan 22 `jian-action-surface` Phase 1 + MCP stdio server (`rmcp`) + `jian dev --mcp`
- ✅ Plan 18 Agent Shell Protocol Phase 3 verbs — Tap / Type / Scroll / Swipe / Snapshot / Inspect `ax_tree` (`jian-asp`, dev-only via the `dev-asp` cargo feature, CI-asserted off the release path)
- ✅ Plan 19 cold-start capstone — typed three-stage `StartupDriver` (`DataPath` / `Visual` / `Background`), `HostAgnosticBootstrap` for DataPath, visual-stage runner for the first `RedrawRequested` after `resumed()`, `jian player` two-stage launch with unified-launch-epoch report timeline
- ✅ Plan 19 D1 `.op.pack` AOT initial-layout writer + reader (`OPL1` little-endian SoA format, `jian pack --aot`); D2 font subsetter wiring (`FontPlan::scan_subtrees` populates `BootstrapHandles::take_core_font_plan`); D3 per-platform startup budget tests; D4 `jian perf compare` CI diff bot + 15% threshold gate + single rolling PR comment
- ✅ Plan 8 §T7 native menu bar · §T8 deep-link trait-routing seam (`app_delegate.rs` macOS / `win_deeplink.rs` Windows) · §T9 updater trait + `selfupdate` feature · §T10 packaging configs (`cargo bundle` macOS .app · `cargo wix` Windows MSI · `cargo deb` + AppImage Linux · `.icns` generator · Sparkle appcast template · AppImageUpdate metadata)
- ✅ §3.3 CJK transliteration · §3.4 collision detection · §6.3 swipe throttle · §8.1 AuditLog
- ✅ §3.1 `BUILD_SALT` build-time injection (`crates/jian-core/build.rs` — env override → git+semver → semver fallback; mac `.git` worktree resolved; FNV-1a double-hash → 16 bytes)
- ✅ Bubble-style event dispatch · binding-aware scene walker
- ✅ macOS Dock icon at runtime via `NSApp.setApplicationIconImage:` (`set_macos_dock_icon_from_png`) so unbundled `jian player` / `jian dev` (or any host that calls `DesktopHost::with_icon`) shows the schema's `app.icon` instead of the default exec icon
- ✅ Action Surface protocol docs — `crates/jian-action-surface/README.md` (embedding + threat-model anchors) + `openpencil-docs/superpowers/notes/2026-04-24-ai-action-surface-client-guide.md` (Claude Desktop / raw-stdio Python clients, error-handling policy, build-salt awareness)

**Up next (each warrants its own session):**

- ⏳ Plan 19 follow-ups — runtime preload of `aot/initial_layout.bin` (skip `ComputeFirstLayout` via `LayoutEngine::preload_initial`); `aot/expressions.bin` precompiled bytecode; `aot/default_state.bin` serialised initial state
- ⏳ Plan 8 §T8 platform receivers — macOS `NSApplicationDelegate` Apple-Event subclass; Windows `WM_COPYDATA` hidden-window `WindowProc`; single-instance forwarding so `jian://` URL-scheme registration can flip back on across all three platforms
- ⏳ Plan 8 / 11 / 12 — GPU surface factories (Metal · D3D12 · OpenGL / WebGL · Vulkan)
- ⏳ Plan 11 — OpenPencil canvas swap (replace `pen-renderer` via `napi-rs`)
- ⏳ Plan 13 — Electron → Tauri migration
- ⏳ Plan 14 — `pen-mcp` Rust port (byte-level parity gate)
- ⏳ Plan 15 — `pen-ai-skills` Rust + 3-backend split (MCP / ActionSurface / ASP)
- ⏳ Plan 16 — `pen-codegen` Rust (9 codegen targets)
- ⏳ Plan 17 — `pen-figma` Rust + Stage E/F/G
- 🔒 Plan 24 — `.op.pack` protect Phase 2 (calendar-locked behind ≥ 3 months Phase 1 production)

## 📚 Spec

The full design lives next to the code:

- Specs: [`openpencil-docs/superpowers/specs/`](../openpencil-docs/superpowers/specs/)
- Plans: [`openpencil-docs/superpowers/plans/`](../openpencil-docs/superpowers/plans/) (`2026-04-17-jian-plan-*` and `2026-04-2x-jian-*`)

## 📜 License

MIT © Jian contributors

<div align="center">
<sub>built with 🦀, <code>winit</code>, <code>skia-safe</code>, <code>taffy</code>, and one stubborn <code>.op</code> file.</sub>
</div>
