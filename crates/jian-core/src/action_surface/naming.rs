//! Slug normalisation + 4-hex disambiguator.
//!
//! Slug priority (spec §3.3):
//! 1. `semantics.aiName` — author-stable override (slug-form)
//! 2. `semantics.label`  — A11y label (frequently a verb phrase)
//! 3. `node.text` / `node.content` — text-node payload (any string form)
//! 4. `node.id`           — fallback
//!
//! `derive_actions` operates on `serde_json::Value` views of each
//! node so it can inspect `semantics` / `events` / `bindings` / `route`
//! without enumerating all 11 PenNode variants — same pattern as
//! `jian-host-desktop::scene::collect_draws`.
//!
//! `short_hash` returns 4 lowercase hex chars seeded by both
//! `build_salt` (cross-build stable) and the node id. Spec §3.4
//! doesn't require cryptographic strength — FNV-1a 64-bit ⇒ 16 bits
//! disambiguator ⇒ ~99% collision-free for typical app sizes.

use serde_json::Value;

/// Compute the slug source for a node JSON view, in spec §3.3 priority
/// order. Returns the *raw* string; callers run `normalize_slug`.
pub fn slug_source(node: &Value) -> String {
    let semantics = node.get("semantics").and_then(|v| v.as_object());

    if let Some(sem) = semantics {
        if let Some(s) = sem
            .get("aiName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return s.to_owned();
        }
        if let Some(s) = sem
            .get("label")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return s.to_owned();
        }
    }

    // Text nodes carry their author-visible content under `content`.
    // It can be a plain string or a styled-segment object — prefer the
    // string form, otherwise accept the first segment's `text`.
    if let Some(text) = node.get("content").and_then(|c| extract_text(c)) {
        if !text.is_empty() {
            return text;
        }
    }

    node.get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

fn extract_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_owned());
    }
    if let Some(arr) = content.as_array() {
        for seg in arr {
            if let Some(t) = seg.get("text").and_then(|v| v.as_str()) {
                if !t.is_empty() {
                    return Some(t.to_owned());
                }
            }
        }
    }
    None
}

/// Apply spec §3.3 normalisation: lowercase, ASCII-alphanumeric kept,
/// everything else collapses to a single `_`, then trim leading and
/// trailing underscores. Empty result is returned as-is — callers
/// fall back to the node id externally.
pub fn normalize_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;

    for ch in raw.chars() {
        for c in ch.to_lowercase() {
            if c.is_ascii_alphanumeric() {
                out.push(c);
                last_underscore = false;
            } else if !last_underscore {
                out.push('_');
                last_underscore = true;
            }
        }
    }
    out.trim_matches('_').to_owned()
}

/// Compute the slug for a node, resolving the priority chain and
/// applying normalisation. If everything is empty, falls back to the
/// node id, then to `"node"`.
pub fn compute_slug(node: &Value) -> String {
    let raw = slug_source(node);
    let normalised = normalize_slug(&raw);
    if !normalised.is_empty() {
        return normalised;
    }
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let by_id = normalize_slug(id);
    if !by_id.is_empty() {
        by_id
    } else {
        "node".to_owned()
    }
}

/// FNV-1a 64-bit; first 16 bits → 4 lowercase hex chars.
pub fn short_hash(node_id: &str, build_salt: &[u8; 16]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in build_salt {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    for b in node_id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:04x}", h & 0xffff)
}

/// `true` when the author set `semantics.aiName` — caller drops the
/// `_<hash4>` suffix (spec §3.4).
pub fn has_ai_name(node: &Value) -> bool {
    node.get("semantics")
        .and_then(|s| s.get("aiName"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_basics() {
        assert_eq!(normalize_slug("Sign In"), "sign_in");
        assert_eq!(normalize_slug("SUBMIT  FORM"), "submit_form");
        assert_eq!(normalize_slug("__leading_trailing__"), "leading_trailing");
        assert_eq!(normalize_slug("a!@#b"), "a_b");
        assert_eq!(normalize_slug("123abc"), "123abc");
    }

    #[test]
    fn normalize_collapses_runs() {
        assert_eq!(normalize_slug("a   b---c"), "a_b_c");
    }

    #[test]
    fn normalize_empty() {
        assert_eq!(normalize_slug(""), "");
        assert_eq!(normalize_slug("___"), "");
    }

    #[test]
    fn slug_priority_ai_name_wins() {
        let v = json!({
            "id": "btn-1",
            "semantics": { "aiName": "submit_form", "label": "Submit" },
            "content": "Send"
        });
        assert_eq!(slug_source(&v), "submit_form");
    }

    #[test]
    fn slug_falls_back_through_priority_chain() {
        let v = json!({ "id": "btn-1", "semantics": { "label": "Sign In" } });
        assert_eq!(slug_source(&v), "Sign In");
        let v2 = json!({ "id": "btn-1", "content": "Click me" });
        assert_eq!(slug_source(&v2), "Click me");
        let v3 = json!({ "id": "node-99" });
        assert_eq!(slug_source(&v3), "node-99");
    }

    #[test]
    fn compute_slug_normalises_and_falls_back() {
        let v = json!({ "id": "btn-1", "semantics": { "label": "Sign In" } });
        assert_eq!(compute_slug(&v), "sign_in");
        let only_id = json!({ "id": "weird-id-99" });
        assert_eq!(compute_slug(&only_id), "weird_id_99");
        let empty = json!({ "id": "" });
        assert_eq!(compute_slug(&empty), "node");
    }

    #[test]
    fn short_hash_4_hex_chars_deterministic() {
        let salt = [0x42; 16];
        let a = short_hash("button-1", &salt);
        let b = short_hash("button-1", &salt);
        assert_eq!(a, b);
        assert_eq!(a.len(), 4);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn short_hash_differs_per_id_and_salt() {
        let s1 = [0u8; 16];
        let s2 = [1u8; 16];
        assert_ne!(short_hash("a", &s1), short_hash("b", &s1));
        assert_ne!(short_hash("same", &s1), short_hash("same", &s2));
    }

    #[test]
    fn has_ai_name_detects_author_override() {
        let with = json!({ "semantics": { "aiName": "x" } });
        let without = json!({ "semantics": { "label": "x" } });
        assert!(has_ai_name(&with));
        assert!(!has_ai_name(&without));
    }
}
