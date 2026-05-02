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
//! Plan 19 Task 6 spec (lines 326-426 of the cold-start plan) calls
//! for writer + reader for three AOT assets. Status:
//!
//! - `aot/initial_layout.bin` — **shipped** (Plan 19 D1). Byte-level
//!   serializer + decoder live in [`initial_layout`]; `jian pack
//!   --aot` wires it through. The runtime preload path lives in
//!   `jian_core::layout::LayoutEngine::preload_initial` and is gated
//!   by [`crate::pack::default_state::DefaultStateSnapshot::is_empty`]'s
//!   sibling on the bootstrap side.
//! - `aot/default_state.bin` — **shipped** (Plan 19 D1 follow-up).
//!   Six-scope JSON-in-binary-frame serializer + decoder live in
//!   [`default_state`]. The pack writer captures values from a
//!   freshly seeded `Runtime` and the runtime side restores them
//!   ahead of `SeedStateGraph` so the schema-default scan can be
//!   skipped on cold start.
//! - `aot/expressions.bin` — **deferred**. Needs `jian_core::expression::Chunk`
//!   to derive `Serialize`, a touchier refactor.
//!
//! The font subset entries depend on the subsetter wiring (Plan 19
//! D2 — runtime side shipped, AOT side waits on D1's expression
//! follow-up because both go in the same pack writer pass).
//!
//! Migrating the existing `jian-cli` `pack` command from its untyped
//! `serde_json::Value` manifest to the typed [`AotManifest`] in this
//! module is mechanical and tracked separately; the format constants
//! below stay wire-compatible with the current MVP schema so the
//! migration lands without touching `manifest.json` consumers.

pub mod default_state;
pub mod initial_layout;
pub mod manifest;

pub use default_state::{
    DefaultStateError, DefaultStateSnapshot, DEFAULT_STATE_MAGIC, DEFAULT_STATE_VERSION,
};
pub use initial_layout::{
    InitialLayoutError, InitialLayoutSnapshot, PackedRect, INITIAL_LAYOUT_MAGIC,
    INITIAL_LAYOUT_VERSION,
};
pub use manifest::*;
