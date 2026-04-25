//! `derive_actions(doc, build_salt)` — pure derivation function.
//!
//! Given a `PenDocument`, walk every page + child node and apply spec
//! §3.2's rules to produce an ordered `Vec<ActionDefinition>`. Result
//! is **bitwise stable** for the same `(doc, build_salt)` pair —
//! covered by `derive_is_deterministic` in the test suite.
//!
//! Phase 1 implements the user-intent rules:
//! - `events.onTap` → `<slug>`
//! - `events.onDoubleTap` → `double_tap_<slug>`
//! - `events.onLongPress` → `long_press_<slug>`
//! - `events.onSubmit` → `submit_<slug>`
//! - `bindings["bind:value"]` → `set_<slug>(value)` (input-style nodes)
//! - `route.push` → `open_<slug>(p)` (route params from `RouteSpec.params`)
//!
//! Swipe / scroll / key actions are deferred until the gesture arena
//! exposes pan-direction + key + wheel events through the schema.

use super::naming::{compute_slug, has_ai_name, short_hash};
use super::types::{
    ActionDefinition, ActionName, AvailabilityStatic, ParamSpec, ParamTy, Scope, SourceKind,
};
use jian_ops_schema::document::PenDocument;
use serde_json::Value;

/// Spec §3.4 collision warnings emitted by `derive_actions`. Each
/// entry names every action involved (so an editor panel can
/// highlight both / all of them) plus the colliding full name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeriveWarning {
    /// Two or more **author-supplied** `aiName`s resolved to the same
    /// full `<scope>.<slug>`. Spec §3.4: every action involved is
    /// downgraded to `StaticHidden` and the author is expected to
    /// rename one. Surfaced through the OpenPencil editor's AI
    /// Actions Panel as a red banner.
    AiNameCollision { full_name: String, source_node_ids: Vec<String> },
    /// A bare or fully-qualified alias claimed a name another action
    /// already owned (or shared). Same StaticHidden treatment.
    AliasCollision { full_name: String, source_node_ids: Vec<String> },
    /// Auto-derived slugs (no `aiName`) collided after the hash4
    /// suffix. Each colliding action got a numeric suffix `_1` /
    /// `_2` / … rather than being hidden — they remain callable, the
    /// suffix is just internal disambiguation.
    AutoSlugDisambiguated { base: String, count: usize },
}

/// Walk `doc` and emit the deterministic action list. Convenience
/// wrapper around [`derive_actions_with_warnings`] for callers that
/// don't care about §3.4 collision warnings.
pub fn derive_actions(doc: &PenDocument, build_salt: &[u8; 16]) -> Vec<ActionDefinition> {
    derive_actions_with_warnings(doc, build_salt).0
}

/// Walk `doc` and emit the deterministic action list **plus**
/// load-time §3.4 warnings. Same bitwise-stable derivation as
/// `derive_actions` — the warnings are populated as a side-product
/// of the existing collision pass.
///
/// `build_salt` is the compile-time disambiguator (typically derived
/// from the package version + git rev) — same input ⇒ same output,
/// byte-for-byte.
///
/// Spec §3.4 conflict handling:
/// - Author-supplied `aiName` collisions → all involved actions
///   downgraded to `StaticHidden`; one `AiNameCollision` warning.
/// - Alias collisions (an alias matching another action's full name)
///   → same StaticHidden treatment + `AliasCollision` warning.
/// - Auto-derived slug + hash4 collisions (rare 16-bit hash hits in
///   very large docs) → numeric `_1` / `_2` suffix appended in
///   derivation order, action stays Available, single
///   `AutoSlugDisambiguated` warning per base name.
pub fn derive_actions_with_warnings(
    doc: &PenDocument,
    build_salt: &[u8; 16],
) -> (Vec<ActionDefinition>, Vec<DeriveWarning>) {
    let mut out = Vec::new();
    let doc_json = match serde_json::to_value(doc) {
        Ok(v) => v,
        Err(_) => return (out, Vec::new()),
    };

    if let Some(pages) = doc_json.get("pages").and_then(|v| v.as_array()) {
        for page in pages {
            let page_id = page
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("page")
                .to_owned();
            let scope_resolver = ScopeResolver::page(&page_id);
            if let Some(children) = page.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    walk(child, &doc_json, &scope_resolver, build_salt, &mut out);
                }
            }
        }
    }

    if let Some(children) = doc_json.get("children").and_then(|v| v.as_array()) {
        // Document-level children fall back to global scope.
        let scope_resolver = ScopeResolver::global();
        for child in children {
            walk(child, &doc_json, &scope_resolver, build_salt, &mut out);
        }
    }

    let auto_warnings = disambiguate_auto_slugs(&mut out);
    let collision_warnings = flag_name_collisions(&mut out);
    let mut warnings = collision_warnings;
    warnings.extend(auto_warnings);
    (out, warnings)
}

