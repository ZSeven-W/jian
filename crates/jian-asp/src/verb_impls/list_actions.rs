//! `list_actions` projection helpers — Plan 18 ASP prod mode / C1.
//!
//! C0 stubbed `run_list_actions` to return an empty array. This
//! module wires the real projection: derive the same
//! `<scope>.<verb-prefix-slug>_<hash4>` action names that
//! `jian-action-surface` produces, then flatten each into an
//! [`ActionRow`][crate::protocol::ActionRow] with the canonical
//! event names from spec §12 (`tap` / `set` / `submit` / `scroll`
//! / `swipe` / `open` / …).
//!
//! ## Why share names with Action Surface
//!
//! Spec §C1 / §12: a single agent client must be able to switch
//! transport between MCP (`jian-action-surface`) and ASP (this
//! module) without re-learning ids. Reusing
//! `ActionDefinition::full_name` is what makes that property hold.
//!
//! ## What we filter
//!
//! `AvailabilityStatic::StaticHidden` — the author flipped
//! `semantics.aiHidden = true` — drops out. So does
//! `AvailabilityStatic::ConfirmGated` (destructive handlers gated
//! behind `confirm:` / `fetch DELETE` / etc.). The agent's
//! `list_actions` view matches MCP's default
//! `include_confirm_gated: false` policy. Authors who need an
//! agent to see a destructive action surface it via Action Surface
//! today; ASP prod follows the same default to keep the two
//! channels aligned.
//!
//! Dynamic state-gating (`bindings.visible == false` /
//! `bindings.disabled == true` against the live `StateGraph`) is
//! C2's job; this module's projection is the static / structural
//! side.

use crate::protocol::ActionRow;
use jian_core::action_surface::{ActionDefinition, AvailabilityStatic, SourceKind};

/// Default page size when `limit` is omitted on the
/// [`crate::protocol::Verb::ListActions`] request. Tighter than
/// the spec's `LIST_ACTIONS_MAX_LIMIT` (1000) so a casual call
/// fits in one page without specifying anything; agents that want
/// to see everything pass `limit: 1000` explicitly.
pub const LIST_ACTIONS_DEFAULT_LIMIT: u32 = 200;

/// Project a derived `ActionDefinition` slice into the flat
/// `ActionRow` rows the wire format carries. Pure function — no
/// runtime borrow, no allocations beyond the output list — so
/// unit tests can pin the projection without spinning up a
/// `Runtime`.
///
/// Filters:
/// - `AvailabilityStatic::Available` actions only. `StaticHidden`
///   is already excluded by `derive_actions` itself; the explicit
///   match here documents the intent and survives a future
///   refactor that might let `StaticHidden` rows leak.
/// - `ConfirmGated` actions are excluded by default. ASP prod's
///   policy mirrors MCP's `include_confirm_gated: false`.
pub fn project_actions(actions: &[ActionDefinition]) -> Vec<ActionRow> {
    actions
        .iter()
        .filter(|a| matches!(a.status, AvailabilityStatic::Available))
        .map(|a| ActionRow {
            id: a.full_name(),
            events: source_kind_to_events(a.source_kind),
        })
        .collect()
}

/// Map [`SourceKind`] → spec §12 `events` strings. The mapping is
/// "what does the agent invoke to fire this action?":
///
/// | SourceKind        | events     | Why                                       |
/// |-------------------|------------|-------------------------------------------|
/// | Tap / DoubleTap / LongPress | `["tap"]` | All synthesise a pointer down/up at the source node's centre. ASP prod's `tap` verb covers all three. |
/// | Confirm / Dismiss | `["tap"]`  | Confirm-gated tap variants — agent UX is identical to a normal tap; the destructive-confirmation flow lives in the runtime, not the agent.  |
/// | SetValue          | `["set"]`  | `bind:value` on a text-input — ASP prod's `type` verb writes the value via the state graph.  |
/// | Submit            | `["submit"]` | `events.onSubmit` on a form. |
/// | OpenRoute         | `["open"]` | `route:` action on a link / button. |
/// | SwipeLeft/Right/Up/Down | `["swipe"]` | All four directions collapse to one event name; the verb's `direction` parameter picks. |
/// | Scroll / LoadMore | `["scroll"]` | Scroll containers + the spec's "fetch the next page" intent. |
fn source_kind_to_events(kind: SourceKind) -> Vec<String> {
    match kind {
        SourceKind::Tap
        | SourceKind::DoubleTap
        | SourceKind::LongPress
        | SourceKind::Confirm
        | SourceKind::Dismiss => vec!["tap".to_owned()],
        SourceKind::SetValue => vec!["set".to_owned()],
        SourceKind::Submit => vec!["submit".to_owned()],
        SourceKind::OpenRoute => vec!["open".to_owned()],
        SourceKind::SwipeLeft
        | SourceKind::SwipeRight
        | SourceKind::SwipeUp
        | SourceKind::SwipeDown => vec!["swipe".to_owned()],
        SourceKind::Scroll | SourceKind::LoadMore => vec!["scroll".to_owned()],
    }
}

