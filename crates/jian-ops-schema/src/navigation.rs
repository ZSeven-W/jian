use serde::{Deserialize, Serialize};

/// Declarative per-node navigation: clicking the node pushes/replaces/pops a route.
/// Equivalent to `events.on_tap = [{"push": "..."}]` but more editor-discoverable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NavigationRoute {
    #[serde(rename = "push")]
    Push(String),
    #[serde(rename = "replace")]
    Replace(String),
    #[serde(rename = "pop")]
    Pop(()),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_push() {
        let j = r#"{"push":"/detail/42"}"#;
        let n: NavigationRoute = serde_json::from_str(j).unwrap();
        assert!(matches!(n, NavigationRoute::Push(ref s) if s == "/detail/42"));
    }

    #[test]
    fn navigate_replace() {
        let j = r#"{"replace":"/login"}"#;
        let n: NavigationRoute = serde_json::from_str(j).unwrap();
        assert!(matches!(n, NavigationRoute::Replace(_)));
    }
}
