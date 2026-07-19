//! Runtime blur-up thumbnail registry and serialized `imageThumbs` helpers.
//!
//! Entries are keyed directly by the stable paint id and own compact encoded
//! JPEG bytes behind an [`Arc`]. The 4 KiB limit is enforced both for runtime
//! stores and persisted input because platform painters may decode these small
//! placeholders synchronously.

use base64::Engine as _;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Key of the top-level thumbnail table in serialized documents.
pub(crate) const IMAGE_THUMBS_KEY: &str = "imageThumbs";
/// Hard bound for paint-thread thumbnail decoding.
pub const MAX_THUMB_BYTES: usize = 4 * 1_024;
const MAX_BASE64_BYTES: usize = MAX_THUMB_BYTES.div_ceil(3) * 4;

type ThumbMap = HashMap<u64, Arc<[u8]>>;

/// Decoded thumbnail table waiting for its typed document load to succeed.
///
/// Loaders take this seed out of raw JSON before deserializing the document,
/// then attach it to the successfully parsed document. A missing table is an
/// absent seed, not an empty replacement, so temporary inline parses cannot
/// clear the active content cache.
#[must_use = "a parsed thumbnail table has no effect until it is committed"]
#[derive(Clone)]
pub struct PendingThumbSeed(Option<ThumbMap>);

