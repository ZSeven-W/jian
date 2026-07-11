//! Backward/forward compat loader.
//!
//! Responsibilities:
//! 1. Parse JSON into `PenDocument` (serde-level).
//! 2. Check `formatVersion` / `version` and reject unsupported majors.
//! 3. Collect non-fatal warnings for unknown fields (future-compat).
//! 4. Return document + warnings.

use crate::document::PenDocument;
use crate::error::{LoadResult, LoadWarning, OpsResult, OpsSchemaError};
use crate::version;

/// Options controlling compat-load behaviour.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    /// Promote explicitly marked legacy frames to widget nodes
    /// (in-memory only). Runtime hosts set this; the OP editor keeps
    /// it off so migration stays an explicit user action.
    pub promote_legacy_widgets: bool,
}

/// Parse an `.op` JSON blob into a `PenDocument` with compat warnings.
pub fn load_str(src: &str) -> OpsResult<LoadResult<PenDocument>> {
    load_str_with(src, LoadOptions::default())
}

/// Like [`load_str`], but with explicit [`LoadOptions`].
pub fn load_str_with(src: &str, opts: LoadOptions) -> OpsResult<LoadResult<PenDocument>> {
    let mut raw: serde_json::Value = serde_json::from_str(src)?;

    let format_version = raw.get("formatVersion").and_then(|v| v.as_str());
    let legacy_version = raw.get("version").and_then(|v| v.as_str());
    let v = format_version.or(legacy_version);

    if !version::supports(v) {
        return Err(OpsSchemaError::UnsupportedFormatVersion {
            found: v.unwrap_or("<missing>").to_owned(),
            supported: version::FORMAT_VERSION_CURRENT,
        });
    }

    let mut warnings = Vec::new();
    if let serde_json::Value::Object(map) = &raw {
        for k in map.keys() {
            if !KNOWN_TOP_LEVEL_FIELDS.contains(&k.as_str()) {
                warnings.push(LoadWarning::UnknownField {
                    path: "$".to_owned(),
                    field: k.to_owned(),
                });
            }
        }
    }

    if let Some(fv) = format_version {
        let (cur_major, cur_minor) = version::parse(Some(version::FORMAT_VERSION_CURRENT));
        let (major, minor) = version::parse(Some(fv));
        // Warn on any version newer than we know — including a newer
        // *minor*, which may carry node types or fields we silently
        // drop. (A newer major is already rejected above.)
        if major > cur_major || (major == cur_major && minor > cur_minor) {
            warnings.push(LoadWarning::FutureFormatVersion {
                found: fv.to_owned(),
                supported_max: version::FORMAT_VERSION_CURRENT,
            });
        }
    }

    if raw.get("responsive").and_then(serde_json::Value::as_bool) == Some(true) {
        let (major, minor) = version::parse(format_version.or(legacy_version));
        if major < 1 || (major == 1 && minor < 2) {
            warnings.push(LoadWarning::ResponsiveBelowMinor {
                declared: format_version
                    .or(legacy_version)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
    }

    if raw.get("logicModules").is_some() {
        warnings.push(LoadWarning::LogicModulesSkipped {
            reason: "Tier 3 WASM is not implemented in this build",
        });
    }

    // Decode the independent paint-id thumbnail table first, but do not
    // publish it until the typed document parse succeeds. Documents saved
    // with a deduplicated image table carry `op-image:<id>` refs; take that
    // table out next and resolve the refs
    // DURING the typed parse (`ImageSrc::deserialize` +
    // `image_src::intern`): every reference receives a clone of one
    // shared `Arc` per unique payload, so a thousand fills sharing an
    // image cost one allocation instead of a thousand copies. The same
    // scope content-interns duplicate large inline data URLs, so
    // legacy (pre-table) files stop inflating too.
    let pending_thumbs = crate::image_thumbs::take_pending_from_document(&mut raw);
    let table = crate::image_table::take_image_table(&mut raw);
    let mut doc: PenDocument =
        crate::node::image_src::intern::with_load_scope(table, || serde_json::from_value(raw))?;

    if opts.promote_legacy_widgets {
        for n in crate::promote::promote_document(&mut doc) {
            warnings.push(LoadWarning::LegacyRolePromoted {
                path: n.node_id,
                from_role: n.from_role,
                to: n.to,
            });
        }
    }

    crate::image_thumbs::attach_to_document(&mut doc, pending_thumbs);

    Ok(LoadResult {
        value: doc,
        warnings,
    })
}

const KNOWN_TOP_LEVEL_FIELDS: &[&str] = &[
    "formatVersion",
    "responsive",
    "version",
    "id",
    "name",
    "themes",
    "variables",
    "pages",
    "children",
    "app",
    "routes",
    "state",
    "lifecycle",
    "logicModules",
    // Both are real `PenDocument` fields (`design_md` / `conversion`); they
    // were missing from this list, so every document carrying a design brief
    // warned "UnknownField: designMd" on open.
    "designMd",
    "conversion",
    // Save-side deduplicated image table (`image_table.rs`) — resolved
    // back to inline data URLs before the typed parse.
    "images",
    // Paint-id keyed blur-up JPEG table — decoded into the runtime registry.
    "imageThumbs",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_v0() {
        let s = r#"{"version":"0.8.0","children":[]}"#;
        let r = load_str(s).unwrap();
        assert!(r.value.format_version.is_none());
        assert_eq!(r.warnings.len(), 0);
    }

    /// A document saved with the deduplicated image table must parse
    /// with every `op-image:` ref resolved back to the inline data
    /// URL — and without an UnknownField warning for `images`.
    #[test]
    fn load_resolves_image_table_refs() {
        let payload = format!("data:image/png;base64,{}", "A".repeat(4096));
        let s = format!(
            r#"{{"version":"0.8.0","images":{{"abc":"{payload}"}},"children":[{{"type":"image","id":"i1","name":"img","x":0,"y":0,"width":10,"height":10,"src":"op-image:abc"}}]}}"#
        );
        let r = load_str(&s).unwrap();
        assert_eq!(r.warnings.len(), 0, "images is a known field");
        let crate::node::PenNode::Image(img) = &r.value.children[0] else {
            panic!("image node expected");
        };
        assert_eq!(&img.src, payload.as_str(), "ref resolved to inline URL");
    }

    #[test]
    fn load_attaches_thumbnail_seed_until_document_activation() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::clear_registry();
            crate::image_thumbs::store_thumb(99, vec![1, 2, 3]);
            let src = r#"{"version":"0.8.0","imageThumbs":{"41":"/9j/2Q=="},"children":[]}"#;

            let loaded = load_str(src).expect("load document with thumbnails");
            assert_eq!(loaded.warnings.len(), 0, "imageThumbs is a known field");
            assert!(
                crate::image_thumbs::thumb_for(41).is_none(),
                "parse-only loads must not publish a document seed"
            );
            assert_eq!(
                &*crate::image_thumbs::thumb_for(99).expect("active thumbnail survives parse"),
                &[1, 2, 3]
            );

            assert!(crate::image_thumbs::activate_for_document(&loaded.value));
            assert_eq!(
                &*crate::image_thumbs::thumb_for(41).expect("activated thumbnail"),
                &[0xff, 0xd8, 0xff, 0xd9]
            );
            assert!(
                crate::image_thumbs::thumb_for(99).is_none(),
                "a present activated table replaces the prior document"
            );

            let absent = load_str(r#"{"version":"0.8.0","children":[]}"#)
                .expect("load document without thumbnails");
            assert!(!crate::image_thumbs::activate_for_document(&absent.value));
            assert!(
                crate::image_thumbs::thumb_for(41).is_some(),
                "a missing additive table leaves the content cache intact"
            );
        });
    }

    #[test]
    fn failed_typed_load_preserves_the_thumbnail_registry() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::replace_from_load(Default::default());
            crate::image_thumbs::store_thumb(99, vec![1, 2, 3]);
            let src = r#"{"version":"0.8.0","imageThumbs":{"41":"/9j/2Q=="},"children":[{"type":"not_a_node"}]}"#;

            assert!(load_str(src).is_err(), "the typed node parse must fail");
            assert_eq!(
                &*crate::image_thumbs::thumb_for(99).expect("prior thumbnail survives"),
                &[1, 2, 3]
            );
            assert!(
                crate::image_thumbs::thumb_for(41).is_none(),
                "a rejected document must not publish its thumbnail table"
            );
        });
    }

    #[test]
    fn pending_thumbnail_seed_follows_a_document_clone() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::clear_registry();
            let loaded =
                load_str(r#"{"version":"0.8.0","imageThumbs":{"42":"/9j/2Q=="},"children":[]}"#)
                    .expect("load document with thumbnails");

            let cloned = loaded.value.clone();
            drop(loaded);
            assert!(crate::image_thumbs::thumb_for(42).is_none());
            assert!(
                crate::image_thumbs::activate_for_document(&cloned),
                "the runtime-owned clone must retain the pending seed"
            );
            assert_eq!(
                &*crate::image_thumbs::thumb_for(42).expect("clone activated thumbnail"),
                &[0xff, 0xd8, 0xff, 0xd9]
            );
        });
    }

    #[test]
    fn explicitly_discarding_a_parsed_document_removes_its_pending_seed() {
        crate::image_thumbs::with_test_serialized(|| {
            crate::image_thumbs::clear_registry();
            let loaded =
                load_str(r#"{"version":"0.8.0","imageThumbs":{"43":"/9j/2Q=="},"children":[]}"#)
                    .expect("load document with thumbnails");
            assert_eq!(crate::image_thumbs::pending_document_seed_count(), 1);

            assert!(crate::image_thumbs::discard_for_document(&loaded.value));
            drop(loaded);
            assert_eq!(
                crate::image_thumbs::pending_document_seed_count(),
                0,
                "explicit discard must remove the exact pointer association"
            );

            let absent = load_str(r#"{"version":"0.8.0","children":[]}"#)
                .expect("parse subsequent document without a table");
            assert!(!crate::image_thumbs::activate_for_document(&absent.value));
            assert!(crate::image_thumbs::thumb_for(43).is_none());
        });
    }

    /// Two nodes referencing the same table entry must share ONE
    /// `Arc` allocation after load — resolving refs into independent
    /// copies would re-inflate exactly the duplication the table
    /// removed from the file.
    #[test]
    fn image_table_refs_share_one_allocation() {
        let payload = format!("data:image/png;base64,{}", "B".repeat(4096));
        let s = format!(
            r#"{{"version":"0.8.0","images":{{"abc":"{payload}"}},"children":[
                {{"type":"image","id":"i1","name":"a","x":0,"y":0,"width":1,"height":1,"src":"op-image:abc"}},
                {{"type":"image","id":"i2","name":"b","x":0,"y":0,"width":1,"height":1,"src":"op-image:abc"}}]}}"#
        );
        let r = load_str(&s).unwrap();
        let (crate::node::PenNode::Image(a), crate::node::PenNode::Image(b)) =
            (&r.value.children[0], &r.value.children[1])
        else {
            panic!("image nodes expected");
        };
        assert_eq!(&a.src, payload.as_str());
        assert!(
            std::ptr::eq(a.src.as_ref(), b.src.as_ref()),
            "both refs share the same Arc allocation"
        );
    }

    /// Legacy files with the payload inlined per node (pre-table
    /// saves) intern duplicate large data URLs on load, so opening an
    /// old bloated file doesn't cost one allocation per reference.
    #[test]
    fn legacy_inline_duplicates_intern_on_load() {
        let payload = format!("data:image/png;base64,{}", "C".repeat(4096));
        let s = format!(
            r#"{{"version":"0.8.0","children":[
                {{"type":"image","id":"i1","name":"a","x":0,"y":0,"width":1,"height":1,"src":"{payload}"}},
                {{"type":"image","id":"i2","name":"b","x":0,"y":0,"width":1,"height":1,"src":"{payload}"}}]}}"#
        );
        let r = load_str(&s).unwrap();
        let (crate::node::PenNode::Image(a), crate::node::PenNode::Image(b)) =
            (&r.value.children[0], &r.value.children[1])
        else {
            panic!("image nodes expected");
        };
        assert!(
            std::ptr::eq(a.src.as_ref(), b.src.as_ref()),
            "duplicate inline payloads intern to one allocation"
        );
    }

    #[test]
    fn load_v1_minimal() {
        let s = r#"{"formatVersion":"1.0","version":"1.0.0","id":"x","children":[]}"#;
        let r = load_str(s).unwrap();
        assert_eq!(r.value.format_version.as_deref(), Some("1.0"));
        assert_eq!(r.warnings.len(), 0);
    }

    #[test]
    fn load_unknown_field_produces_warning() {
        let s = r#"{"version":"0.8.0","children":[],"myExperimental":42}"#;
        let r = load_str(s).unwrap();
        assert!(r.warnings.iter().any(
            |w| matches!(w, LoadWarning::UnknownField { field, .. } if field == "myExperimental")
        ));
    }

    #[test]
    fn future_minor_warns_instead_of_silent() {
        let src = r#"{"formatVersion":"1.7","version":"1.7.0","children":[]}"#;
        let r = load_str(src).unwrap();
        assert!(r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. })));
    }

    #[test]
    fn current_minor_does_not_warn() {
        let src = r#"{"formatVersion":"1.1","version":"1.1.0","children":[]}"#;
        let r = load_str(src).unwrap();
        assert!(!r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. })));
    }

    #[test]
    fn promote_is_off_by_default_on_for_load_str_with() {
        let legacy = r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","role":"input","children":[]}]}"#;
        // Default load leaves the legacy frame as a frame.
        let r = load_str(legacy).unwrap();
        assert!(matches!(
            r.value.children[0],
            crate::node::PenNode::Frame(_)
        ));
        assert!(!r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::LegacyRolePromoted { .. })));
        // Opt-in load promotes it and reports a warning.
        let r2 = load_str_with(
            legacy,
            LoadOptions {
                promote_legacy_widgets: true,
            },
        )
        .unwrap();
        assert!(matches!(
            r2.value.children[0],
            crate::node::PenNode::TextInput(_)
        ));
        assert!(r2.warnings.iter().any(
            |w| matches!(w, LoadWarning::LegacyRolePromoted { to, .. } if *to == "text_input")
        ));
    }

    #[test]
    fn load_v2_is_rejected() {
        let s = r#"{"formatVersion":"2.0","version":"2","children":[]}"#;
        assert!(matches!(
            load_str(s),
            Err(OpsSchemaError::UnsupportedFormatVersion { .. })
        ));
    }

    #[test]
    fn responsive_below_1_2_warns_but_activates() {
        let src = r#"{"version":"1.1","formatVersion":"1.1","responsive":true,"children":[]}"#;
        let result = load_str(src).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|warning| matches!(warning, LoadWarning::ResponsiveBelowMinor { .. })));
        assert!(result.value.is_responsive());
    }
}
