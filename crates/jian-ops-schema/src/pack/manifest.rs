//! `.op.pack` manifest types — see [`super`] for the on-disk layout.

use serde::{Deserialize, Serialize};

/// Wire string identifying the archive format. Distinct from
/// [`PACK_FORMAT_VERSION`] so a reader can detect "this is an
/// `.op.pack` zip, not some other zip" cheaply.
pub const PACK_FORMAT: &str = "op.pack";

/// Current `.op.pack` format version. Bump on **breaking** wire
/// changes; additive AOT entries don't bump (presence is opt-in via
/// [`AotInventory`]).
pub const PACK_FORMAT_VERSION: &str = "0.1";

// ------------------------------------------------------------------
// Canonical entry paths inside the zip.
// ------------------------------------------------------------------

/// Top-level zip entry that always carries the canonical JSON document.
/// A reader that finds no other entries must still find this one.
pub const ENTRY_APP_OP: &str = "app.op";

/// Manifest path. The JSON schema is [`AotManifest`].
pub const ENTRY_MANIFEST: &str = "manifest.json";

/// Precompiled-bytecode blob for every expression the document
/// references (page bindings, action expressions, list-key extractors).
/// Plan 19 Task 6 byte format: SoA tuples of (source_hash, len, bytes).
pub const ENTRY_AOT_EXPRESSIONS: &str = "aot/expressions.bin";

/// `map<node_id, (x, y, w, h)>` precomputed for the default viewport
/// declared in [`AotInventory::default_viewport`]. Lets the runtime
/// skip its first layout pass.
pub const ENTRY_AOT_INITIAL_LAYOUT: &str = "aot/initial_layout.bin";

/// Serialized `StateGraph` initial values. Faster to deserialize than
/// to walk `app.op`'s `state` blocks again.
pub const ENTRY_AOT_DEFAULT_STATE: &str = "aot/default_state.bin";

/// Directory prefix for font assets. Per [`FontEntry`], each family
/// can carry both a critical-frame subset and a full-file lazy load.
pub const DIR_FONTS: &str = "fonts/";

/// Directory prefix for image assets referenced by `image` nodes.
pub const DIR_IMAGES: &str = "images/";

/// Directory prefix for Tier-3 WASM logic modules referenced by `call`
/// actions.
pub const DIR_LOGIC: &str = "logic/";

// ------------------------------------------------------------------
// Manifest types.
// ------------------------------------------------------------------

/// Top-level manifest written to `manifest.json`. Always present;
/// readers parse this first to discover the rest of the archive.
///
/// `Eq` is intentionally NOT derived because the optional `aot` field
/// transitively holds `f32` (via [`DefaultViewport`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AotManifest {
    /// Always [`PACK_FORMAT`] (`"op.pack"`). Distinguishes a Jian pack
    /// from any other zip a user might point us at.
    pub format: String,
    /// Always [`PACK_FORMAT_VERSION`] (`"0.1"`).
    pub version: String,
    /// App identity (id / name / version). Mirrors `app.op`'s `app`
    /// block so a launcher can list packs without unzipping
    /// `app.op` itself.
    pub app: ManifestAppMetadata,
    /// Capabilities the bundled document declares, in canonical
    /// kebab-case wire form (`"network"`, `"storage"`, `"file_system"`).
    /// A host enforces these via the runtime's
    /// `DeclaredCapabilityGate` — the manifest copy is informational
    /// (lets a launcher show "this app wants: network, storage" without
    /// loading the runtime).
    ///
    /// Always serialized (including as `[]`) so the wire shape stays
    /// stable for parsers that key off field presence; `default` lets
    /// older packs without the field round-trip.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Zip entries the writer included **other than `manifest.json`
    /// itself** — the manifest is always present (a reader finds it
    /// before reading any `entries` content), so listing it here
    /// would be redundant.
    ///
    /// Writers MUST list every non-manifest entry they emit, including
    /// optional AOT files (`aot/*.bin`), font subsets / full files
    /// under `fonts/`, image binaries under `images/`, and WASM logic
    /// modules under `logic/`. Readers may rely on this list as a
    /// complete inventory rather than walking every `ZipEntry` —
    /// keeping `entries` partial breaks lazy loaders that key off it.
    /// At minimum this Vec must contain [`ENTRY_APP_OP`].
    pub entries: Vec<String>,
    /// Optional AOT inventory. `None` means a JSON-only pack
    /// (the existing `jian pack` MVP shape — no AOT entries to load).
    /// `Some` means AOT entries are present at the paths described.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aot: Option<AotInventory>,
}

