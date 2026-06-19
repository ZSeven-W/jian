//! Scene primitives shared by the render backends.
//!
//! The render-ready `SceneNode` / `SceneGraph` skeleton was removed
//! 2026-06-19: it was unused (jian paints via `DrawOp` / `Painter`, never
//! from this graph), and OpenPencil's mature render scene now lives in the
//! `jian-scene` crate. Only the load-bearing `Color` type remains here.

pub mod properties;

pub use properties::Color;