/// Walk the derived list, group by full name, downgrade every member
/// of an aiName / alias collision to `StaticHidden`, and emit one
/// `DeriveWarning` per colliding name. **Auto-slug collisions are
/// handled separately** by `disambiguate_auto_slugs` — by the time
/// this pass runs they've already been suffixed, so the only
/// collisions we still see come from author-supplied aiName / alias.
///
/// **De-dup per action**: an action that keeps its own primary as an
/// alias (`home.save` with `aiAliases: ["save"]`) or repeats an
/// alias should still register that name *once*.
///
/// **Same-source dedup**: a single PenNode can derive multiple
/// actions (`onTap` + `onLongPress`), and `aiAliases` is currently
/// node-level so every derived action picks up the same alias list.
/// Counting each node-level alias once per action would self-collide
/// every time. We dedup by `(name, source_node_id)` in the count
/// pass, then collapse to per-name groups for warning + status.
fn flag_name_collisions(actions: &mut [ActionDefinition]) -> Vec<DeriveWarning> {
    use std::collections::{BTreeMap, HashSet};
    let mut name_to_actions: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut name_is_alias: BTreeMap<String, bool> = BTreeMap::new();
    // (name, source_node_id) → first idx already counted. A second
    // action from the same source claiming the same name doesn't
    // double-count (the §3.4 rule fires on cross-node collisions
    // only).
    let mut counted: HashSet<(String, String)> = HashSet::new();

    for (idx, a) in actions.iter().enumerate() {
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(a.name.full());
        for alias in &a.aliases {
            seen.insert(alias.full());
        }
        for name in seen {
            let key = (name.clone(), a.source_node_id.clone());
            if !counted.insert(key) {
                continue;
            }
            let is_alias = name != a.name.full();
            name_to_actions.entry(name.clone()).or_default().push(idx);
            // True if *any* action holds this name as an alias.
            name_is_alias
                .entry(name)
                .and_modify(|v| *v = *v || is_alias)
                .or_insert(is_alias);
        }
    }
    let mut warnings = Vec::new();
    for (name, idxs) in &name_to_actions {
        if idxs.len() <= 1 {
            continue;
        }
        let source_node_ids: Vec<String> = idxs
            .iter()
            .map(|i| actions[*i].source_node_id.clone())
            .collect();
        if name_is_alias.get(name).copied().unwrap_or(false) {
            warnings.push(DeriveWarning::AliasCollision {
                full_name: name.clone(),
                source_node_ids,
            });
        } else {
            warnings.push(DeriveWarning::AiNameCollision {
                full_name: name.clone(),
                source_node_ids,
            });
        }
        for &i in idxs {
            actions[i].status = AvailabilityStatic::StaticHidden;
        }
    }
    warnings
}

/// Resolve auto-derived slug collisions by appending a numeric `_1`
/// / `_2` / … suffix in derivation order. Auto-derived means
/// `has_explicit_name == false`. Authored aiName collisions still
/// flow into `flag_name_collisions` afterwards.
///
/// **Reservation step**: explicit names are collected first so an
/// auto-derived slug that happens to match an explicit `aiName`
/// gets bumped to `_1` and the author-stable action keeps the
/// pristine slug — instead of both falling into `flag_name_collisions`
/// and the explicit action getting hidden alongside the auto.
///
/// Returns one warning per collided base name so editors can show a
/// "rename suggestion" hint — none of the suffixed actions become
/// hidden, just disambiguated.
fn disambiguate_auto_slugs(actions: &mut [ActionDefinition]) -> Vec<DeriveWarning> {
    use std::collections::{BTreeMap, HashSet};
    // Reserved names = every explicit primary + alias. Auto-derived
    // entries that hit any of these get bumped, so the author-stable
    // action keeps the unsuffixed slug.
    let mut reserved: HashSet<String> = HashSet::new();
    for a in actions.iter() {
        if a.has_explicit_name {
            reserved.insert(a.name.full());
        }
        for alias in &a.aliases {
            reserved.insert(alias.full());
        }
    }
    // Group auto-derived entries that share a full name *or* hit
    // a reserved name. Each group bumps every member that collides.
    let mut name_to_indices: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, a) in actions.iter().enumerate() {
        if !a.has_explicit_name {
            name_to_indices.entry(a.name.full()).or_default().push(idx);
        }
    }
    let mut warnings = Vec::new();
    for (base, idxs) in name_to_indices {
        let collides_with_reserved = reserved.contains(&base);
        if idxs.len() <= 1 && !collides_with_reserved {
            continue;
        }
        warnings.push(DeriveWarning::AutoSlugDisambiguated {
            base: base.clone(),
            count: idxs.len(),
        });
        // Rule: when colliding with a reserved (explicit) name, all
        // auto entries get a suffix — the explicit owner keeps the
        // pristine slug. Otherwise the first auto keeps the bare
        // slug (consistent with the previous behaviour).
        let skip_first = if collides_with_reserved { 0 } else { 1 };
        for (n, &i) in idxs.iter().enumerate().skip(skip_first) {
            let suffix = if collides_with_reserved { n + 1 } else { n };
            actions[i].name.slug = format!("{}_{}", actions[i].name.slug, suffix);
        }
    }
    warnings
}

