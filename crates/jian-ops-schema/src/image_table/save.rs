//! Streaming save-side image externalization.
//!
//! The historical save path first materialized the complete document as a
//! `serde_json::Value`, rewrote every image string, then materialized the
//! complete pretty JSON string. On image-heavy files those two owned trees
//! temporarily cost more memory than the live document. This scope lets the
//! ordinary typed serializer write straight to its destination: each
//! `ImageSrc` emits a small `op-image:` reference while this module records one
//! shared `Arc` per payload for the top-level table appended by the caller.

use super::{fnv1a64, MIN_EXTERNALIZE_LEN, REF_PREFIX};
use crate::image_thumbs::ImageThumbSnapshot;
use crate::node::ImageSrc;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

mod order;
pub use order::{visit_legacy_node_roots, SaveImageOrder};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct AllocationKey {
    address: usize,
    len: usize,
}

#[derive(Default)]
struct SaveCollector {
    /// Stable id + shared payload in first-reference order. The old
    /// `Value`-rewrite path used serde_json's preserve-order map and therefore
    /// emitted this same traversal order; keep it stable for small Git diffs.
    images: Vec<(String, Arc<str>)>,
    image_indices: HashMap<String, usize>,
    /// Fast path for the common case where document clones share ImageSrc Arcs.
    allocations: HashMap<AllocationKey, String>,
    referenced_paint_ids: BTreeSet<u64>,
}

impl SaveCollector {
    fn record_source(&mut self, source: &ImageSrc) -> Option<String> {
        let source_text = source.as_str();
        if source_text.starts_with(REF_PREFIX) {
            return None;
        }
        self.referenced_paint_ids
            .insert(crate::node::image_src::paint_image_id(source_text));
        if source_text.len() < MIN_EXTERNALIZE_LEN || !source_text.starts_with("data:") {
            return None;
        }

        let payload = source.as_arc();
        let allocation = AllocationKey {
            address: Arc::as_ptr(&payload) as *const () as usize,
            len: payload.len(),
        };
        if let Some(id) = self.allocations.get(&allocation) {
            return Some(format!("{REF_PREFIX}{id}"));
        }

        let base = format!("{:016x}", fnv1a64(source_text.as_bytes()));
        let mut id = base.clone();
        let mut probe = 1_u32;
        loop {
            match self.image_indices.get(&id).copied() {
                None => {
                    let index = self.images.len();
                    self.images.push((id.clone(), payload));
                    self.image_indices.insert(id.clone(), index);
                    break;
                }
                Some(index) if self.images[index].1.as_ref() == source_text => break,
                Some(_) => {
                    probe += 1;
                    id = format!("{base}-{probe}");
                }
            }
        }
        self.allocations.insert(allocation, id.clone());
        Some(format!("{REF_PREFIX}{id}"))
    }
}

thread_local! {
    static ACTIVE_COLLECTOR: RefCell<Option<Rc<RefCell<SaveCollector>>>> = const {
        RefCell::new(None)
    };
}

/// Tables collected while the document serializes inside [`with_save_scope`].
pub struct SaveTables<'a> {
    collector: Rc<RefCell<SaveCollector>>,
    thumbnails: &'a ImageThumbSnapshot,
}

impl SaveTables<'_> {
    /// True when no large data URL needed externalization.
    pub fn images_is_empty(&self) -> bool {
        self.collector.borrow().images.is_empty()
    }

    /// Borrow the collected image table as a streaming serde value.
    pub fn images(&self) -> SaveImageTable<'_> {
        SaveImageTable {
            collector: &self.collector,
        }
    }

    /// Encode only thumbnails whose image sources occurred in this document.
    pub fn image_thumbs(&self) -> Map<String, Value> {
        let collector = self.collector.borrow();
        self.thumbnails
            .serialized_for(&collector.referenced_paint_ids)
    }
}

/// Streaming serde view over the deduplicated image table.
pub struct SaveImageTable<'a> {
    collector: &'a Rc<RefCell<SaveCollector>>,
}

/// Which optional tables a streaming document write emitted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SaveTableWriteStats {
    pub wrote_images_table: bool,
    pub wrote_image_thumbs: bool,
}

impl Serialize for SaveImageTable<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let collector = self.collector.borrow();
        let mut map = serializer.serialize_map(Some(collector.images.len()))?;
        for (id, payload) in &collector.images {
            map.serialize_entry(id, payload.as_ref())?;
        }
        map.end()
    }
}

