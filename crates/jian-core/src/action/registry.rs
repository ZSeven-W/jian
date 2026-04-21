//! The ActionRegistry — name → factory. Factories are invoked at **parse
//! time** from a `serde_json::Value` (the action body), producing a fully
//! prepared `BoxedAction`.

use super::action_trait::{ActionChain, ActionFactory, BoxedAction};
use super::error::ActionError;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct ActionRegistry {
    factories: BTreeMap<String, ActionFactory>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            factories: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, factory: ActionFactory) {
        self.factories.insert(name.into(), factory);
    }

    pub fn parse_single(&self, obj: &Value) -> Result<BoxedAction, ActionError> {
        let map = obj.as_object().ok_or_else(|| {
            ActionError::Custom(format!("Action must be a JSON object, got `{}`", obj))
        })?;
        if map.len() != 1 {
            return Err(ActionError::Custom(format!(
                "Action must have exactly one key, got {}: {:?}",
                map.len(),
                map.keys().collect::<Vec<_>>()
            )));
        }
        let (name, body) = map.iter().next().unwrap();
        let factory = self
            .factories
            .get(name)
            .ok_or_else(|| ActionError::UnknownAction(name.clone()))?;
        factory(body)
    }

    pub fn parse_list(&self, arr: &Value) -> Result<ActionChain, ActionError> {
        let list = arr.as_array().ok_or_else(|| {
            ActionError::Custom(format!("ActionList must be a JSON array, got `{}`", arr))
        })?;
        let mut out = Vec::with_capacity(list.len());
        for item in list {
            out.push(self.parse_single(item)?);
        }
        Ok(ActionChain(out))
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::action_trait::ActionImpl;
    use super::super::context::ActionContext;
    use super::super::error::ActionResult;
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct Noop {
        name: &'static str,
    }

    #[async_trait(?Send)]
    impl ActionImpl for Noop {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn execute(&self, _: &ActionContext) -> ActionResult {
            Ok(())
        }
    }

    #[test]
    fn parse_single_ok() {
        let mut reg = ActionRegistry::new();
        reg.register(
            "noop",
            Box::new(|_body| Ok(Box::new(Noop { name: "noop" }) as BoxedAction)),
        );
        let act = reg.parse_single(&json!({"noop": {}})).unwrap();
        assert_eq!(act.name(), "noop");
    }

    #[test]
    fn unknown_action_errors() {
        let reg = ActionRegistry::new();
        assert!(matches!(
            reg.parse_single(&json!({"mystery": 42})),
            Err(ActionError::UnknownAction(_))
        ));
    }

    #[test]
    fn multi_key_object_errors() {
        let reg = ActionRegistry::new();
        assert!(reg.parse_single(&json!({"a": 1, "b": 2})).is_err());
    }

    #[test]
    fn parse_list_empty() {
        let reg = ActionRegistry::new();
        let chain = reg.parse_list(&json!([])).unwrap();
        assert!(chain.0.is_empty());
    }

    #[test]
    fn parse_list_multiple() {
        let mut reg = ActionRegistry::new();
        reg.register(
            "a",
            Box::new(|_| Ok(Box::new(Noop { name: "a" }) as BoxedAction)),
        );
        reg.register(
            "b",
            Box::new(|_| Ok(Box::new(Noop { name: "b" }) as BoxedAction)),
        );
        let chain = reg.parse_list(&json!([{"a": {}}, {"b": {}}])).unwrap();
        assert_eq!(chain.0.len(), 2);
        assert_eq!(chain.0[0].name(), "a");
        assert_eq!(chain.0[1].name(), "b");
    }
}