/// Compute a short revision tag for a row set. The tag is FNV-1a
/// over each row's id (events are derived from ids and don't
/// independently affect identity), truncated to 8 hex chars. Only
/// needs to be **stable for the same input** — collisions on a
/// hot-reload doc rotation are vanishingly unlikely at 32 bits and
/// the worst case is "looks like the same revision" which is the
/// same false positive a 64-bit hash would have just less often.
///
/// Spec §12 / codex round 1 MEDIUM: a bare numeric cursor is
/// dangerous across hot-reloads because a stale index could land
/// on a different action in the new derivation. Tagging the cursor
/// with the current revision lets the next call detect mismatch
/// and return `invalid cursor` so the agent re-fetches from page 0.
pub fn revision_tag(rows: &[ActionRow]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for r in rows {
        for b in r.id.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        // Separator so concatenation is unambiguous.
        h ^= 0xff;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h as u32))
}

/// Slice `rows` per spec §12 pagination. Cursor scheme:
/// `"<revision>:<next-row-index>"` — `revision` is
/// [`revision_tag`] over the same row set; the suffix is the
/// decimal-encoded zero-based row index to resume from. Clients
/// don't parse the cursor; they pass it back verbatim.
///
/// Codex round 1 MEDIUM: cursors used to be a bare decimal index
/// with no revision tag, which silently mis-paginated across a
/// hot-reload. With the tag, a stale cursor whose revision differs
/// from the current rows surfaces as `invalid cursor` and the
/// client re-fetches from page 0.
///
/// Returns `Err(msg)` for malformed cursors, mismatched revisions,
/// or out-of-range indices. Returns `Ok((page, next))` where
/// `next` is `Some(...)` when more rows remain, `None` when
/// exhausted.
pub fn paginate(
    rows: Vec<ActionRow>,
    cursor: Option<&str>,
    limit: u32,
) -> Result<(Vec<ActionRow>, Option<String>), &'static str> {
    let revision = revision_tag(&rows);
    let start = match cursor {
        None | Some("") => 0usize,
        Some(c) => {
            // `<revision>:<index>` shape; reject anything else
            // (including the legacy bare-number form so an old
            // client gets a clean stale-cursor signal).
            let (rev, idx) = c.split_once(':').ok_or("invalid cursor")?;
            if rev != revision {
                return Err("invalid cursor");
            }
            idx.parse::<usize>().map_err(|_| "invalid cursor")?
        }
    };
    if start > rows.len() {
        return Err("invalid cursor");
    }
    let end = (start + limit as usize).min(rows.len());
    let slice = rows[start..end].to_vec();
    let next = if end < rows.len() {
        Some(format!("{revision}:{end}"))
    } else {
        None
    };
    Ok((slice, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::action_surface::{ActionDefinition, ActionName, AvailabilityStatic, Scope};

    fn def(
        scope: Scope,
        slug: &str,
        kind: SourceKind,
        status: AvailabilityStatic,
    ) -> ActionDefinition {
        ActionDefinition {
            name: ActionName {
                scope,
                slug: slug.to_owned(),
            },
            source_node_id: format!("node-{slug}"),
            source_kind: kind,
            description: String::new(),
            status,
            aliases: Vec::new(),
            params: Vec::new(),
            has_explicit_name: false,
        }
    }

    #[test]
    fn project_actions_drops_hidden_and_confirm_gated() {
        let actions = vec![
            def(
                Scope::page("home"),
                "save_a1b2",
                SourceKind::Tap,
                AvailabilityStatic::Available,
            ),
            def(
                Scope::page("home"),
                "delete_c3d4",
                SourceKind::Tap,
                AvailabilityStatic::ConfirmGated,
            ),
            def(
                Scope::page("home"),
                "secret_e5f6",
                SourceKind::Tap,
                AvailabilityStatic::StaticHidden,
            ),
        ];
        let rows = project_actions(&actions);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "home.save_a1b2");
        assert_eq!(rows[0].events, vec!["tap".to_owned()]);
    }

    #[test]
    fn source_kind_to_events_covers_every_variant() {
        // Pin the spec §12 mapping. If a new SourceKind variant lands,
        // this test trips the exhaustiveness so the projector gets
        // updated in the same commit.
        let cases: &[(SourceKind, &[&str])] = &[
            (SourceKind::Tap, &["tap"]),
            (SourceKind::DoubleTap, &["tap"]),
            (SourceKind::LongPress, &["tap"]),
            (SourceKind::Confirm, &["tap"]),
            (SourceKind::Dismiss, &["tap"]),
            (SourceKind::SetValue, &["set"]),
            (SourceKind::Submit, &["submit"]),
            (SourceKind::OpenRoute, &["open"]),
            (SourceKind::SwipeLeft, &["swipe"]),
            (SourceKind::SwipeRight, &["swipe"]),
            (SourceKind::SwipeUp, &["swipe"]),
            (SourceKind::SwipeDown, &["swipe"]),
            (SourceKind::Scroll, &["scroll"]),
            (SourceKind::LoadMore, &["scroll"]),
        ];
        for (kind, expected) in cases {
            let got = source_kind_to_events(*kind);
            let got_strs: Vec<&str> = got.iter().map(String::as_str).collect();
            assert_eq!(got_strs, *expected, "{kind:?} → {expected:?}");
        }
    }

    #[test]
    fn paginate_returns_full_page_when_under_limit() {
        let rows = vec![
            ActionRow {
                id: "a".into(),
                events: vec!["tap".into()],
            },
            ActionRow {
                id: "b".into(),
                events: vec!["set".into()],
            },
        ];
        let (page, next) = paginate(rows.clone(), None, 200).expect("ok");
        assert_eq!(page.len(), 2);
        assert!(next.is_none());
    }

    #[test]
    fn paginate_truncates_and_emits_cursor() {
        let rows: Vec<ActionRow> = (0..10)
            .map(|i| ActionRow {
                id: format!("row-{i}"),
                events: vec!["tap".into()],
            })
            .collect();
        let (page, next) = paginate(rows.clone(), None, 4).expect("ok");
        assert_eq!(page.len(), 4);
        // Cursor shape: `<revision>:<index>`.
        let next = next.expect("more rows");
        assert!(next.contains(':'));
        let (rev, idx) = next.split_once(':').unwrap();
        assert_eq!(rev, revision_tag(&rows));
        assert_eq!(idx, "4");
    }

    #[test]
    fn paginate_resumes_from_cursor() {
        let rows: Vec<ActionRow> = (0..10)
            .map(|i| ActionRow {
                id: format!("row-{i}"),
                events: vec!["tap".into()],
            })
            .collect();
        // First call yields a tagged cursor; second call must accept it.
        let (_first, next1) = paginate(rows.clone(), None, 4).expect("ok");
        let (page, next) = paginate(rows.clone(), next1.as_deref(), 4).expect("ok");
        assert_eq!(page[0].id, "row-4");
        assert_eq!(page.len(), 4);
        let next = next.expect("more rows");
        assert!(next.starts_with(&revision_tag(&rows)));
    }

    #[test]
    fn paginate_emits_none_cursor_on_last_partial_page() {
        let rows: Vec<ActionRow> = (0..5)
            .map(|i| ActionRow {
                id: format!("row-{i}"),
                events: vec!["tap".into()],
            })
            .collect();
        let cursor = format!("{}:3", revision_tag(&rows));
        let (page, next) = paginate(rows, Some(&cursor), 4).expect("ok");
        assert_eq!(page.len(), 2);
        assert!(next.is_none(), "last partial page sets next_cursor=None");
    }

    #[test]
    fn paginate_rejects_malformed_cursor() {
        let rows = vec![ActionRow {
            id: "a".into(),
            events: vec!["tap".into()],
        }];
        // Bare-number form is now rejected (was the legacy form).
        let err = paginate(rows.clone(), Some("4"), 4).unwrap_err();
        assert_eq!(err, "invalid cursor");
        // Garbage tag.
        let err = paginate(rows.clone(), Some("garbage:not-a-number"), 4).unwrap_err();
        assert_eq!(err, "invalid cursor");
    }

    #[test]
    fn paginate_rejects_cursor_beyond_end() {
        let rows = vec![ActionRow {
            id: "a".into(),
            events: vec!["tap".into()],
        }];
        let cursor = format!("{}:99", revision_tag(&rows));
        let err = paginate(rows, Some(&cursor), 4).unwrap_err();
        assert_eq!(err, "invalid cursor");
    }

    #[test]
    fn paginate_rejects_stale_revision_after_action_set_changes() {
        // Codex round 1 MEDIUM: a hot-reload between paginated calls
        // must surface as `invalid cursor` so the client re-fetches
        // from page 0 instead of mis-paginating against a different
        // action set.
        let rows_a: Vec<ActionRow> = (0..10)
            .map(|i| ActionRow {
                id: format!("a-{i}"),
                events: vec!["tap".into()],
            })
            .collect();
        let (_p, cursor_a) = paginate(rows_a.clone(), None, 4).expect("ok");
        let cursor_a = cursor_a.expect("more rows");
        // Simulate hot-reload: different rows, different revision.
        let rows_b: Vec<ActionRow> = (0..10)
            .map(|i| ActionRow {
                id: format!("b-{i}"),
                events: vec!["tap".into()],
            })
            .collect();
        assert_ne!(revision_tag(&rows_a), revision_tag(&rows_b));
        let err = paginate(rows_b, Some(&cursor_a), 4).unwrap_err();
        assert_eq!(err, "invalid cursor");
    }

    #[test]
    fn paginate_treats_empty_cursor_string_as_no_cursor() {
        let rows = vec![ActionRow {
            id: "a".into(),
            events: vec!["tap".into()],
        }];
        let (page, _) = paginate(rows, Some(""), 4).expect("empty cursor ok");
        assert_eq!(page.len(), 1);
    }
}
