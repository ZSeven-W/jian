//! `jian-ops-schema` — canonical types + JSON Schema for Jian `.op` files.

pub mod app;
pub mod compat;
pub mod document;
pub mod error;
pub mod events;
pub mod expression;
pub mod font_plan;
pub mod gestures;
pub mod lifecycle;
pub mod logic_module;
pub mod navigation;
pub mod node;
pub mod pack;
pub mod page;
pub mod routes;
pub mod semantics;
pub mod sizing;
pub mod state;
pub mod style;
pub mod variable;
pub mod version;

pub use compat::load_str;
pub use document::PenDocument;
pub use error::{LoadResult, LoadWarning, OpsResult, OpsSchemaError};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
