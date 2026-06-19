//! Canonical render-ready scene for jian-based design tools.
//!
//! `layout_scene` is the layout-resolved, variable-resolved render tree built
//! from a document; it is the form the `RenderBackend` painters consume. Moved
//! out of OpenPencil's `op-editor-ui` so it can be shared cross-product.
pub mod layout_scene;
pub mod path_geometry;
// layout_scene_hit is added in the next task.
pub use layout_scene::{SceneTextAlign, SceneTextVerticalAlign};
