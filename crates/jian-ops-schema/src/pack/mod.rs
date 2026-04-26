//! `.op.pack` archive format (Plan 19 Task 6 foundation).
//!
//! A `.op.pack` is a deflate-compressed zip carrying a single `.op`
//! document plus optional precompiled assets — fonts, AOT bytecode,
//! pre-computed layout, serialized initial state, image binaries, logic
//! WASM modules. The on-disk layout is:
//!
//! ```text
//! app.op.pack/
//! ├── manifest.json              (AotManifest — schema in this module)
//! ├── app.op                     canonical JSON, always included
//! ├── aot/
//! │   ├── expressions.bin        precompiled bytecode (per-expression)
//! │   ├── initial_layout.bin     map<node_id, (x,y,w,h)> for default viewport
//! │   └── default_state.bin      serialized StateGraph initial values
//! ├── fonts/
//! │   ├── <Family>-sub.<ext>     critical-frame subset
//! │   └── <Family>.<ext>         full font, lazy-loaded
//! ├── images/
//! │   └── ...
//! └── logic/
//!     └── <module-id>.wasm
//! ```
//!
//! Plan 19 Task 6 spec (lines 326-426 of the cold-start plan) calls for
//! both writer and reader. **This module ships the format types only.**
//! The actual byte-level serializers for `aot/expressions.bin`,
//! `aot/initial_layout.bin`, and `aot/default_state.bin` need
//! `jian_core::expression::Chunk` and the layout / state types to derive
//! `Serialize`, which is a touchier refactor that lands in a follow-up
//! commit alongside the bincode-based serializer. The font subset entries
//! depend on the subsetter wiring (Plan 19 Task 4 follow-up).
//!
//! Until those follow-ups land, the existing `jian-cli` `pack` command
//! continues to emit a JSON-only `.op.pack` (its own `manifest.json`
//! shape in `crates/jian-cli/src/commands/pack.rs`). Migrating that
//! command to the typed [`AotManifest`] in this module is mechanical and
//! tracked alongside the AOT serializer follow-up — the format constants
//! below are deliberately wire-compatible with the current MVP schema.

pub mod manifest;

pub use manifest::*;
