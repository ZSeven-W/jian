//! Document-level deduplicated image table for the serialized `.op`
//! form.
//!
//! In memory, image payloads live inline as `data:` URLs on every
//! fill / image node that uses them — cheap, because `ImageSrc` is an
//! `Arc<str>` and shared references are refcount bumps. Serializing
//! that form expands every reference to the full base64 payload: an
//! image-heavy Figma import (hundreds of bitmaps, each referenced by
//! several fills) measured 5.9× larger on disk than the unique bytes.
//!
//! These helpers convert between the two forms at the JSON `Value`
//! boundary, so the typed `PenDocument` (and every in-memory
//! consumer: renderer, export, MCP, live-sync) only ever sees the
//! inline form:
//!
//! - [`externalize_images`] — save side. Moves each large `data:` URL
//!   into a top-level `"images"` table keyed by content hash and
//!   replaces every occurrence with `"op-image:<id>"`; independently
//!   snapshots referenced paint-id thumbnails into `"imageThumbs"`.
//! - [`inline_images`] — Value-level load side. Resolves `op-image:`
//!   refs back to the full data URLs, seeds blur-up thumbnails, and removes
//!   both tables, restoring the exact pre-save strings (the round trip is
//!   lossless). The
//!   PRODUCT load paths use [`take_image_table`] +
//!   [`crate::node::image_src::intern::with_load_scope`] instead:
//!   resolving during the typed parse hands every reference a clone of
//!   ONE shared `Arc` per unique payload, where a Value-level inline
//!   would clone the payload once per reference and re-inflate the
//!   document.
//!
//! The traversal is STRUCTURAL (see `visit_image_strings_mut`): it
//! follows the node tree (`children` / `pages[].children`) and visits
//! only the schema positions typed as `ImageSrc` — `ImageNode.src`
//! and image-fill `url`s in `fill` / `stroke.fill` / `states.*`
//! overrides. Free-form JSON (`state` defaults, `app`, `routes`,
//! variables, action params) is never entered, so arbitrary user data
//! that merely LOOKS like an image object — or text that starts with
//! `data:` / `op-image:` — can never be rewritten.

use serde_json::{Map, Value};

/// Key of the top-level table in the serialized document.
const IMAGES_KEY: &str = "images";
/// Prefix marking an externalized reference.
const REF_PREFIX: &str = "op-image:";
/// Only externalize `data:` URLs at least this long. Short payloads
/// (small icons) gain little from dedup, and keeping them inline
/// preserves the historical file shape for simple documents.
const MIN_EXTERNALIZE_LEN: usize = 4096;

/// Whether a node-tree object is an image node / image fill.
fn is_image_typed(map: &Map<String, Value>) -> bool {
    matches!(map.get("type"), Some(Value::String(t)) if t == "image")
}

/// Public alias of the structural visitor for external tools (e.g.
/// the `.op` three-way merge rewriting refs after an id remap).
pub fn visit_image_src_strings_mut(root: &mut Value, f: &mut impl FnMut(&mut String)) {
    visit_image_strings_mut(root, f);
}

