//! Stub — full definition in Task 13.
use serde::{Deserialize, Serialize};

pub type ActionList = Vec<Action>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Action(pub serde_json::Value);
