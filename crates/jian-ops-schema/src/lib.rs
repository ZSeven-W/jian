//! `jian-ops-schema` — canonical types + JSON Schema for Jian `.op` files.

pub mod app;
pub mod document;
pub mod error;
pub mod events;
pub mod expression;
pub mod gestures;
pub mod navigation;
pub mod node;
pub mod page;
pub mod routes;
pub mod sizing;
pub mod state;
pub mod style;
pub mod variable;

pub use error::{OpsResult, OpsSchemaError};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
