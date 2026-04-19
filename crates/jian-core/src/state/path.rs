//! Parser for state paths like `$state.count` / `$app.user.name` / `$route.params.id`.
//!
//! Grammar (informal):
//!   Path   ::= ScopeRef ('.' Segment)*
//!   Segment ::= Identifier | '[' IndexOrKey ']'
//!
//! Returns a `StatePath` with scope + vector of segments.

use super::scope::Scope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePath {
    pub scope: Scope,
    pub segments: Vec<Segment>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PathError {
    #[error("path does not start with `$`: `{0}`")]
    MissingDollar(String),
    #[error("unknown scope prefix: `{0}`")]
    UnknownScope(String),
    #[error("unexpected end of path")]
    UnexpectedEnd,
    #[error("unmatched bracket in `{0}`")]
    UnmatchedBracket(String),
    #[error("empty segment")]
    EmptySegment,
}

impl StatePath {
    pub fn parse(src: &str) -> Result<Self, PathError> {
        if !src.starts_with('$') {
            return Err(PathError::MissingDollar(src.to_owned()));
        }

        // Find end of scope prefix: first '.' or '[' or end.
        let prefix_end = src
            .find(|c: char| c == '.' || c == '[')
            .unwrap_or(src.len());
        let prefix = &src[..prefix_end];
        let scope = Scope::parse_prefix(prefix)
            .ok_or_else(|| PathError::UnknownScope(prefix.to_owned()))?;

        let mut segments = Vec::new();
        let mut rest = &src[prefix_end..];
        // Skip an optional leading '.'.
        if let Some(stripped) = rest.strip_prefix('.') {
            rest = stripped;
        }
        while !rest.is_empty() {
            // Case: bracketed segment `[key]` or `[0]`
            if let Some(stripped) = rest.strip_prefix('[') {
                let end = stripped
                    .find(']')
                    .ok_or_else(|| PathError::UnmatchedBracket(src.to_owned()))?;
                let inside = &stripped[..end];
                if let Ok(idx) = inside.parse::<usize>() {
                    segments.push(Segment::Index(idx));
                } else {
                    let key = inside.trim_matches(|c| c == '"' || c == '\'');
                    segments.push(Segment::Key(key.to_owned()));
                }
                rest = &stripped[end + 1..];
                if let Some(stripped2) = rest.strip_prefix('.') {
                    rest = stripped2;
                }
                continue;
            }
            // Case: dotted segment up to next '.' or '['
            let next = rest.find(|c| c == '.' || c == '[').unwrap_or(rest.len());
            if next == 0 {
                return Err(PathError::EmptySegment);
            }
            segments.push(Segment::Key(rest[..next].to_owned()));
            rest = &rest[next..];
            if let Some(stripped) = rest.strip_prefix('.') {
                rest = stripped;
            }
        }

        Ok(StatePath { scope, segments })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_only() {
        let p = StatePath::parse("$app").unwrap();
        assert_eq!(p.scope, Scope::App);
        assert!(p.segments.is_empty());
    }

    #[test]
    fn simple_dotted() {
        let p = StatePath::parse("$app.user.name").unwrap();
        assert_eq!(p.scope, Scope::App);
        assert_eq!(
            p.segments,
            vec![Segment::Key("user".into()), Segment::Key("name".into())]
        );
    }

    #[test]
    fn indexed_access() {
        let p = StatePath::parse("$app.items[0].title").unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("items".into()),
                Segment::Index(0),
                Segment::Key("title".into()),
            ]
        );
    }

    #[test]
    fn bracket_quoted_key() {
        let p = StatePath::parse(r#"$app.map["weird-key"]"#).unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("map".into()),
                Segment::Key("weird-key".into()),
            ]
        );
    }

    #[test]
    fn state_alias_uses_app() {
        // `$state` is NOT a standalone scope; expect UnknownScope.
        let err = StatePath::parse("$state.count").unwrap_err();
        assert!(matches!(err, PathError::UnknownScope(_)));
    }

    #[test]
    fn missing_dollar() {
        assert!(matches!(
            StatePath::parse("app.x"),
            Err(PathError::MissingDollar(_))
        ));
    }

    #[test]
    fn unknown_scope() {
        assert!(matches!(
            StatePath::parse("$whatever.x"),
            Err(PathError::UnknownScope(_))
        ));
    }
}