/// Visit every image-source string slot in `root`'s NODE TREE:
/// `ImageNode.src` and image-fill `url`s — direct `fill` arrays,
/// `stroke.fill`, and widget-state override (`states.*`) fills —
/// across `children` / `pages[].children` and nested `children`.
///
/// The traversal is STRUCTURAL, not shape-matching: it never descends
/// into free-form JSON fields (`state` defaults, `app`, `routes`,
/// `variables`, `designMd`, action params), so arbitrary user data
/// that merely LOOKS like an image object is out of scope by
/// construction. This must stay in lockstep with where the schema
/// types `ImageSrc` — the typed loader
/// (`ImageSrc::deserialize` + `image_src::intern`) resolves refs at
/// exactly these positions, and a slot visited here but typed as
/// plain JSON on load would produce a dangling ref that loses the
/// payload on the next save. Iterative (explicit stack) — documents
/// deep enough to defeat serde's recursion limit go through this too.
fn visit_image_strings_mut(root: &mut Value, f: &mut impl FnMut(&mut String)) {
    let Value::Object(map) = root else {
        return;
    };
    let mut stack: Vec<&mut Value> = Vec::new();
    for (k, v) in map.iter_mut() {
        if k == "children" || k == "pages" {
            if let Value::Array(items) = v {
                stack.extend(items.iter_mut());
            }
        }
    }
    while let Some(node) = stack.pop() {
        let Value::Object(node) = node else {
            continue;
        };
        let image_node = is_image_typed(node);
        for (k, v) in node.iter_mut() {
            match (k.as_str(), v) {
                ("src", Value::String(s)) if image_node => f(s),
                ("fill", fill) => visit_fill_array_mut(fill, f),
                ("stroke", stroke) => visit_stroke_mut(stroke, f),
                ("states", Value::Object(states)) => {
                    for override_value in states.values_mut() {
                        let Value::Object(state_override) = override_value else {
                            continue;
                        };
                        for (ok, ov) in state_override.iter_mut() {
                            match ok.as_str() {
                                "fill" => visit_fill_array_mut(ov, f),
                                "stroke" => visit_stroke_mut(ov, f),
                                _ => {}
                            }
                        }
                    }
                }
                ("children", Value::Array(items)) => stack.extend(items.iter_mut()),
                _ => {}
            }
        }
    }
}

/// Visit the `url` of every image fill in a `fill` array.
fn visit_fill_array_mut(fill: &mut Value, f: &mut impl FnMut(&mut String)) {
    let Value::Array(items) = fill else {
        return;
    };
    for item in items {
        let Value::Object(item) = item else {
            continue;
        };
        if !is_image_typed(item) {
            continue;
        }
        if let Some(Value::String(url)) = item.get_mut("url") {
            f(url);
        }
    }
}

/// Visit the image-fill urls of a `stroke` object's `fill` array.
fn visit_stroke_mut(stroke: &mut Value, f: &mut impl FnMut(&mut String)) {
    let Value::Object(stroke) = stroke else {
        return;
    };
    if let Some(fill) = stroke.get_mut("fill") {
        visit_fill_array_mut(fill, f);
    }
}

/// Save-side rewrite: move every sufficiently large `data:` URL
/// string in `root` into the top-level `"images"` table and replace
/// it with an `op-image:<id>` reference. Ids are FNV-1a content
/// hashes, so identical payloads collapse to one table entry no
/// matter how many nodes reference them, and re-saving an unchanged
/// document produces identical ids. No-op on non-object roots; the
/// `images` is only created when at least one URL externalizes;
/// `imageThumbs` is independently created only for referenced paint ids that
/// are present in the runtime thumbnail registry.
pub fn externalize_images(root: &mut Value) {
    // Preserve (and extend) an existing table so a caller that
    // externalizes twice stays idempotent.
    let mut table = {
        let Value::Object(map) = &mut *root else {
            return;
        };
        match map.remove(IMAGES_KEY) {
            Some(Value::Object(existing)) => existing,
            Some(other) => {
                // Malformed table — put it back untouched and bail
                // rather than destroy data we don't understand.
                map.insert(IMAGES_KEY.to_owned(), other);
                return;
            }
            None => Map::new(),
        }
    };
    // Collect persisted thumbnail keys while sources still name their final
    // payloads. On an idempotent second pass, resolve existing `op-image:`
    // refs through the preserved image table before hashing; hashing the ref
    // string itself would silently switch identities and drop the thumb.
    let referenced_paint_ids = referenced_paint_ids_for_save(root, &table);
    let thumb_table = crate::image_thumbs::snapshot_for(&referenced_paint_ids);
    visit_image_strings_mut(root, &mut |s| {
        if s.len() >= MIN_EXTERNALIZE_LEN && s.starts_with("data:") {
            let id = intern(&mut table, s);
            *s = format!("{REF_PREFIX}{id}");
        }
    });
    let Value::Object(map) = root else {
        return;
    };
    if !table.is_empty() {
        map.insert(IMAGES_KEY.to_owned(), Value::Object(table));
    }
    if thumb_table.is_empty() {
        map.remove(crate::image_thumbs::IMAGE_THUMBS_KEY);
    } else {
        map.insert(
            crate::image_thumbs::IMAGE_THUMBS_KEY.to_owned(),
            Value::Object(thumb_table),
        );
    }
}