/// Run `f` with streaming image externalization enabled on this thread.
///
/// Scopes nest safely and restore the previous collector on both normal return
/// and unwind. The caller must serialize the document before reading `tables`.
pub fn with_save_scope<R>(
    thumbnails: &ImageThumbSnapshot,
    f: impl FnOnce(&SaveTables<'_>) -> R,
) -> R {
    with_collector(SaveCollector::default(), thumbnails, f)
}

fn with_collector<R>(
    collector: SaveCollector,
    thumbnails: &ImageThumbSnapshot,
    f: impl FnOnce(&SaveTables<'_>) -> R,
) -> R {
    let collector = Rc::new(RefCell::new(collector));
    let previous = ACTIVE_COLLECTOR.with(|active| active.borrow_mut().replace(collector.clone()));

    struct Restore(Option<Rc<RefCell<SaveCollector>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_COLLECTOR.with(|active| *active.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(previous);
    f(&SaveTables {
        collector,
        thumbnails,
    })
}

/// Stream a typed document, its deduplicated image tables, and one additional
/// top-level extension without allocating a document-sized JSON tree/string.
///
/// The extension is written last. OpenPencil uses this for `editorMeta`; other
/// Jian hosts can use any schema-compatible additive top-level field.
/// Typed fields keep serde declaration order, while image entries keep their
/// first-reference order. That matches the former preserve-order `Value` path
/// without retaining its document-sized allocation.
pub fn write_document_with_extension<
    W: std::io::Write,
    D: Serialize + SaveImageOrder + ?Sized,
    T: Serialize,
>(
    writer: &mut W,
    document: &D,
    thumbnails: &ImageThumbSnapshot,
    extension_name: &str,
    extension: &T,
) -> serde_json::Result<SaveTableWriteStats> {
    let mut collector = SaveCollector::default();
    order::prepare(document, &mut collector);
    with_collector(collector, thumbnails, |tables| {
        {
            let mut root = RootTailWriter::new(writer);
            serde_json::to_writer_pretty(&mut root, document)?;
            root.remove_closing_brace()?;
        }

        let wrote_images_table = !tables.images_is_empty();
        if wrote_images_table {
            write_field_prefix(writer, "images")?;
            write_nested_pretty(writer, &tables.images())?;
        }

        let image_thumbs = tables.image_thumbs();
        let wrote_image_thumbs = !image_thumbs.is_empty();
        if wrote_image_thumbs {
            write_field_prefix(writer, "imageThumbs")?;
            write_nested_pretty(writer, &image_thumbs)?;
        }

        write_field_prefix(writer, extension_name)?;
        write_nested_pretty(writer, extension)?;
        writer.write_all(b"\n}").map_err(serde_json::Error::io)?;

        Ok(SaveTableWriteStats {
            wrote_images_table,
            wrote_image_thumbs,
        })
    })
}

fn write_nested_pretty(
    writer: &mut impl std::io::Write,
    value: &impl Serialize,
) -> serde_json::Result<()> {
    let mut indented = AdditionalIndentWriter::new(writer);
    serde_json::to_writer_pretty(&mut indented, value)
}

/// Add one outer indentation level to an independently pretty-serialized value.
struct AdditionalIndentWriter<'a, W> {
    inner: &'a mut W,
    line_start: bool,
}

impl<'a, W> AdditionalIndentWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            line_start: false,
        }
    }
}

impl<W: std::io::Write> std::io::Write for AdditionalIndentWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut offset = 0;
        while offset < bytes.len() {
            if self.line_start {
                self.inner.write_all(b"  ")?;
                self.line_start = false;
            }
            let remaining = &bytes[offset..];
            match remaining.iter().position(|byte| *byte == b'\n') {
                Some(relative_newline) => {
                    let end = offset + relative_newline + 1;
                    self.inner.write_all(&bytes[offset..end])?;
                    self.line_start = true;
                    offset = end;
                }
                None => {
                    self.inner.write_all(remaining)?;
                    break;
                }
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn write_field_prefix(writer: &mut impl std::io::Write, key: &str) -> serde_json::Result<()> {
    writer.write_all(b",\n  ").map_err(serde_json::Error::io)?;
    serde_json::to_writer(&mut *writer, key)?;
    writer.write_all(b": ").map_err(serde_json::Error::io)
}

/// Keep the final two bytes so the pretty serializer's root `\n}` can be
/// removed before top-level extensions are appended.
struct RootTailWriter<'a, W> {
    inner: &'a mut W,
    tail: [u8; 2],
    tail_len: usize,
}

impl<'a, W> RootTailWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            tail: [0; 2],
            tail_len: 0,
        }
    }
}

