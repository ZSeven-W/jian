//! The five state scopes defined in spec 02 §5.2.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    App,
    Page,
    SelfNode,
    Route,
    Storage,
    /// Design variables — shared theme scope.
    Vars,
}

impl Scope {
    pub fn as_prefix(self) -> &'static str {
        match self {
            Scope::App => "$app",
            Scope::Page => "$page",
            Scope::SelfNode => "$self",
            Scope::Route => "$route",
            Scope::Storage => "$storage",
            Scope::Vars => "$vars",
        }
    }

    pub fn parse_prefix(s: &str) -> Option<Self> {
        Some(match s {
            "$app" => Scope::App,
            "$page" => Scope::Page,
            "$self" => Scope::SelfNode,
            "$route" => Scope::Route,
            "$storage" => Scope::Storage,
            "$vars" => Scope::Vars,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_back() {
        for s in [
            Scope::App,
            Scope::Page,
            Scope::SelfNode,
            Scope::Route,
            Scope::Storage,
            Scope::Vars,
        ] {
            let p = s.as_prefix();
            assert_eq!(Scope::parse_prefix(p), Some(s));
        }
    }

    #[test]
    fn unknown_prefix_fails() {
        assert_eq!(Scope::parse_prefix("$item"), None);
    }
}
