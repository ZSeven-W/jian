//! Scene walker — re-export shim.
//!
//! The `RuntimeDocument` + `LayoutEngine` → `Vec<DrawOp>` walker now
//! lives in `jian-core` (`jian_core::render::scene`) so wasm-clean
//! consumers — e.g. OpenPencil's future `op-canvas` crate — can reuse
//! it without depending on this desktop host crate. This module keeps
//! the historical `crate::scene::collect_draws[_with_state]` path
//! alive so existing host callers compile unchanged.

pub use jian_core::render::{collect_draws, collect_draws_with_state};