/// Paint ids referenced by the node tree before save-side source rewriting.
fn referenced_paint_ids_for_save(
    root: &Value,
    image_table: &Map<String, Value>,
) -> std::collections::BTreeSet<u64> {
    let mut ids = std::collections::BTreeSet::new();
    visit_image_strings_ref(root, &mut |source| {
        let final_source = if let Some(table_id) = source.strip_prefix(REF_PREFIX) {
            let Some(Value::String(payload)) = image_table.get(table_id) else {
                return;
            };
            payload.as_str()
        } else {
            source
        };
        ids.insert(crate::node::image_src::paint_image_id(final_source));
    });
    ids
}

/// Insert `payload` into the table (if absent) and return its id.
/// Hash collisions with *different* content probe `-2`, `-3`, … so
/// distinct payloads never share an id.
fn intern(table: &mut Map<String, Value>, payload: &str) -> String {
    let base = format!("{:016x}", fnv1a64(payload.as_bytes()));
    let mut id = base.clone();
    let mut probe = 1u32;
    loop {
        match table.get(&id) {
            None => {
                table.insert(id.clone(), Value::String(payload.to_owned()));
                return id;
            }
            Some(Value::String(existing)) if existing == payload => return id,
            _ => {
                probe += 1;
                id = format!("{base}-{probe}");
            }
        }
    }
}

/// Load-side rewrite: resolve every `op-image:<id>` string back to
/// the full data URL from the top-level `"images"` table, then drop
/// the table. A ref whose id is missing from the table (hand-edited
/// or badly merged file) is left as-is — it renders as a placeholder
/// instead of failing the load. No-op when the document carries no
/// table. The independent `imageThumbs` table is always consumed first; an
/// absent thumbnail table leaves the active content cache unchanged.
pub fn inline_images(root: &mut Value) {
    crate::image_thumbs::seed_from_document(root);
    let table = {
        let Value::Object(map) = &mut *root else {
            return;
        };
        let Some(Value::Object(table)) = map.remove(IMAGES_KEY) else {
            return;
        };
        table
    };
    visit_image_strings_mut(root, &mut |s| {
        if let Some(Value::String(payload)) =
            s.strip_prefix(REF_PREFIX).and_then(|id| table.get(id))
        {
            *s = payload.clone();
        }
    });
}

/// Collect every image-table id referenced from the node tree of
/// `root` (same positions as `visit_image_strings_mut`). Used by
/// the structured merge to prune unreferenced table entries.
pub fn referenced_ids(root: &Value) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    visit_image_strings_ref(root, &mut |s| {
        if let Some(id) = s.strip_prefix(REF_PREFIX) {
            out.insert(id.to_owned());
        }
    });
    out
}

/// Immutable twin of [`visit_image_strings_mut`] — keep the two
/// traversals structurally identical.
fn visit_image_strings_ref(root: &Value, f: &mut impl FnMut(&str)) {
    let Value::Object(map) = root else {
        return;
    };
    let mut stack: Vec<&Value> = Vec::new();
    for (k, v) in map {
        if k == "children" || k == "pages" {
            if let Value::Array(items) = v {
                stack.extend(items.iter());
            }
        }
    }
    while let Some(node) = stack.pop() {
        let Value::Object(node) = node else {
            continue;
        };
        let image_node = is_image_typed(node);
        for (k, v) in node {
            match (k.as_str(), v) {
                ("src", Value::String(s)) if image_node => f(s),
                ("fill", fill) => visit_fill_array_ref(fill, f),
                ("stroke", stroke) => visit_stroke_ref(stroke, f),
                ("states", Value::Object(states)) => {
                    for override_value in states.values() {
                        let Value::Object(state_override) = override_value else {
                            continue;
                        };
                        for (ok, ov) in state_override {
                            match ok.as_str() {
                                "fill" => visit_fill_array_ref(ov, f),
                                "stroke" => visit_stroke_ref(ov, f),
                                _ => {}
                            }
                        }
                    }
                }
                ("children", Value::Array(items)) => stack.extend(items.iter()),
                _ => {}
            }
        }
    }
}