impl AotManifest {
    /// Build a manifest skeleton for a JSON-only pack (no AOT). The
    /// existing `jian pack` MVP can adopt this directly once it wires
    /// in the typed manifest path; the wire shape matches its current
    /// untyped `serde_json::Value`-based output.
    pub fn json_only(app: ManifestAppMetadata, capabilities: Vec<String>) -> Self {
        Self {
            format: PACK_FORMAT.to_owned(),
            version: PACK_FORMAT_VERSION.to_owned(),
            app,
            capabilities,
            entries: vec![ENTRY_APP_OP.to_owned()],
            aot: None,
        }
    }
}

/// Manifest copy of an app's identity. Same field shapes as
/// [`crate::app::AppConfig`] but only the launcher-relevant trio so we
/// don't duplicate the entire schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestAppMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// AOT inventory — every field is `Option` because each AOT entry is
/// independently opt-in. A pack can ship precompiled expressions
/// without precomputed layout, etc.
///
/// `Eq` is intentionally NOT derived because [`DefaultViewport`] holds
/// `f32` (no `Eq` impl). `PartialEq` is enough for round-trip tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AotInventory {
    /// Path to [`ENTRY_AOT_EXPRESSIONS`] if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expressions: Option<String>,
    /// Path to [`ENTRY_AOT_INITIAL_LAYOUT`] if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_layout: Option<String>,
    /// Path to [`ENTRY_AOT_DEFAULT_STATE`] if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_state: Option<String>,
    /// Default viewport used to bake `initial_layout.bin`. Readers
    /// use this to decide "is the precomputed layout valid for the
    /// window I'm about to open?". When the host viewport differs the
    /// runtime falls back to a fresh layout pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_viewport: Option<DefaultViewport>,
    /// Per-family font assets. Empty when no fonts were subsetted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<FontEntry>,
}

/// Viewport rect the AOT layout was baked against. Hosts compare this
/// to the actual window size at launch and decide whether to use the
/// precomputed layout or fall back to a runtime layout pass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DefaultViewport {
    pub width: f32,
    pub height: f32,
}