impl PendingThumbSeed {
    /// Immediately publish a present table as the active registry.
    ///
    /// Typed document loaders should prefer [`attach_to_document`] so the
    /// table changes only when that document becomes paint-active.
    pub fn commit(self) {
        if let Some(loaded) = self.0 {
            replace_from_load(loaded);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DocumentSeedKey {
    version_ptr: usize,
    version_len: usize,
    version_capacity: usize,
}

impl DocumentSeedKey {
    fn for_document(doc: &crate::PenDocument) -> Self {
        Self {
            version_ptr: doc.version.as_ptr() as usize,
            version_len: doc.version.len(),
            version_capacity: doc.version.capacity(),
        }
    }
}

#[derive(Default)]
struct PendingDocumentSeeds {
    entries: HashMap<DocumentSeedKey, ThumbMap>,
    order: VecDeque<DocumentSeedKey>,
}

impl PendingDocumentSeeds {
    fn insert(&mut self, key: DocumentSeedKey, seed: ThumbMap) {
        if self.entries.insert(key, seed).is_some() {
            self.order.retain(|candidate| *candidate != key);
        }
        self.order.push_back(key);
        while self.entries.len() > MAX_PENDING_DOCUMENT_SEEDS {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }

    fn take(&mut self, key: DocumentSeedKey) -> Option<ThumbMap> {
        let seed = self.entries.remove(&key)?;
        self.order.retain(|candidate| *candidate != key);
        Some(seed)
    }

    fn cloned(&self, key: DocumentSeedKey) -> Option<ThumbMap> {
        self.entries.get(&key).cloned()
    }

    fn remove(&mut self, key: DocumentSeedKey) -> bool {
        let removed = self.entries.remove(&key).is_some();
        if removed {
            self.order.retain(|candidate| *candidate != key);
        }
        removed
    }
}

static THUMBS: OnceLock<Mutex<ThumbMap>> = OnceLock::new();
static PENDING_DOCUMENT_SEEDS: OnceLock<Mutex<PendingDocumentSeeds>> = OnceLock::new();
static NEXT_DOCUMENT_TAG: AtomicUsize = AtomicUsize::new(0);
const MAX_PENDING_DOCUMENT_SEEDS: usize = 128;
const DOCUMENT_TAG_VARIANTS: usize = 64;
const DOCUMENT_TAG_BASE_CAPACITY: usize = 32;

fn thumbs() -> &'static Mutex<ThumbMap> {
    THUMBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_thumbs() -> std::sync::MutexGuard<'static, ThumbMap> {
    thumbs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn pending_document_seeds() -> &'static Mutex<PendingDocumentSeeds> {
    PENDING_DOCUMENT_SEEDS.get_or_init(|| Mutex::new(PendingDocumentSeeds::default()))
}

fn lock_pending_document_seeds() -> std::sync::MutexGuard<'static, PendingDocumentSeeds> {
    pending_document_seeds()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tag_pending_document(doc: &mut crate::PenDocument) {
    let variant = NEXT_DOCUMENT_TAG.fetch_add(1, Ordering::Relaxed) % DOCUMENT_TAG_VARIANTS;
    let capacity = doc
        .version
        .len()
        .saturating_add(DOCUMENT_TAG_BASE_CAPACITY + variant);
    let mut tagged = String::with_capacity(capacity);
    tagged.push_str(&doc.version);
    doc.version = tagged;
}

/// Associate a parsed seed with its typed document without publishing it.
///
/// The side table avoids changing `PenDocument`'s public struct shape. Its
/// key follows the owned version buffer across moves. A small capacity tag
/// gives even an empty version a private allocation and prevents a dropped
/// seed from colliding with an ordinary, untagged document before bounded
/// eviction removes it. Generic parse-only documents that are dropped without
/// an explicit discard leave only a bounded entry; a future present-table
/// attach overwrites any reused key before that document can activate.
pub fn attach_to_document(doc: &mut crate::PenDocument, pending: PendingThumbSeed) {
    let Some(seed) = pending.0 else {
        return;
    };
    tag_pending_document(doc);
    lock_pending_document_seeds().insert(DocumentSeedKey::for_document(doc), seed);
}

/// Activate and consume the present seed associated with `doc`.
///
/// Returns `false` for documents loaded without `imageThumbs`; that path is a
/// strict no-op so unrelated inline parses keep the active content cache.
pub fn activate_for_document(doc: &crate::PenDocument) -> bool {
    let Some(seed) = lock_pending_document_seeds().take(DocumentSeedKey::for_document(doc)) else {
        return false;
    };
    replace_from_load(seed);
    true
}

/// Discard a document's pending seed without changing the active registry.
pub fn discard_for_document(doc: &crate::PenDocument) -> bool {
    let Some(pending) = PENDING_DOCUMENT_SEEDS.get() else {
        return false;
    };
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(DocumentSeedKey::for_document(doc))
}

/// Propagate a pending seed when a parsed document is cloned before install.
pub(crate) fn propagate_to_clone(source: &crate::PenDocument, cloned: &mut crate::PenDocument) {
    let Some(pending) = PENDING_DOCUMENT_SEEDS.get() else {
        return;
    };
    let seed = pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .cloned(DocumentSeedKey::for_document(source));
    if let Some(seed) = seed {
        attach_to_document(cloned, PendingThumbSeed(Some(seed)));
    }
}

/// Store encoded JPEG bytes for a source's stable paint id.
///
/// The registry owns one `Arc`; paint and backend lookups clone that Arc
/// rather than copying the thumbnail payload. Payloads over
/// [`MAX_THUMB_BYTES`] are ignored.
pub fn store_thumb(paint_id: u64, jpeg_bytes: impl Into<Arc<[u8]>>) {
    let jpeg_bytes = jpeg_bytes.into();
    if jpeg_bytes.len() <= MAX_THUMB_BYTES {
        lock_thumbs().insert(paint_id, jpeg_bytes);
    }
}

/// Return the encoded JPEG thumbnail registered for `paint_id`.
pub fn thumb_for(paint_id: u64) -> Option<Arc<[u8]>> {
    lock_thumbs().get(&paint_id).cloned()
}

/// Consume a serialized `imageThumbs` table and seed the active registry.
///
/// Direct JSON loaders should call this before taking the independent
/// `images` table for [`crate::node::image_src::intern::with_load_scope`].
/// A missing table is a no-op. A present malformed table is an empty
/// replacement, matching a present table whose individual entries all fail
/// validation.
pub fn seed_from_document(root: &mut Value) {
    take_pending_from_document(root).commit();
}

/// Remove and decode a serialized `imageThumbs` table without publishing it.
///
/// Typed document loaders use this transactional seam so malformed documents
/// cannot replace the registry belonging to the document that remains open.
/// A missing table produces an absent seed whose activation is a no-op.
pub fn take_pending_from_document(root: &mut Value) -> PendingThumbSeed {
    PendingThumbSeed(take_present_from_document(root))
}

/// Serialize only thumbnails referenced by the document being saved.
pub(crate) fn snapshot_for(referenced_ids: &BTreeSet<u64>) -> Map<String, Value> {
    let registry = lock_thumbs();
    referenced_ids
        .iter()
        .filter_map(|id| {
            registry.get(id).map(|bytes| {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                (id.to_string(), Value::String(encoded))
            })
        })
        .collect()
}

/// Remove and decode a document's thumbnail table.
///
/// Missing, malformed, non-decimal, and invalid-base64 entries yield no
/// thumbnail. This compatibility helper collapses absence to an empty map;
/// lifecycle-aware callers use [`take_pending_from_document`] instead.
#[cfg(test)]
pub(crate) fn take_from_document(root: &mut Value) -> ThumbMap {
    take_present_from_document(root).unwrap_or_default()
}

fn take_present_from_document(root: &mut Value) -> Option<ThumbMap> {
    let Value::Object(root) = root else {
        return None;
    };
    let value = root.remove(IMAGE_THUMBS_KEY)?;
    let Value::Object(table) = value else {
        return Some(HashMap::new());
    };
    Some(
        table
            .into_iter()
            .filter_map(|(id, value)| {
                let id = id.parse::<u64>().ok()?;
                let encoded = value.as_str()?;
                if encoded.len() > MAX_BASE64_BYTES {
                    return None;
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()?;
                if bytes.len() > MAX_THUMB_BYTES {
                    return None;
                }
                Some((id, Arc::from(bytes)))
            })
            .collect(),
    )
}

/// Clear active and pending thumbnail state.
///
/// Normal document loads never call this; it is an explicit baseline seam
/// for tests and diagnostic sessions.
pub fn clear_registry() {
    #[cfg(test)]
    return with_test_serialized(clear_registry_inner);

    #[cfg(not(test))]
    clear_registry_inner();
}

fn clear_registry_inner() {
    lock_thumbs().clear();
    let mut pending = lock_pending_document_seeds();
    pending.entries.clear();
    pending.order.clear();
}

#[cfg(test)]
pub(crate) fn pending_document_seed_count() -> usize {
    lock_pending_document_seeds().entries.len()
}

/// Atomically replace the active document's runtime thumbnail registry.
pub(crate) fn replace_from_load(loaded: ThumbMap) {
    #[cfg(test)]
    return with_test_serialized(|| *lock_thumbs() = loaded);

    #[cfg(not(test))]
    {
        *lock_thumbs() = loaded;
    }
}

#[cfg(test)]
static TEST_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_SERIAL_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Keep tests that intentionally replace the process-global registry isolated.
#[cfg(test)]
pub(crate) fn with_test_serialized<R>(f: impl FnOnce() -> R) -> R {
    if TEST_SERIAL_HELD.with(std::cell::Cell::get) {
        return f();
    }
    let _lock = TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TEST_SERIAL_HELD.with(|held| held.set(true));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_SERIAL_HELD.with(|held| held.set(false));
        }
    }
    let _reset = Reset;
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xd9];

    #[test]
    fn registry_returns_shared_thumbnail_bytes() {
        with_test_serialized(|| {
            replace_from_load(Default::default());
            store_thumb(41, JPEG.to_vec());

            let first = thumb_for(41).expect("stored thumbnail");
            let second = thumb_for(41).expect("stored thumbnail");
            assert_eq!(&*first, JPEG);
            assert!(Arc::ptr_eq(&first, &second), "lookups clone one Arc");
        });
    }

    #[test]
    fn snapshot_uses_decimal_ids_and_compact_base64() {
        with_test_serialized(|| {
            replace_from_load(Default::default());
            store_thumb(41, JPEG.to_vec());
            store_thumb(99, vec![1, 2, 3]);

            let table = snapshot_for(&BTreeSet::from([41]));
            assert_eq!(Value::Object(table.clone()), json!({"41": "/9j/2Q=="}));
            assert!(table.get("99").is_none(), "unreferenced thumb is pruned");
        });
    }

    #[test]
    fn direct_load_table_replaces_registry_but_absence_is_a_noop() {
        with_test_serialized(|| {
            replace_from_load(Default::default());
            store_thumb(99, vec![1, 2, 3]);
            let mut with_table = json!({"imageThumbs": {"41": "/9j/2Q=="}});

            seed_from_document(&mut with_table);
            assert!(with_table.get("imageThumbs").is_none());
            assert_eq!(&*thumb_for(41).expect("loaded thumbnail"), JPEG);
            assert!(thumb_for(99).is_none(), "load replaces stale entries");

            let mut without_table = Value::Object(Default::default());
            seed_from_document(&mut without_table);
            assert!(
                thumb_for(41).is_some(),
                "an absent additive table must preserve the content cache"
            );
        });
    }

    #[test]
    fn pending_table_changes_nothing_until_committed() {
        with_test_serialized(|| {
            replace_from_load(Default::default());
            store_thumb(99, vec![1, 2, 3]);
            let mut document = json!({"imageThumbs": {"41": "/9j/2Q=="}});

            let pending = take_pending_from_document(&mut document);
            assert_eq!(
                &*thumb_for(99).expect("active registry remains visible"),
                &[1, 2, 3]
            );
            assert!(thumb_for(41).is_none());

            pending.commit();
            assert!(thumb_for(99).is_none());
            assert_eq!(&*thumb_for(41).expect("committed thumbnail"), JPEG);
        });
    }

    #[test]
    fn oversized_and_malformed_thumbnails_never_enter_the_registry() {
        with_test_serialized(|| {
            replace_from_load(Default::default());
            store_thumb(41, vec![0; 4_097]);
            assert!(thumb_for(41).is_none(), "runtime stores enforce the bound");

            let oversized = base64::engine::general_purpose::STANDARD.encode(vec![0; 4_097]);
            let mut document = json!({
                "imageThumbs": {
                    "41": oversized,
                    "42": "not base64!",
                    "not-decimal": "/9j/2Q=="
                }
            });
            replace_from_load(take_from_document(&mut document));
            assert!(
                thumb_for(41).is_none(),
                "persisted bytes over 4 KiB are skipped"
            );
            assert!(thumb_for(42).is_none(), "malformed base64 is skipped");
        });
    }

    #[test]
    fn reused_document_key_keeps_only_the_newest_seed() {
        let key = DocumentSeedKey {
            version_ptr: 17,
            version_len: 5,
            version_capacity: 37,
        };
        let mut pending = PendingDocumentSeeds::default();
        pending.insert(key, HashMap::from([(1, Arc::from([1_u8]))]));
        pending.insert(key, HashMap::from([(2, Arc::from([2_u8]))]));

        let seed = pending.take(key).expect("newest seed remains");
        assert!(!seed.contains_key(&1), "the stale seed was overwritten");
        assert_eq!(&**seed.get(&2).expect("new seed"), &[2]);
        assert!(pending.entries.is_empty());
        assert!(pending.order.is_empty());
    }
}
