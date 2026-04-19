//! Format version negotiation per C1 (backward compat).

/// Current format version this crate understands.
pub const FORMAT_VERSION_CURRENT: &str = "1.0";

/// Minimum format version this crate can still parse.
pub const FORMAT_VERSION_MIN: &str = "0.0";

/// Parse a format version string into a (major, minor) tuple.
/// Returns `(0, 0)` for the absent/legacy case.
pub fn parse(v: Option<&str>) -> (u32, u32) {
    match v {
        None => (0, 0),
        Some(s) => {
            let mut parts = s.split('.');
            let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            (major, minor)
        }
    }
}

pub fn supports(v: Option<&str>) -> bool {
    let (major, _) = parse(v);
    // We support major versions 0 and 1.
    major <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absent() {
        assert_eq!(parse(None), (0, 0));
    }
    #[test]
    fn parse_numeric() {
        assert_eq!(parse(Some("1.0")), (1, 0));
    }
    #[test]
    fn parse_with_patch() {
        assert_eq!(parse(Some("0.8.0")), (0, 8));
    }
    #[test]
    fn supports_v0() {
        assert!(supports(None));
        assert!(supports(Some("0.8.0")));
    }
    #[test]
    fn supports_v1() {
        assert!(supports(Some("1.0")));
    }
    #[test]
    fn rejects_v2() {
        assert!(!supports(Some("2.0")));
    }
}
