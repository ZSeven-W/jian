//! Tier 2 Action DSL interpreter.
//!
//! An Action is a single-key JSON object: `{ "<name>": <body> }`. An
//! ActionList is an array of Actions executed serially by default. This
//! module parses the JSON, dispatches to per-action implementations, and
//! supports async IO and nested control flow.

pub mod action_trait;
pub mod cancel;
pub mod capability;
pub mod context;
pub mod error;
pub mod executor;
pub mod registry;
pub mod services;
pub mod value;

pub use action_trait::{ActionChain, ActionFactory, ActionImpl, BoxedAction};
pub use capability::{Capability, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate};
pub use context::ActionContext;
pub use executor::{execute_list, execute_list_shared, ExecOutcome};
pub use registry::ActionRegistry;

use std::cell::RefCell;
use std::rc::Rc;

pub type SharedRegistry = Rc<RefCell<ActionRegistry>>;

/// Build the default registry — actions registered in later tasks.
pub fn default_registry() -> SharedRegistry {
    Rc::new(RefCell::new(ActionRegistry::new()))
}

pub use cancel::CancellationToken;
pub use error::{ActionError, ActionResult};