impl<W: std::io::Write> RootTailWriter<'_, W> {
    fn remove_closing_brace(self) -> serde_json::Result<()> {
        if self.tail_len == 0 || self.tail[self.tail_len - 1] != b'}' {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "canonical document did not end in a root object brace",
            )));
        }
        let prefix = &self.tail[..self.tail_len - 1];
        if !prefix.iter().all(u8::is_ascii_whitespace) {
            self.inner
                .write_all(prefix)
                .map_err(serde_json::Error::io)?;
        }
        Ok(())
    }
}

impl<W: std::io::Write> std::io::Write for RootTailWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match bytes.len() {
            0 => return Ok(0),
            1 if self.tail_len < 2 => {
                self.tail[self.tail_len] = bytes[0];
                self.tail_len += 1;
                return Ok(1);
            }
            1 => {
                self.inner.write_all(&self.tail[..1])?;
                self.tail[0] = self.tail[1];
                self.tail[1] = bytes[0];
                return Ok(1);
            }
            _ => {}
        }

        if self.tail_len > 0 {
            self.inner.write_all(&self.tail[..self.tail_len])?;
        }
        if bytes.len() > 2 {
            self.inner.write_all(&bytes[..bytes.len() - 2])?;
        }
        self.tail.copy_from_slice(&bytes[bytes.len() - 2..]);
        self.tail_len = 2;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Return the wire reference for `source` inside a save scope, recording its
