//! Canonical render-ready scene for jian-based design tools.
//!
//! `layout_scene` is the layout-resolved, variable-resolved render tree built
//! from a document; it is the form the `RenderBackend` painters consume. Moved
//! out of OpenPencil's `op-editor-ui` so it can be shared cross-product.
pub mod layout_scene;
// path_geometry + layout_scene_hit are added in later tasks.
pub use layout_scene::{SceneTextAlign, SceneTextVerticalAlign};
