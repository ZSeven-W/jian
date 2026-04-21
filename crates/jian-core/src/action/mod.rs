//! Tier 2 Action DSL interpreter.
//!
//! An Action is a single-key JSON object: `{ "<name>": <body> }`. An
//! ActionList is an array of Actions executed serially by default. This
//! module parses the JSON, dispatches to per-action implementations, and
//! supports async IO and nested control flow.

pub mod cancel;
pub mod capability;
pub mod context;
pub mod error;
pub mod services;

pub use context::ActionContext;

pub use cancel::CancellationToken;
pub use error::{ActionError, ActionResult};