/// payload and paint id as a side effect. Outside a scope, return `None` so the
/// historical transparent `ImageSrc` serialization remains unchanged.
pub(crate) fn scoped_reference(source: &ImageSrc) -> Option<String> {
    ACTIVE_COLLECTOR.with(|active| {
        let active = active.borrow();
        let collector = active.as_ref()?;
        let reference = collector.borrow_mut().record_source(source);
        reference
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_thumbs;
    use crate::node::{ImageNode, PenNode, PenNodeBase};
    use crate::PenDocument;

    fn large_source(tag: &str) -> ImageSrc {
        ImageSrc::from(format!(
            "data:image/png;base64,{tag}{}",
            "A".repeat(MIN_EXTERNALIZE_LEN)
        ))
    }

    fn image_node(id: &str, source: ImageSrc) -> PenNode {
        PenNode::Image(ImageNode {
            base: PenNodeBase {
                id: id.to_owned(),
                ..Default::default()
            },
            src: source,
            object_fit: None,
            width: None,
            height: None,
            limits: Default::default(),
            corner_radius: None,
            effects: None,
            exposure: None,
            contrast: None,
            saturation: None,
            temperature: None,
            tint: None,
            highlights: None,
            shadows: None,
            image_prompt: None,
            image_search_query: None,
            video: None,
            state: None,
            bindings: None,
            events: None,
            lifecycle: None,
            semantics: None,
            gestures: None,
            route: None,
        })
    }

    #[test]
    fn scope_externalizes_shared_sources_once_and_default_stays_inline() {
        let source = large_source("shared");
        let doc = PenDocument {
            version: "1.0.0".into(),
            name: None,
            themes: None,
            variables: None,
            pages: None,
            children: vec![
                image_node("one", source.clone()),
                image_node("two", source.clone()),
            ],
            format_version: None,
            responsive: None,
            id: None,
            app: None,
            routes: None,
            state: None,
            lifecycle: None,
            logic_modules: None,
            design_md: None,
            conversion: None,
        };

        let ordinary = serde_json::to_string(&doc).expect("ordinary serialize");
        assert_eq!(ordinary.matches("data:image/png").count(), 2);

        let thumbnails = image_thumbs::capture_snapshot();
        with_save_scope(&thumbnails, |tables| {
            let scoped = serde_json::to_string(&doc).expect("scoped serialize");
            assert_eq!(scoped.matches("op-image:").count(), 2);
            assert_eq!(scoped.matches("data:image/png").count(), 0);
            let table = serde_json::to_value(tables.images()).expect("image table");
            assert_eq!(table.as_object().map(|map| map.len()), Some(1));
            assert_eq!(table.to_string().matches("data:image/png").count(), 1);
        });

        let restored = serde_json::to_string(&doc).expect("serialize after scope");
        assert_eq!(restored, ordinary, "scope is always restored");
    }

    #[test]
    fn unwind_restores_the_previous_image_serializer() {
        let result = std::panic::catch_unwind(|| {
            let thumbnails = image_thumbs::capture_snapshot();
            with_save_scope(&thumbnails, |_| panic!("intentional"));
        });
        assert!(result.is_err());
        let source = large_source("after-panic");
        let json = serde_json::to_string(&source).expect("ordinary ImageSrc");
        assert!(json.contains("data:image/png"));
        assert!(!json.contains("op-image:"));
    }

    #[test]
    fn thumbnail_snapshot_is_fixed_before_the_save_scope_runs() {
        image_thumbs::with_test_serialized(|| {
            image_thumbs::clear_registry();
            let source = large_source("thumb");
            let paint_id = crate::node::image_src::paint_image_id(source.as_str());
            image_thumbs::store_thumb(paint_id, vec![0xff, 0xd8, 0xff, 0xd9]);
            let snapshot = image_thumbs::capture_snapshot();
            image_thumbs::clear_registry();

            with_save_scope(&snapshot, |tables| {
                serde_json::to_writer(std::io::sink(), &source).expect("collect source");
                assert_eq!(
                    Value::Object(tables.image_thumbs()),
                    serde_json::json!({paint_id.to_string(): "/9j/2Q=="})
                );
            });
        });
    }

    #[test]
    fn document_writer_appends_the_captured_thumbnail_table() {
        image_thumbs::with_test_serialized(|| {
            image_thumbs::clear_registry();
            let source = large_source("writer-thumb");
            let paint_id = crate::node::image_src::paint_image_id(source.as_str());
            image_thumbs::store_thumb(paint_id, vec![0xff, 0xd8, 0xff, 0xd9]);
            let snapshot = image_thumbs::capture_snapshot();
            image_thumbs::clear_registry();
            let doc = PenDocument {
                version: "1.0.0".into(),
                name: None,
                themes: None,
                variables: None,
                pages: None,
                children: vec![image_node("image", source)],
                format_version: None,
                responsive: None,
                id: None,
                app: None,
                routes: None,
                state: None,
                lifecycle: None,
                logic_modules: None,
                design_md: None,
                conversion: None,
            };
            let mut output = Vec::new();
            let stats = write_document_with_extension(
                &mut output,
                &doc,
                &snapshot,
                "editorMeta",
                &serde_json::json!({"activePageIndex": 2}),
            )
            .expect("stream document");
            let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
            assert!(stats.wrote_images_table);
            assert!(stats.wrote_image_thumbs);
            assert_eq!(
                parsed["imageThumbs"],
                serde_json::json!({paint_id.to_string(): "/9j/2Q=="})
            );
            assert_eq!(parsed["editorMeta"]["activePageIndex"], 2);
        });
    }

    #[test]
    fn dangling_table_ref_does_not_capture_a_thumbnail_for_the_ref_text() {
        image_thumbs::with_test_serialized(|| {
            image_thumbs::clear_registry();
            let dangling = ImageSrc::from("op-image:missing");
            let misleading_id = crate::node::image_src::paint_image_id(dangling.as_str());
            image_thumbs::store_thumb(misleading_id, vec![0xff, 0xd8, 0xff, 0xd9]);
            let snapshot = image_thumbs::capture_snapshot();

            with_save_scope(&snapshot, |tables| {
                let json = serde_json::to_string(&dangling).expect("serialize dangling ref");
                assert_eq!(json, r#""op-image:missing""#);
                assert!(tables.image_thumbs().is_empty());
                assert!(tables.images_is_empty());
            });
            image_thumbs::clear_registry();
        });
    }

    #[test]
    fn collision_probe_keeps_both_payloads_in_the_streaming_table() {
        let source = large_source("collision");
        let base = format!("{:016x}", fnv1a64(source.as_str().as_bytes()));
        let thumbnails = image_thumbs::capture_snapshot();
        with_save_scope(&thumbnails, |tables| {
            let mut collector = tables.collector.borrow_mut();
            collector
                .images
                .push((base.clone(), Arc::from("data:image/png;base64,different")));
            collector.image_indices.insert(base.clone(), 0);
            drop(collector);
            let reference = serde_json::to_string(&source).expect("serialize collision");
            assert!(reference.contains(&format!("op-image:{base}-2")));
            let image_table = serde_json::to_value(tables.images()).expect("table");
            assert_eq!(image_table.as_object().map(|map| map.len()), Some(2));
        });
    }

    #[test]
    fn canonical_writer_preserves_legacy_table_and_top_level_key_order() {
        image_thumbs::with_test_serialized(|| {
            image_thumbs::clear_registry();
            let sources: Vec<String> = (0..9)
                .map(|index| large_source(&format!("order-{index}")).to_string())
                .collect();
            for source in &sources {
                image_thumbs::store_thumb(
                    crate::node::image_src::paint_image_id(source),
                    vec![0xff, 0xd8, index_byte(source), 0xd9],
                );
            }
            let document: PenDocument = serde_json::from_value(serde_json::json!({
                "version": "1.0.0",
                "pages": [
                    {"id": "p1", "name": "One", "children": [
                        {"type": "image", "id": "p1-a", "src": sources[0]},
                        {"type": "image", "id": "p1-b", "src": sources[1]}
                    ]},
                    {"id": "p2", "name": "Two", "children": [
                        {"type": "image", "id": "p2-a", "src": sources[2]}
                    ]}
                ],
                "children": [
                    {"type": "image", "id": "root-a", "src": sources[3]},
                    {"type": "frame", "id": "frame", "fill": [
                        {"type": "image", "url": sources[4]}
                    ], "children": [
                        {"type": "image", "id": "child-a", "src": sources[5]},
                        {"type": "image", "id": "child-b", "src": sources[6]}
                    ]},
                    {"type": "tabs", "id": "tabs", "states": {
                        "hover": {"fill": [{"type": "image", "url": sources[7]}]},
                        "pressed": {"stroke": {"thickness": 1, "fill": [
                            {"type": "image", "url": sources[8]}
                        ]}}
                    }}
                ]
            }))
            .expect("ordered document");
            let extension = serde_json::json!({"activePageIndex": 1});

            let mut legacy = serde_json::to_value(&document).expect("legacy Value");
            super::super::externalize_images(&mut legacy);
            legacy
                .as_object_mut()
                .expect("document object")
                .insert("editorMeta".into(), extension.clone());
            let legacy_text = serde_json::to_string_pretty(&legacy).expect("legacy text");

            let snapshot = image_thumbs::capture_snapshot();
            let mut streamed = Vec::new();
            write_document_with_extension(
                &mut streamed,
                &document,
                &snapshot,
                "editorMeta",
                &extension,
            )
            .expect("streaming text");
            let streamed = String::from_utf8(streamed).expect("UTF-8");

            assert_eq!(
                streamed, legacy_text,
                "typed prepass must preserve images, imageThumbs, and root key order"
            );
            image_thumbs::clear_registry();
        });
    }

    fn index_byte(source: &str) -> u8 {
        source
            .bytes()
            .find(|byte| byte.is_ascii_digit())
            .unwrap_or(b'0')
    }

    #[test]
    fn writer_error_restores_transparent_image_serialization() {
        struct RejectWrites;
        impl std::io::Write for RejectWrites {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("intentional failure"))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let source = large_source("writer-error");
        let doc = PenDocument {
            version: "1.0.0".into(),
            name: None,
            themes: None,
            variables: None,
            pages: None,
            children: vec![image_node("image", source.clone())],
            format_version: None,
            responsive: None,
            id: None,
            app: None,
            routes: None,
            state: None,
            lifecycle: None,
            logic_modules: None,
            design_md: None,
            conversion: None,
        };
        let thumbnails = image_thumbs::capture_snapshot();
        let error = write_document_with_extension(
            &mut RejectWrites,
            &doc,
            &thumbnails,
            "editorMeta",
            &serde_json::json!({"activePageIndex": 0}),
        )
        .expect_err("writer rejects output");
        assert!(error.is_io());
        assert!(
            serde_json::to_string(&source)
                .expect("serialize after failure")
                .contains("data:image/png"),
            "TLS save scope was restored after serializer failure"
        );
    }
}
