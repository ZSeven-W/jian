//! Tier 2 Action DSL interpreter.
//!
//! An Action is a single-key JSON object: `{ "<name>": <body> }`. An
//! ActionList is an array of Actions executed serially by default. This
//! module parses the JSON, dispatches to per-action implementations, and
//! supports async IO and nested control flow.

pub mod action_trait;
pub mod actions;
pub mod animation_registry;
pub mod cancel;
pub mod capability;
pub mod catalog;
pub mod context;
pub mod error;
pub mod executor;
pub mod policy;
pub mod registry;
pub mod services;
pub mod task_queue;
pub mod value;

pub use action_trait::{ActionChain, ActionFactory, ActionImpl, BoxedAction};
pub use animation_registry::{
    animatable_property_registry, AnimatableProperty, AnimatablePropertyRegistry, AnimationApply,
    AnimationInterpolate, AnimationRegistryError, AnimationValueType, SHADER_UNIFORM_PREFIX,
};
pub use capability::{Capability, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate};
pub use catalog::{preview_action_descriptors, ActionDescriptor};
pub use context::ActionContext;
pub use executor::{execute_list_async, ExecOutcome};
pub use policy::{ActionPolicy, AllowListPolicy, PreviewActionPolicy};
pub use registry::ActionRegistry;
pub use task_queue::{CompletedTask, TaskClock, TaskQueue};

use std::cell::RefCell;
use std::rc::Rc;

pub type SharedRegistry = Rc<RefCell<ActionRegistry>>;

/// Build the default registry with every MVP action registered.
pub fn default_registry() -> SharedRegistry {
    let reg = Rc::new(RefCell::new(ActionRegistry::new()));
    actions::register_all(&reg);
    reg
}

pub use cancel::CancellationToken;
pub use error::{ActionError, ActionResult};