fn walk(
    node: &Value,
    doc_json: &Value,
    parent_scope: &ScopeResolver,
    build_salt: &[u8; 16],
    out: &mut Vec<ActionDefinition>,
) {
    let scope = parent_scope.refine(node);
    emit_for_node(node, doc_json, &scope, build_salt, out);
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        let next = ScopeResolver::from_scope(scope);
        for child in children {
            walk(child, doc_json, &next, build_salt, out);
        }
    }
}

fn emit_for_node(
    node: &Value,
    doc_json: &Value,
    scope: &Scope,
    build_salt: &[u8; 16],
    out: &mut Vec<ActionDefinition>,
) {
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let slug = compute_slug(node);
    let suffixed = if has_ai_name(node) {
        slug.clone()
    } else {
        format!("{}_{}", slug, short_hash(id, build_salt))
    };
    let description = node
        .get("semantics")
        .and_then(|s| s.get("aiDescription"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let aliases = node
        .get("semantics")
        .and_then(|s| s.get("aiAliases"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let events = node.get("events").and_then(|v| v.as_object());

    // Verb-style actions (no parameters). Spec §3.2 requires the
    // event handler to be **non-empty**; an `onTap: []` stub doesn't
    // express user intent and shouldn't surface as a callable action.
    let verb_rules: [(&str, &str, SourceKind); 4] = [
        ("onTap", "", SourceKind::Tap),
        ("onDoubleTap", "double_tap_", SourceKind::DoubleTap),
        ("onLongPress", "long_press_", SourceKind::LongPress),
        ("onSubmit", "submit_", SourceKind::Submit),
    ];
    for (event_key, prefix, kind) in verb_rules {
        if let Some(handler) = events.and_then(|e| e.get(event_key)) {
            if !is_non_empty_action_list(handler) {
                continue;
            }
            let slug_v = format!("{}{}", prefix, suffixed);
            out.push(make_action(
                scope,
                &slug_v,
                id,
                kind,
                description.clone(),
                &aliases,
                node,
                Some(handler),
                Vec::new(),
            ));
        }
    }

    // --- onScroll / onReachEnd → load_more_<slug>
    // Spec §3.2 maps either event on a list / feed container to the
    // agent's "load the next page" intent. We pick whichever is
    // present (with onReachEnd preferred since it explicitly fires on
    // pagination boundaries). Empty handlers are skipped to match the
    // 非空 rule shared by every other derivation.
    if let Some(handler) = events
        .and_then(|e| e.get("onReachEnd"))
        .filter(|h| is_non_empty_action_list(h))
        .or_else(|| events.and_then(|e| e.get("onScroll")).filter(|h| is_non_empty_action_list(h)))
    {
        let slug_v = format!("load_more_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::LoadMore,
            description.clone(),
            &aliases,
            node,
            Some(handler),
            Vec::new(),
        ));
    }

    // --- bindings["bind:value"] → set_<slug>(value: typeof($state.X))
    if let Some(target) = bind_target(node) {
        let ty = state_type_for_path(doc_json, &target).unwrap_or(ParamTy::Unknown);
        let slug_v = format!("set_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::SetValue,
            description.clone(),
            &aliases,
            node,
            None,
            vec![ParamSpec {
                name: "value".to_owned(),
                ty,
            }],
        ));
    }

    // --- route.push → open_<slug>(p₁: ..., p₂: ...)
    let route_push = node
        .get("route")
        .and_then(|r| r.get("push"))
        .and_then(|v| v.as_str());
    if let Some(path_pattern) = route_push {
        let params = route_param_specs(doc_json, path_pattern);
        let slug_v = format!("open_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::OpenRoute,
            description.clone(),
            &aliases,
            node,
            None,
            params,
        ));
    }
}

/// Parse an `aiAliases` entry. When the entry contains a scope
/// separator (e.g. `"home.sign_in_a3f7"`), interpret it as a fully
/// qualified `<scope>.<slug>`; otherwise treat it as a slug in the
/// owning node's scope. Without this distinction `aiAliases:
/// ["home.old"]` resolves as `home.home.old` and never matches a
/// real action — which was the previous bug.
fn parse_alias(raw: &str, default_scope: &Scope) -> ActionName {
    if let Some((scope_part, slug_part)) = split_qualified(raw) {
        ActionName {
            scope: Scope(scope_part.to_owned()),
            slug: slug_part.to_owned(),
        }
    } else {
        ActionName {
            scope: default_scope.clone(),
            slug: raw.to_owned(),
        }
    }
}

/// Split a candidate full name into `(scope, slug)`. We accept the
/// three scope shapes the derivation produces:
/// - `modal.<dialog_id>.<slug>` (3+ segments, modal prefix)
/// - `global.<slug>`
/// - `<page_id>.<slug>`
/// The caller distinguishes via the dotted form. We require the
/// remainder after the first `.` to be a non-empty slug.
fn split_qualified(raw: &str) -> Option<(&str, &str)> {
    if let Some(rest) = raw.strip_prefix("modal.") {
        // modal.<dialog>.<slug>
        let (dialog, slug) = rest.split_once('.')?;
        if dialog.is_empty() || slug.is_empty() {
            return None;
        }
        let scope_end = "modal.".len() + dialog.len();
        Some((&raw[..scope_end], slug))
    } else {
        let (scope, slug) = raw.split_once('.')?;
        if scope.is_empty() || slug.is_empty() {
            return None;
        }
        Some((scope, slug))
    }
}

/// True when `handler` is an array with at least one entry. Spec
/// §3.2 phrases every event-source rule as "events.X 非空"; an empty
/// stub list shouldn't surface a callable action.
fn is_non_empty_action_list(handler: &Value) -> bool {
    match handler {
        Value::Array(a) => !a.is_empty(),
        Value::Null => false,
        _ => true,
    }
}

/// Extract `bindings["bind:value"]` and validate it points at a
/// writable `$state.<path>` — bindings to `$route` / `$app` /
/// computed expressions don't get a `set_*` action because the
/// runtime can't write to them directly.
fn bind_target(node: &Value) -> Option<String> {
    let raw = node
        .get("bindings")
        .and_then(|b| b.get("bind:value"))
        .and_then(|v| v.as_str())?;
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix("$state.")?;
    if rest.is_empty() || rest.contains(|c: char| c.is_whitespace()) {
        return None;
    }
    Some(rest.to_owned())
}

/// Look up the declared `$state.<path>` type in the document's `state`
/// schema. Supports dotted keys *and* `[idx]` array indexing
/// (e.g. `items[0].title` or `items.0.title`) — runtime accepts both
/// forms.
fn state_type_for_path(doc_json: &Value, path: &str) -> Option<ParamTy> {
    let segments = parse_path_segments(path)?;
    let mut iter = segments.into_iter();
    let head = iter.next()?;
    let head_key = match head {
        PathSegment::Key(k) => k,
        PathSegment::Index(_) => return None, // `$state.[0]` is invalid
    };
    let entry = doc_json.get("state")?.as_object()?.get(&head_key)?.get("type")?;
    let mut current = entry.clone();
    for seg in iter {
        current = traverse_type(&current, &seg)?;
    }
    Some(primitive_for(&current))
}

#[derive(Debug)]
enum PathSegment {
    Key(String),
    Index(i64),
}

/// Parse `a.b[0].c` / `a.0.c` into `[Key("a"), Key("b"), Index(0), Key("c")]`.
fn parse_path_segments(path: &str) -> Option<Vec<PathSegment>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_bracket = false;
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' if !in_bracket => {
                if !cur.is_empty() {
                    push_segment(&mut out, std::mem::take(&mut cur));
                }
            }
            '[' if !in_bracket => {
                if !cur.is_empty() {
                    push_segment(&mut out, std::mem::take(&mut cur));
                }
                in_bracket = true;
            }
            ']' if in_bracket => {
                let n: i64 = cur.parse().ok()?;
                out.push(PathSegment::Index(n));
                cur.clear();
                in_bracket = false;
                if let Some('.') = chars.peek() {
                    chars.next();
                }
            }
            _ => cur.push(c),
        }
    }
    if in_bracket {
        return None;
    }
    if !cur.is_empty() {
        push_segment(&mut out, cur);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn push_segment(out: &mut Vec<PathSegment>, raw: String) {
    if let Ok(n) = raw.parse::<i64>() {
        out.push(PathSegment::Index(n));
    } else {
        out.push(PathSegment::Key(raw));
    }
}

fn traverse_type(ty: &Value, seg: &PathSegment) -> Option<Value> {
    let obj = ty.as_object()?;
    match seg {
        PathSegment::Key(k) => {
            // `{ object: { ... } }` — descend by named key.
            obj.get("object")?.get(k).cloned()
        }
        PathSegment::Index(_) => {
            // `{ array: T }` — index doesn't matter, type stays T.
            obj.get("array").cloned()
        }
    }
}

fn primitive_for(ty: &Value) -> ParamTy {
    match ty.as_str() {
        Some("int") => ParamTy::Int,
        Some("float") => ParamTy::Float,
        Some("number") => ParamTy::Number,
        Some("string") => ParamTy::String,
        Some("bool") => ParamTy::Bool,
        Some("date") => ParamTy::Date,
        _ => ParamTy::Unknown,
    }
}

/// Parse `:param` segments out of a route path and look up declared
/// types in `routes.routes[<path>].params`. Missing declarations
/// default to `String` per spec §3.5.
fn route_param_specs(doc_json: &Value, path_pattern: &str) -> Vec<ParamSpec> {
    let mut specs = Vec::new();
    let declared = doc_json
        .get("routes")
        .and_then(|r| r.get("routes"))
        .and_then(|m| m.as_object())
        .and_then(|m| m.get(path_pattern))
        .and_then(|spec| spec.get("params"))
        .and_then(|p| p.as_object())
        .cloned();

    for seg in path_pattern.split('/').filter(|s| !s.is_empty()) {
        if let Some(name) = seg.strip_prefix(':') {
            let ty = declared
                .as_ref()
                .and_then(|m| m.get(name))
                .map(primitive_for)
                .unwrap_or(ParamTy::String);
            specs.push(ParamSpec {
                name: name.to_owned(),
                ty,
            });
        }
    }
    specs
}

#[allow(clippy::too_many_arguments)]
fn make_action(
    scope: &Scope,
    slug: &str,
    source_node_id: &str,
    source_kind: SourceKind,
    description: Option<String>,
    aliases: &[String],
    node: &Value,
    handler: Option<&Value>,
    params: Vec<ParamSpec>,
) -> ActionDefinition {
    let name = ActionName {
        scope: scope.clone(),
        slug: slug.to_owned(),
    };
    let alias_names = aliases.iter().map(|a| parse_alias(a, scope)).collect();
    let status = super::availability::classify(node, handler);
    let auto_desc = description
        .or_else(|| auto_describe(source_kind, slug))
        .unwrap_or_default();
    ActionDefinition {
        name,
        source_node_id: source_node_id.to_owned(),
        source_kind,
        description: auto_desc,
        status,
        aliases: alias_names,
        params,
        has_explicit_name: super::naming::has_ai_name(node),
    }
}

fn auto_describe(kind: SourceKind, slug: &str) -> Option<String> {
    Some(match kind {
        SourceKind::Tap => format!("Tap {}", slug),
        SourceKind::DoubleTap => format!("Double-tap {}", slug),
        SourceKind::LongPress => format!("Long-press {}", slug),
        SourceKind::Submit => format!("Submit {}", slug),
        SourceKind::SetValue => format!("Set the value of {}", slug),
        SourceKind::OpenRoute => format!("Open {}", slug),
        SourceKind::SwipeLeft => format!("Swipe left on {}", slug),
        SourceKind::SwipeRight => format!("Swipe right on {}", slug),
        SourceKind::SwipeUp => format!("Swipe up on {}", slug),
        SourceKind::SwipeDown => format!("Swipe down on {}", slug),
        SourceKind::Scroll => format!("Scroll {}", slug),
        SourceKind::LoadMore => format!("Load more {}", slug),
        SourceKind::Confirm => format!("Confirm {}", slug),
        SourceKind::Dismiss => format!("Dismiss {}", slug),
    })
}

/// Tracks the current scope and refines as we descend. A child sitting
/// inside a `dialog` ancestor switches scope to `modal.<dialog_id>`;
/// otherwise the parent scope carries through.
struct ScopeResolver {
    current: Scope,
}

impl ScopeResolver {
    fn page(page_id: &str) -> Self {
        Self {
            current: Scope::page(page_id),
        }
    }
    fn global() -> Self {
        Self {
            current: Scope::global(),
        }
    }
    fn from_scope(scope: Scope) -> Self {
        Self { current: scope }
    }

    fn refine(&self, node: &Value) -> Scope {
        let role = node
            .get("semantics")
            .and_then(|s| s.get("role"))
            .and_then(|v| v.as_str());
        if role == Some("dialog") {
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("dialog");
            return Scope::modal(id);
        }
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::document::PenDocument;

    fn doc_from(json: &str) -> PenDocument {
        serde_json::from_str(json).expect("schema must parse")
    }

    #[test]
    fn empty_document_yields_no_actions() {
        let doc = doc_from(r#"{ "version":"0.8.0", "children":[] }"#);
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert!(acts.is_empty());
    }

    #[test]
    fn on_tap_emits_basic_action() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"sign-in","content":"Sign In",
                  "events":{ "onTap": [ { "set": { "$state.user.signed_in": "true" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0x42u8; 16]);
        assert_eq!(acts.len(), 1);
        let a = &acts[0];
        assert_eq!(a.source_kind, SourceKind::Tap);
        assert_eq!(a.name.scope.as_str(), "home");
        assert!(a.name.slug.starts_with("sign_in_"));
        assert_eq!(a.name.slug.len(), "sign_in_".len() + 4);
    }

    #[test]
    fn ai_name_drops_hash_suffix() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"x",
                  "semantics":{ "aiName":"sign_in" },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0x42u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].name.slug, "sign_in");
        assert_eq!(acts[0].name.full(), "home.sign_in");
    }

    #[test]
    fn ai_hidden_marks_static_hidden() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"x",
                  "semantics":{ "aiName":"hidden_btn", "aiHidden":true },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].status, AvailabilityStatic::StaticHidden);
    }

    #[test]
    fn destructive_handler_is_confirm_gated() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"delete-btn",
                  "semantics":{ "label":"Delete account" },
                  "events":{ "onTap": [ { "storage_wipe": null } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].status, AvailabilityStatic::ConfirmGated);
    }

    #[test]
    fn dialog_ancestor_picks_modal_scope() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"dlg","semantics":{ "role":"dialog" },
                  "children":[
                    { "type":"frame","id":"close",
                      "semantics":{ "aiName":"close" },
                      "events":{ "onTap": [ { "pop": null } ] }
                    }
                  ]
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].name.scope.as_str(), "modal.dlg");
        assert_eq!(acts[0].name.full(), "modal.dlg.close");
    }

    #[test]
    fn route_emits_open_action() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"list","name":"List","children":[
                { "type":"frame","id":"card",
                  "semantics":{ "aiName":"open_detail" },
                  "route":{ "push": "/detail/:id" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].source_kind, SourceKind::OpenRoute);
        assert_eq!(acts[0].name.slug, "open_open_detail");
    }

    #[test]
    fn bind_value_emits_set_action() {
        // Schema MVP doesn't define a `text_input` variant yet —
        // a `frame` carrying `bindings: bind:value` is the closest
        // valid form that exercises this rule.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"signup","name":"Sign up","children":[
                { "type":"frame","id":"email-input",
                  "semantics":{ "aiName":"email" },
                  "bindings": { "bind:value": "$state.email" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].source_kind, SourceKind::SetValue);
        assert_eq!(acts[0].name.slug, "set_email");
    }

    #[test]
    fn derive_is_deterministic() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a", "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"b", "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let salt = [0xab; 16];
        let a = derive_actions(&doc, &salt);
        let b = derive_actions(&doc, &salt);
        assert_eq!(a, b);
    }

    #[test]
    fn bind_value_skips_non_state_targets() {
        // Bindings that point at $route / $app / a computed expression
        // can't be written through `set_*` — derive should skip them.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"p","name":"P","children":[
                { "type":"frame","id":"a","semantics":{ "aiName":"a" },
                  "bindings": { "bind:value": "$route.params.q" } },
                { "type":"frame","id":"b","semantics":{ "aiName":"b" },
                  "bindings": { "bind:value": "$state.x + 1" } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert!(
            acts.is_empty(),
            "expected zero set_* actions, got: {:#?}",
            acts
        );
    }

    #[test]
    fn bind_value_emits_param_with_inferred_type() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "state":{ "count":{ "type":"int", "default":0 } },
              "pages":[{ "id":"p","name":"P","children":[
                { "type":"frame","id":"input","semantics":{ "aiName":"counter" },
                  "bindings": { "bind:value": "$state.count" } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].source_kind, SourceKind::SetValue);
        assert_eq!(acts[0].params.len(), 1);
        assert_eq!(acts[0].params[0].name, "value");
        assert_eq!(acts[0].params[0].ty, ParamTy::Int);
    }

    #[test]
    fn route_params_inferred_from_routes_config() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "routes":{
                "entry":"/",
                "routes":{
                  "/detail/:id":{ "pageId":"detail", "params":{ "id":"int" } }
                }
              },
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"card","semantics":{ "aiName":"open" },
                  "route":{ "push": "/detail/:id" } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].params.len(), 1);
        assert_eq!(acts[0].params[0].name, "id");
        assert_eq!(acts[0].params[0].ty, ParamTy::Int);
    }

    #[test]
    fn on_reach_end_emits_load_more() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"feed","name":"Feed","children":[
                { "type":"frame","id":"list","semantics":{ "aiName":"posts" },
                  "events":{ "onReachEnd": [ { "fetch": { "url":"/api","method":"GET" } } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].source_kind, SourceKind::LoadMore);
        assert_eq!(acts[0].name.slug, "load_more_posts");
    }

    #[test]
    fn on_scroll_emits_load_more_when_no_reach_end() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"feed","name":"Feed","children":[
                { "type":"frame","id":"list","semantics":{ "aiName":"posts" },
                  "events":{ "onScroll": [ { "set": { "$state.scrolled": "true" } } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].source_kind, SourceKind::LoadMore);
    }

    #[test]
    fn ai_alias_full_name_round_trips() {
        // Spec example: rename `home.sign_in_a3f7` → `home.sign_in`
        // with the old full name kept as an alias. The alias must
        // resolve to the same scope, NOT to `home.home.sign_in_a3f7`.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"x",
                  "semantics":{ "aiName":"sign_in", "aiAliases":["home.sign_in_a3f7","legacy_slug"] },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        let aliases = &acts[0].aliases;
        assert_eq!(aliases.len(), 2);
        // Fully qualified alias keeps its scope.
        assert_eq!(aliases[0].scope.as_str(), "home");
        assert_eq!(aliases[0].slug, "sign_in_a3f7");
        // Bare alias falls back to the owning scope.
        assert_eq!(aliases[1].scope.as_str(), "home");
        assert_eq!(aliases[1].slug, "legacy_slug");
    }

    #[test]
    fn empty_handler_does_not_emit_action() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a", "events":{ "onTap": [] } },
                { "type":"frame","id":"b", "events":{ "onSubmit": [] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert!(
            acts.is_empty(),
            "empty handlers should not produce actions, got {:#?}",
            acts
        );
    }

    #[test]
    fn state_path_handles_array_index() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "state":{ "items":{ "type":{ "array":{ "object":{ "title":"string" } } } } },
              "pages":[{ "id":"p","name":"P","children":[
                { "type":"frame","id":"input","semantics":{ "aiName":"first_title" },
                  "bindings": { "bind:value": "$state.items[0].title" } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].params[0].ty, ParamTy::String);
    }

    #[test]
    fn route_params_default_to_string_when_undeclared() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"card","semantics":{ "aiName":"open" },
                  "route":{ "push": "/detail/:slug" } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].params.len(), 1);
        assert_eq!(acts[0].params[0].ty, ParamTy::String);
    }

    #[test]
    fn collision_emits_warning() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a","semantics":{ "aiName":"save" },
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"b","semantics":{ "aiName":"save" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let (_acts, warnings) = derive_actions_with_warnings(&doc, &[0u8; 16]);
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            DeriveWarning::AiNameCollision {
                full_name,
                source_node_ids,
            } => {
                assert_eq!(full_name, "home.save");
                assert!(source_node_ids.contains(&"a".to_owned()));
                assert!(source_node_ids.contains(&"b".to_owned()));
            }
            other => panic!("expected AiNameCollision, got {:?}", other),
        }
    }

    #[test]
    fn alias_collision_warning_distinguishes_from_ai_name() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"old","semantics":{ "aiName":"new", "aiAliases":["legacy"] },
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"taker","semantics":{ "aiName":"legacy" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let (_acts, warnings) = derive_actions_with_warnings(&doc, &[0u8; 16]);
        assert!(warnings
            .iter()
            .any(|w| matches!(w, DeriveWarning::AliasCollision { .. })));
    }

    #[test]
    fn auto_slug_collision_uses_numeric_suffix() {
        // Force a 16-bit hash collision by making both auto-slugs
        // identical in salt + id. Realistic: same-label buttons in
        // an unsemantic scaffold doc.
        // Instead of finding a real hash collision, we exploit the
        // fact that slug derivation falls back to the node id when
        // there's no aiName/label/text — two nodes with the same id
        // would produce the same auto slug pre-hash. (Spec normally
        // expects unique ids, but a robust derivation handles dupes
        // anyway.) We can't actually have duplicate ids in the same
        // schema, so simulate via flag-time direct manipulation:
        //
        // Easiest deterministic case: two nodes whose ids differ but
        // happen to hash to the same hash4 under salt 0. Brute-force
        // fixture would be flaky; instead we test the
        // disambiguation function directly by constructing two
        // ActionDefinitions with the same full name + has_explicit_name=false.
        use crate::action_surface::{ActionName, AvailabilityStatic, Scope, SourceKind};
        let mut acts = vec![
            ActionDefinition {
                name: ActionName {
                    scope: Scope::page("home"),
                    slug: "click_dead".into(),
                },
                source_node_id: "n1".into(),
                source_kind: SourceKind::Tap,
                description: "".into(),
                status: AvailabilityStatic::Available,
                aliases: vec![],
                params: vec![],
                has_explicit_name: false,
            },
            ActionDefinition {
                name: ActionName {
                    scope: Scope::page("home"),
                    slug: "click_dead".into(),
                },
                source_node_id: "n2".into(),
                source_kind: SourceKind::Tap,
                description: "".into(),
                status: AvailabilityStatic::Available,
                aliases: vec![],
                params: vec![],
                has_explicit_name: false,
            },
        ];
        let warnings = disambiguate_auto_slugs(&mut acts);
        assert_eq!(warnings.len(), 1);
        // First keeps the bare slug; second gets _1.
        assert_eq!(acts[0].name.slug, "click_dead");
        assert_eq!(acts[1].name.slug, "click_dead_1");
        // Both stay Available — auto-slug collision is non-blocking.
        assert_eq!(acts[0].status, AvailabilityStatic::Available);
        assert_eq!(acts[1].status, AvailabilityStatic::Available);
    }

    #[test]
    fn no_warnings_for_clean_doc() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a","semantics":{ "aiName":"save" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let (_acts, warnings) = derive_actions_with_warnings(&doc, &[0u8; 16]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn ai_name_collision_in_same_scope_static_hides_both() {
        // Spec §3.4: two `aiName: "save"` in the same scope can't be
        // disambiguated, so both flip to StaticHidden.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a","semantics":{ "aiName":"save" },
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"b","semantics":{ "aiName":"save" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 2);
        assert_eq!(acts[0].status, AvailabilityStatic::StaticHidden);
        assert_eq!(acts[1].status, AvailabilityStatic::StaticHidden);
    }

    #[test]
    fn ai_name_same_value_different_scope_does_not_collide() {
        // `home.save` vs `settings.save` are distinct full names.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[
                { "id":"home","name":"Home","children":[
                  { "type":"frame","id":"a","semantics":{ "aiName":"save" },
                    "events":{ "onTap": [ { "pop": null } ] } }
                ]},
                { "id":"settings","name":"Settings","children":[
                  { "type":"frame","id":"b","semantics":{ "aiName":"save" },
                    "events":{ "onTap": [ { "pop": null } ] } }
                ]}
              ],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        for a in &acts {
            assert_eq!(a.status, AvailabilityStatic::Available);
        }
    }

    #[test]
    fn alias_equal_to_own_primary_does_not_self_collide() {
        // Author keeps the canonical name as an alias too (no-op
        // migration): `home.save` with `aiAliases: ["save"]` still
        // resolves to one action, no ambiguity.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a",
                  "semantics":{ "aiName":"save", "aiAliases":["save","home.save"] },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].status, AvailabilityStatic::Available);
    }

    #[test]
    fn duplicate_alias_within_same_action_does_not_self_collide() {
        // Author lists the same alias twice (typo / copy-paste).
        // Should still resolve to one action.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a",
                  "semantics":{ "aiName":"save", "aiAliases":["legacy","legacy"] },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].status, AvailabilityStatic::Available);
    }

    #[test]
    fn alias_colliding_with_canonical_name_hides_both() {
        // Renamed-with-alias scenario gone wrong: a fresh button
        // claims the same `aiName` an old button still keeps as a
        // legacy alias. Both must flip to StaticHidden.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"old","semantics":{ "aiName":"new_name", "aiAliases":["legacy"] },
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"new","semantics":{ "aiName":"legacy" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts.len(), 2);
        for a in &acts {
            assert_eq!(a.status, AvailabilityStatic::StaticHidden);
        }
    }

    #[test]
    fn salt_changes_hash_but_preserves_ai_name() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"auto",
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"named",
                  "semantics":{ "aiName":"keep_me" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let s1 = [1u8; 16];
        let s2 = [2u8; 16];
        let a = derive_actions(&doc, &s1);
        let b = derive_actions(&doc, &s2);
        assert_ne!(a[0].name.slug, b[0].name.slug);
        assert_eq!(a[1].name.slug, b[1].name.slug);
        assert_eq!(a[1].name.slug, "keep_me");
    }
}
