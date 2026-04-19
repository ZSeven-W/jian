//! Runtime document — in-memory representation of a loaded `.op` file.
//!
//! Holds:
//! - the original parsed schema document
//! - a flat tree index (`NodeTree`)
//! - the active page (when present)

pub mod loader;
pub mod tree;

pub use tree::{NodeData, NodeKey, NodeTree};

use jian_ops_schema::document::PenDocument;

pub struct RuntimeDocument {
    pub schema: PenDocument,
    pub tree: NodeTree,
    pub active_page: Option<String>,
}

impl RuntimeDocument {
    pub fn node_count(&self) -> usize {
        self.tree.len()
    }
}
