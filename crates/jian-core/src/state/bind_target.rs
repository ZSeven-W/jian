//! The one parser for a widget's `bindings["bind:value"]` target.
//!
//! `$state` and `$app` name the same app scope (`Scope::parse_prefix`),
//! and the generation skills teach `bind:value: "$app.email"`. Every
//! consumer — the write-back after input, the read-back seed on mount,
//! and the action-surface derivation — used to strip only `$state.`,
//! so an `$app.` binding displayed typed text but never reached the
//! state graph. They all go through [`app_bind_key`] now.

/// Why a `bind:value` source is not a supported two-way target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindTargetError {
    /// Not `$state.<key>` / `$app.<key>` (e.g. `$route.x`, `$page.x`).
    UnsupportedScope,
    /// Nested path or expression (`$app.a.b`, `$app.items[0]`, `$app.x + 1`).
    NotSingleKey,
    /// `$app.` with nothing after it.
    EmptyKey,
}

impl std::fmt::Display for BindTargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedScope => "bind:value must target $app.<key> or $state.<key>",
            Self::NotSingleKey => {
                "bind:value must be a single app-state key, not a path or expression"
            }
            Self::EmptyKey => "bind:value names no state key",
        })
    }
}

/// Parse a `bind:value` source into its app-scope key.
pub fn parse_app_bind_key(raw: &str) -> Result<&str, BindTargetError> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("$state.")
        .or_else(|| trimmed.strip_prefix("$app."))
        .ok_or(BindTargetError::UnsupportedScope)?;
    if rest.is_empty() {
        return Err(BindTargetError::EmptyKey);
    }
    if rest.contains(['.', '[', ']']) || !rest.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(BindTargetError::NotSingleKey);
    }
    Ok(rest)
}

/// [`parse_app_bind_key`] as an `Option`, for call sites that simply
/// skip an unsupported target (diagnostics come from the validator).
pub fn app_bind_key(raw: &str) -> Option<&str> {
    parse_app_bind_key(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_and_state_prefixes_are_equivalent() {
        assert_eq!(app_bind_key("$app.email"), Some("email"));
        assert_eq!(app_bind_key(" $state.email "), Some("email"));
    }

    #[test]
    fn unsupported_targets_are_classified() {
        assert_eq!(
            parse_app_bind_key("$route.q"),
            Err(BindTargetError::UnsupportedScope)
        );
        assert_eq!(
            parse_app_bind_key("$app.user.name"),
            Err(BindTargetError::NotSingleKey)
        );
        assert_eq!(
            parse_app_bind_key("$state.x + 1"),
            Err(BindTargetError::NotSingleKey)
        );
        assert_eq!(parse_app_bind_key("$app."), Err(BindTargetError::EmptyKey));
    }
}