fn visit_fill_array_ref(fill: &Value, f: &mut impl FnMut(&str)) {
    let Value::Array(items) = fill else {
        return;
    };
    for item in items {
        let Value::Object(item) = item else {
            continue;
        };
        if !is_image_typed(item) {
            continue;
        }
        if let Some(Value::String(url)) = item.get("url") {
            f(url);
        }
    }
}

fn visit_stroke_ref(stroke: &Value, f: &mut impl FnMut(&str)) {
    let Value::Object(stroke) = stroke else {
        return;
    };
    if let Some(fill) = stroke.get("fill") {
        visit_fill_array_ref(fill, f);
    }
}

/// Whether `root` carries an externalized image table (used by the
/// compat loader to pick the resolution path).
pub fn has_image_table(root: &Value) -> bool {
    matches!(root.get(IMAGES_KEY), Some(Value::Object(_)))
}

/// Remove the top-level `"images"` table from `root` and return it as
/// an id → shared-payload map for
/// [`crate::node::image_src::intern::with_load_scope`]. Each payload
/// becomes exactly ONE `Arc<str>`; every `ImageSrc` that references
/// it during the typed parse receives a clone of that `Arc`, so a
/// thousand fills sharing an image cost one allocation — the Value
/// tree is never inflated with per-reference payload copies. Non-string
/// table values are dropped (a ref to them stays unresolved and paints
/// as a placeholder). Returns an empty map when no table is present.
pub fn take_image_table(
    root: &mut Value,
) -> std::collections::HashMap<String, std::sync::Arc<str>> {
    let mut out = std::collections::HashMap::new();
    let Value::Object(map) = root else {
        return out;
    };
    let Some(Value::Object(table)) = map.remove(IMAGES_KEY) else {
        return out;
    };
    for (id, payload) in table {
        if let Value::String(s) = payload {
            out.insert(id, std::sync::Arc::from(s));
        }
    }
    out
}