/// One font family's pack entries. `critical` is the codepoint subset
/// loaded for the first frame (Plan 19 Task 4); `full` is the complete
/// font file loaded on a post-paint background frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FontEntry {
    pub family: String,
    /// Path to the critical subset. `None` means "no subset; load the
    /// full file directly during the critical path".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<String>,
    /// Path to the full font file. Always present.
    pub full: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pack_format_constants_are_stable_strings() {
        // These are wire-format. A breaking change here is a major
        // version bump; this test guards against accidental edits.
        assert_eq!(PACK_FORMAT, "op.pack");
        assert_eq!(PACK_FORMAT_VERSION, "0.1");
        assert_eq!(ENTRY_APP_OP, "app.op");
        assert_eq!(ENTRY_MANIFEST, "manifest.json");
        assert_eq!(ENTRY_AOT_EXPRESSIONS, "aot/expressions.bin");
        assert_eq!(ENTRY_AOT_INITIAL_LAYOUT, "aot/initial_layout.bin");
        assert_eq!(ENTRY_AOT_DEFAULT_STATE, "aot/default_state.bin");
        assert_eq!(DIR_FONTS, "fonts/");
        assert_eq!(DIR_IMAGES, "images/");
        assert_eq!(DIR_LOGIC, "logic/");
    }

    #[test]
    fn json_only_manifest_round_trips() {
        let m = AotManifest::json_only(
            ManifestAppMetadata {
                id: "demo.counter".into(),
                name: "Counter".into(),
                version: "0.1.0".into(),
            },
            vec!["network".into(), "storage".into()],
        );
        let s = serde_json::to_string(&m).unwrap();
        let back: AotManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
        assert!(back.aot.is_none());
        assert_eq!(back.entries, vec!["app.op"]);
    }

    #[test]
    fn json_only_manifest_keeps_empty_capabilities_on_wire() {
        // Codex round 1 MODERATE: existing `jian-cli pack` always
        // emits `"capabilities": []` (the bare `serde_json::json!`
        // literal preserves empty arrays). The typed manifest must
        // match — drop the `skip_serializing_if = Vec::is_empty`
        // annotation and always serialize the field.
        let m = AotManifest::json_only(
            ManifestAppMetadata {
                id: "x".into(),
                name: "X".into(),
                version: "0".into(),
            },
            vec![],
        );
        let s = serde_json::to_value(&m).unwrap();
        assert_eq!(
            s.get("capabilities").and_then(|v| v.as_array()).map(|v| v.len()),
            Some(0),
            "empty capabilities must serialize as `[]`, not be omitted: {s}"
        );
    }

    #[test]
    fn manifest_without_capabilities_field_deserializes_to_empty() {
        // Round-trip the third wire-shape compatibility case: a
        // manifest emitted by an older writer that didn't include
        // `capabilities` at all must still parse, with the field
        // defaulting to `vec![]`.
        let json = json!({
            "format": "op.pack",
            "version": "0.1",
            "app": { "id": "old", "name": "Old", "version": "0.0.1" },
            "entries": ["app.op"]
        });
        let m: AotManifest = serde_json::from_value(json).expect("missing-capabilities round-trips");
        assert!(m.capabilities.is_empty());
    }

    #[test]
    fn json_only_manifest_omits_aot_field_on_wire() {
        // Wire compatibility with the existing MVP `jian pack` shape:
        // a JSON-only pack must not carry an `"aot": null` field. The
        // skip_serializing_if guarantees that.
        let m = AotManifest::json_only(
            ManifestAppMetadata {
                id: "x".into(),
                name: "X".into(),
                version: "0".into(),
            },
            vec![],
        );
        let s = serde_json::to_value(&m).unwrap();
        assert!(s.get("aot").is_none(), "aot must be absent for JSON-only packs: {s}");
    }

    #[test]
    fn aot_inventory_round_trips_with_subset_of_fields_set() {
        let inv = AotInventory {
            expressions: Some(ENTRY_AOT_EXPRESSIONS.into()),
            initial_layout: None,
            default_state: Some(ENTRY_AOT_DEFAULT_STATE.into()),
            default_viewport: Some(DefaultViewport { width: 800.0, height: 600.0 }),
            fonts: vec![FontEntry {
                family: "Inter".into(),
                critical: Some("fonts/Inter-sub.ttf".into()),
                full: "fonts/Inter.ttf".into(),
            }],
        };
        let s = serde_json::to_string(&inv).unwrap();
        let back: AotInventory = serde_json::from_str(&s).unwrap();
        assert_eq!(back, inv);
    }

    #[test]
    fn aot_inventory_skips_none_fields_on_wire() {
        let inv = AotInventory {
            expressions: Some(ENTRY_AOT_EXPRESSIONS.into()),
            ..Default::default()
        };
        let s = serde_json::to_value(&inv).unwrap();
        // serde defaults to snake_case for these fields. Both casings
        // are checked with `&&` so the assertion isn't trivially
        // satisfiable by one missing key (Codex round 1 MINOR — the
        // earlier `||` shape always passed).
        assert!(
            s.get("initialLayout").is_none() && s.get("initial_layout").is_none(),
            "initial_layout must not serialize when None"
        );
        assert!(
            s.get("defaultState").is_none() && s.get("default_state").is_none(),
            "default_state must not serialize when None"
        );
        assert!(s.get("fonts").is_none(), "empty fonts must not serialize");
    }

    #[test]
    fn font_entry_round_trips_with_and_without_critical_subset() {
        let with_subset = FontEntry {
            family: "Inter".into(),
            critical: Some("fonts/Inter-sub.ttf".into()),
            full: "fonts/Inter.ttf".into(),
        };
        let without_subset = FontEntry {
            family: "Roboto".into(),
            critical: None,
            full: "fonts/Roboto.ttf".into(),
        };
        for entry in [with_subset, without_subset] {
            let s = serde_json::to_string(&entry).unwrap();
            let back: FontEntry = serde_json::from_str(&s).unwrap();
            assert_eq!(back, entry);
        }
    }

    #[test]
    fn full_aot_manifest_parses_canonical_wire_form() {
        // This is the format a future writer commit will produce.
        // The test pins the wire shape so the writer + reader can be
        // built independently and still interoperate.
        let json = json!({
            "format": "op.pack",
            "version": "0.1",
            "app": { "id": "demo.counter", "name": "Counter", "version": "0.1.0" },
            "capabilities": ["network"],
            "entries": ["app.op", "aot/expressions.bin", "fonts/Inter-sub.ttf"],
            "aot": {
                "expressions": "aot/expressions.bin",
                "default_viewport": { "width": 800.0, "height": 600.0 },
                "fonts": [
                    { "family": "Inter", "critical": "fonts/Inter-sub.ttf", "full": "fonts/Inter.ttf" }
                ]
            }
        });
        let m: AotManifest = serde_json::from_value(json).expect("canonical wire form parses");
        let aot = m.aot.expect("AOT present");
        assert_eq!(aot.expressions.as_deref(), Some("aot/expressions.bin"));
        assert!(aot.initial_layout.is_none());
        assert!(aot.default_state.is_none());
        assert_eq!(aot.fonts.len(), 1);
        assert_eq!(aot.default_viewport.unwrap().width, 800.0);
    }
}