/// FNV-1a 64-bit — tiny, dependency-free, stable across builds (the
/// ids are part of the file format, so `std`'s randomized `SipHash`
/// is not an option).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A `data:` URL long enough to externalize, with a content twist
    /// so distinct payloads hash differently.
    fn big_data_url(tag: &str) -> String {
        format!(
            "data:image/png;base64,{tag}{}",
            "A".repeat(MIN_EXTERNALIZE_LEN)
        )
    }

    #[test]
    fn shared_payload_externalizes_to_one_table_entry() {
        let url = big_data_url("x");
        let mut doc = json!({
            "version": "0.8.0",
            "children": [
                {"type": "rectangle", "fill": [{"type": "image", "url": url}]},
                {"type": "rectangle", "fill": [{"type": "image", "url": url}]},
                {"type": "image", "src": url},
            ],
        });
        externalize_images(&mut doc);

        let table = doc.get("images").and_then(Value::as_object).expect("table");
        assert_eq!(table.len(), 1, "three refs share one entry");
        let (id, stored) = table.iter().next().expect("entry");
        assert_eq!(stored.as_str(), Some(url.as_str()));
        let serialized = doc.to_string();
        assert_eq!(
            serialized.matches(&format!("op-image:{id}")).count(),
            3,
            "every occurrence became the same ref"
        );
        assert_eq!(
            serialized.matches("data:image/png").count(),
            1,
            "payload appears exactly once (in the table)"
        );
    }

    #[test]
    fn round_trip_restores_the_exact_document() {
        let mut doc = json!({
            "version": "0.8.0",
            "children": [
                {"type": "rectangle", "fill": [{"type": "image", "url": big_data_url("x")}]},
                {"type": "frame", "children": [
                    {"type": "image", "src": big_data_url("y")},
                ]},
            ],
        });
        let original = doc.clone();
        externalize_images(&mut doc);
        assert!(has_image_table(&doc));
        inline_images(&mut doc);
        assert_eq!(doc, original, "externalize → inline is lossless");
    }

    #[test]
    fn small_and_non_data_strings_stay_inline() {
        let mut doc = json!({
            "children": [
                {"type": "image", "src": "data:image/png;base64,tiny"},
                {"type": "image", "src": "https://example.com/cat.png"},
                {"type": "text", "content": "not an image"},
            ],
        });
        let original = doc.clone();
        externalize_images(&mut doc);
        assert_eq!(doc, original, "nothing crossed the externalize bar");
        assert!(!has_image_table(&doc));
    }

    #[test]
    fn distinct_payloads_get_distinct_ids() {
        let mut doc = json!({
            "children": [
                {"type": "image", "src": big_data_url("a")},
                {"type": "image", "src": big_data_url("b")},
            ],
        });
        externalize_images(&mut doc);
        let table = doc.get("images").and_then(Value::as_object).expect("table");
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn hash_collision_probes_a_new_id() {
        // Force a collision by pre-seeding the table with a different
        // payload under the id the new payload hashes to.
        let url = big_data_url("x");
        let id = format!("{:016x}", fnv1a64(url.as_bytes()));
        let mut doc = json!({
            "images": {&id: "data:image/png;base64,other"},
            "children": [{"type": "image", "src": url}],
        });
        externalize_images(&mut doc);
        let table = doc.get("images").and_then(Value::as_object).expect("table");
        assert_eq!(table.len(), 2, "collision kept both payloads");
        assert!(table.contains_key(&format!("{id}-2")), "probed id");
    }

    #[test]
    fn unresolvable_ref_is_left_untouched() {
        let mut doc = json!({
            "images": {},
            "children": [{"type": "image", "src": "op-image:deadbeef"}],
        });
        inline_images(&mut doc);
        assert_eq!(
            doc["children"][0]["src"].as_str(),
            Some("op-image:deadbeef"),
            "missing table entry leaves the ref (placeholder at paint)"
        );
        assert!(doc.get("images").is_none(), "table key is dropped");
    }

    /// Authored TEXT content must never be rewritten, even when it
    /// looks exactly like a data URL or an existing table ref — only
    /// image-source fields (`src` / `url`) participate.
    #[test]
    fn text_content_is_never_rewritten() {
        let url = big_data_url("x");
        let mut doc = json!({
            "children": [
                {"type": "image", "src": url},
                // A text node quoting a data URL as literal content.
                {"type": "text", "content": url},
            ],
        });
        externalize_images(&mut doc);
        let id = {
            let table = doc.get("images").and_then(Value::as_object).expect("table");
            assert_eq!(table.len(), 1);
            table.keys().next().expect("id").clone()
        };
        assert_eq!(
            doc["children"][1]["content"].as_str(),
            Some(url.as_str()),
            "text content stays inline"
        );
        // A text node whose content IS a valid ref string must survive
        // the inline pass untouched (no payload spliced into text).
        doc["children"][1]["content"] = Value::String(format!("op-image:{id}"));
        let quoted = doc["children"][1]["content"].clone();
        inline_images(&mut doc);
        assert_eq!(doc["children"][0]["src"].as_str(), Some(url.as_str()));
        assert_eq!(doc["children"][1]["content"], quoted, "text ref untouched");
    }

    /// Free-form JSON fields (state defaults, app params, …) must be
    /// untouchable even when they contain an object that is
    /// byte-identical to an image node — the typed loader cannot
    /// resolve refs there, so externalizing them would lose the
    /// payload on the next save.
    #[test]
    fn image_shaped_objects_outside_the_node_tree_stay_inline() {
        let url = big_data_url("x");
        let mut doc = json!({
            "children": [
                {"type": "frame", "state": {
                    "avatar": {"type": "object", "default": {"type": "image", "src": url}},
                }},
            ],
            "state": {"bg": {"type": "string", "default": url}},
        });
        let original = doc.clone();
        externalize_images(&mut doc);
        assert_eq!(doc, original, "free-form JSON is never externalized");
        assert!(!has_image_table(&doc));
    }

    /// Widget-state override fills (`states.hover.fill[…]`) are real
    /// `ImageSrc` positions and must participate in the dedup.
    #[test]
    fn widget_state_override_fills_externalize() {
        let url = big_data_url("x");
        let mut doc = json!({
            "children": [
                {"type": "tabs", "states": {
                    "hover": {"fill": [{"type": "image", "url": url}]},
                    "pressed": {"stroke": {"fill": [{"type": "image", "url": url}]}},
                }},
            ],
        });
        externalize_images(&mut doc);
        let table = doc.get("images").and_then(Value::as_object).expect("table");
        assert_eq!(table.len(), 1, "both override refs share one entry");
        let serialized = doc.to_string();
        assert_eq!(serialized.matches("op-image:").count(), 2);
        inline_images(&mut doc);
        assert_eq!(
            doc["children"][0]["states"]["hover"]["fill"][0]["url"].as_str(),
            Some(url.as_str()),
            "inline restores the override fill"
        );
    }

    #[test]
    fn referenced_ids_reports_only_image_src_refs() {
        let doc = json!({
            "children": [
                {"type": "image", "src": "op-image:aa"},
                {"type": "rectangle", "fill": [{"type": "image", "url": "op-image:bb"}]},
                {"type": "text", "content": "op-image:cc"},
            ],
        });
        let ids = referenced_ids(&doc);
        assert!(ids.contains("aa") && ids.contains("bb"));
        assert!(!ids.contains("cc"), "text mention is not a reference");
    }

    #[test]
    fn externalize_is_idempotent() {
        let mut doc = json!({
            "children": [{"type": "image", "src": big_data_url("x")}],
        });
        externalize_images(&mut doc);
        let once = doc.clone();
        externalize_images(&mut doc);
        assert_eq!(doc, once, "second pass changes nothing");
    }

    #[test]
    fn thumbnail_snapshot_is_filtered_and_idempotent_before_rewrite() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::replace_from_load(Default::default());
            let url = big_data_url("thumb");
            let paint_id = crate::node::image_src::paint_image_id(&url);
            let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
            crate::image_thumbs::store_thumb(paint_id, jpeg.clone());
            crate::image_thumbs::store_thumb(paint_id.wrapping_add(1), vec![1, 2, 3]);
            let mut doc = json!({
                "children": [{"type": "image", "src": url}],
            });
            let original = doc.clone();

            externalize_images(&mut doc);
            assert_eq!(
                doc["imageThumbs"],
                json!({paint_id.to_string(): "/9j/2Q=="}),
                "the table uses decimal paint ids and excludes unrelated registry entries"
            );
            let once = doc.clone();
            externalize_images(&mut doc);
            assert_eq!(
                doc, once,
                "a second pass resolves existing refs before collecting paint ids"
            );

            crate::image_thumbs::replace_from_load(Default::default());
            inline_images(&mut doc);
            assert_eq!(doc, original, "both top-level tables are load-only forms");
            assert_eq!(
                &*crate::image_thumbs::thumb_for(paint_id).expect("inline seeded thumbnail"),
                jpeg.as_slice()
            );
        });
    }

    #[test]
    fn no_referenced_thumbnail_keeps_image_thumbs_absent() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::replace_from_load(Default::default());
            let mut doc = json!({
                "children": [{"type": "image", "src": big_data_url("no-thumb")}],
            });
            externalize_images(&mut doc);
            assert!(
                doc.get("imageThumbs").is_none(),
                "an additive table is not emitted when the loaded document had none"
            );
        });
    }

    #[test]
    fn thumbnail_table_does_not_depend_on_image_externalization() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::replace_from_load(Default::default());
            let url = "assets/small-logo.jpg";
            let paint_id = crate::node::image_src::paint_image_id(url);
            crate::image_thumbs::store_thumb(paint_id, vec![0xff, 0xd8, 0xff, 0xd9]);
            let mut doc = json!({
                "children": [{"type": "image", "src": url}],
            });

            externalize_images(&mut doc);
            assert!(doc.get("images").is_none(), "small source stays inline");
            assert_eq!(
                doc["imageThumbs"],
                json!({paint_id.to_string(): "/9j/2Q=="}),
                "the paint id needs no images-table mapping"
            );
        });
    }
}
