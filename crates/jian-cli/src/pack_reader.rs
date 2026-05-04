//! `.op.pack` archive reader (Plan 19 D1 end-to-end wiring).
//!
//! Pulls the schema document plus optional AOT entries
//! (`aot/initial_layout.bin`, `aot/default_state.bin`) out of a
//! `.op.pack` zip so `jian player` can drive the cold-start path
//! against a pre-baked archive instead of a raw `.op`.
//!
//! The reader is intentionally narrow:
//!
//! - It does NOT extract every entry to disk (use `jian unpack` for
//!   that). It keeps the AOT bytes in memory because the runtime
//!   consumes them through `InitialLayoutSnapshot::read_bytes` /
//!   `DefaultStateSnapshot::read_bytes` and never needs the file.
//! - It does NOT consult the manifest's `entries` list (the manifest
//!   is informational; readers walk the zip directly because a
//!   manifest-truth check would just duplicate the zip's central
//!   directory). The manifest IS parsed enough to confirm
//!   `format == "op.pack"` so a stray non-Jian zip fails fast with
//!   a clear error.
//! - It does NOT handle font / image / logic entries — those are
//!   future-stage concerns wired in when the host's font provider /
//!   image cache / logic loader land.
//!
//! Zip-slip safety: every entry name is validated through the same
//! component-walk used by `commands::unpack::safe_entry_path`. A
//! truly malicious archive can't leak bytes outside our
//! `Vec<u8>`-shaped staging area, but defending the read step keeps
//! the surface uniform.

use anyhow::{anyhow, Context, Result};
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::pack::{
    AotManifest, DefaultStateSnapshot, ExpressionsSnapshot, InitialLayoutSnapshot,
    ENTRY_AOT_DEFAULT_STATE, ENTRY_AOT_EXPRESSIONS, ENTRY_AOT_INITIAL_LAYOUT, ENTRY_APP_OP,
    ENTRY_MANIFEST, PACK_FORMAT, PACK_FORMAT_VERSION,
};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// MeasureBackend tag the runtime preload accepts. `pack::manifest::
/// AotInventory::measurement_backend` records what the writer used;
/// a reader MUST reject when the tags disagree because text-bearing
/// rects baked under a different shaper would diverge from the live
/// render. Today's writer emits `"estimate"`
/// (`jian_core::layout::measure::EstimateBackend`); a future
/// `SkiaMeasure`-shaping host will broaden this list. Codex round 2
/// MEDIUM.
pub const READER_EXPECTED_BACKEND: &str = "estimate";

/// Everything `jian player` needs to drive cold-start from a
/// `.op.pack`. Snapshots are `Option` because every AOT entry is
/// independently optional in the format (a JSON-only pack carries
/// `app.op` alone).
#[derive(Debug)]
pub struct PackContents {
    /// Parsed `app.op` document.
    pub schema: PenDocument,
    /// Decoded `aot/initial_layout.bin`, if present and valid.
    pub initial_layout: Option<InitialLayoutSnapshot>,
    /// Decoded `aot/default_state.bin`, if present and valid.
    pub default_state: Option<DefaultStateSnapshot>,
    /// Decoded `aot/expressions.bin`, if present and valid (Plan 19
    /// D2). The bootstrap installs these into the runtime's
    /// `ExpressionCache` ahead of `SeedStateGraph` so binding
    /// evaluation hits pre-compiled bytecode without paying parse +
    /// compile.
    pub expressions: Option<ExpressionsSnapshot>,
}

/// Read a `.op.pack` archive and return its parsed contents. Errors
/// surface as `anyhow::Error` with context describing which entry
/// failed so a CLI user can map the diagnostic back to the archive.
///
/// Behaviour notes:
/// - A malformed `manifest.json` (e.g. wrong `format` field) errors
///   out — we don't fall back to a "best-effort" read because a
///   missing manifest is the OS-level signal "this isn't a Jian
///   pack" and silently treating other zips as packs would be a
///   surprise mode.
/// - A garbled AOT entry (bad magic, truncated, non-finite floats,
///   etc.) is treated as **non-fatal**: the snapshot becomes `None`
///   and a warning prints to stderr. The runtime then runs
///   `ComputeFirstLayout` / `SeedStateGraph` from the schema as it
///   would for a no-AOT pack. This matches the readers'
///   "fall back rather than misparse" contract.
pub fn read_op_pack(path: &Path) -> Result<PackContents> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut zr = zip::ZipArchive::new(file)
        .with_context(|| format!("read zip directory of {}", path.display()))?;

    // First pass: index entry name → zip-archive index. We do this
    // up front so the AOT readers don't have to walk the central
    // directory three times. `name()` borrows from the archive so
    // we copy into owned strings.
    //
    // Codex round 2 MEDIUM: a malicious or buggy archive can list
    // the same canonical name twice (e.g. two `app.op` entries).
    // `BTreeMap::insert` would silently keep the LATER index,
    // making `app.op` interpretation ambiguous. Reject duplicates
    // explicitly so the load fails loudly rather than picking one.
    let mut by_name: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for i in 0..zr.len() {
        let entry = zr.by_index(i)?;
        let name = entry.name().to_owned();
        // Reject zip-slip-style names up front. The reader doesn't
        // write to disk, but `safe_entry_path` also rejects empty /
        // NUL-byte / parent-directory names that could trip
        // downstream string matches.
        let _ = safe_entry_path(&name)
            .with_context(|| format!("unsafe entry name in archive: {name}"))?;
        if by_name.contains_key(&name) {
            return Err(anyhow!(
                "duplicate zip entry `{name}` — refusing to disambiguate"
            ));
        }
        by_name.insert(name, i);
    }

    // Manifest gate: a `.op.pack` MUST carry `manifest.json` typed
    // as `AotManifest` with `format == "op.pack"`, a recognised
    // `version`, and an `entries` list including `app.op` (codex
    // round 1 MEDIUM #5: previously we only checked `format` so
    // pack-version drift or entries-app-op-missing routed silently
    // into the hardcoded readers below).
    let manifest_idx = by_name
        .get(ENTRY_MANIFEST)
        .copied()
        .ok_or_else(|| anyhow!("`{ENTRY_MANIFEST}` missing — not a Jian .op.pack"))?;
    let manifest_bytes =
        read_entry_bytes(&mut zr, manifest_idx, ENTRY_MANIFEST, MANIFEST_MAX_BYTES)?;
    let manifest: AotManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("parse {ENTRY_MANIFEST}"))?;
    if manifest.format != PACK_FORMAT {
        return Err(anyhow!(
            "manifest `format` is `{}`, expected `{PACK_FORMAT}` — \
             not a Jian .op.pack archive",
            manifest.format
        ));
    }
    if manifest.version != PACK_FORMAT_VERSION {
        return Err(anyhow!(
            "manifest `version` is `{}`, this `jian` only reads `{PACK_FORMAT_VERSION}` \
             — re-pack with a matching `jian pack` build",
            manifest.version
        ));
    }
    if !manifest.entries.iter().any(|e| e == ENTRY_APP_OP) {
        return Err(anyhow!(
            "manifest `entries` does not include `{ENTRY_APP_OP}` — malformed pack"
        ));
    }

    // app.op — required. A pack without it is malformed regardless
    // of what `entries` claims.
    let app_op_idx = by_name
        .get(ENTRY_APP_OP)
        .copied()
        .ok_or_else(|| anyhow!("`{ENTRY_APP_OP}` missing from archive"))?;
    let app_op_bytes = read_entry_bytes(&mut zr, app_op_idx, ENTRY_APP_OP, APP_OP_MAX_BYTES)?;
    let app_op_text = std::str::from_utf8(&app_op_bytes)
        .with_context(|| format!("`{ENTRY_APP_OP}` is not UTF-8"))?;
    let schema = jian_ops_schema::load_str(app_op_text)
        .with_context(|| format!("parse `{ENTRY_APP_OP}`"))?
        .value;

    // AOT entries — optional. We only consume entries the manifest
    // inventories (codex round 1 MEDIUM #5: an orphan AOT file in
    // the zip but not in `manifest.entries` shouldn't drive the
    // runtime preload — it's a packing bug at the very least and
    // potentially a tampering signal). A garbled-but-inventoried
    // entry still warns + drops the snapshot rather than failing the
    // whole load, matching the snapshot reader's "fall back rather
    // than misparse" contract.
    // Codex round 2 MEDIUM: reject the layout snapshot when the
    // manifest's `aot.measurement_backend` is missing or doesn't
    // match `READER_EXPECTED_BACKEND`. Text-bearing rects baked
    // under a different shaper would disagree with the live render
    // — better to fall back to a fresh ComputeFirstLayout than to
    // serve mis-shaped geometry from a stale pack.
    let backend_ok = match manifest
        .aot
        .as_ref()
        .and_then(|a| a.measurement_backend.as_ref())
    {
        Some(tag) if tag == READER_EXPECTED_BACKEND => true,
        Some(other) => {
            eprintln!(
                "jian: warning — pack baked with measurement backend `{other}`, \
                 this reader expects `{READER_EXPECTED_BACKEND}`; falling back to \
                 ComputeFirstLayout"
            );
            false
        }
        None => {
            eprintln!(
                "jian: warning — pack manifest has no `measurement_backend` tag; \
                 falling back to ComputeFirstLayout"
            );
            false
        }
    };

    let layout_inventoried = backend_ok
        && manifest
            .entries
            .iter()
            .any(|e| e == ENTRY_AOT_INITIAL_LAYOUT);
    let initial_layout = if layout_inventoried {
        match by_name.get(ENTRY_AOT_INITIAL_LAYOUT) {
            Some(&idx) => {
                let bytes =
                    read_entry_bytes(&mut zr, idx, ENTRY_AOT_INITIAL_LAYOUT, AOT_LAYOUT_MAX_BYTES)?;
                match InitialLayoutSnapshot::read_bytes(&bytes) {
                    Ok(snap) => Some(snap),
                    Err(e) => {
                        eprintln!(
                            "jian: warning — `{ENTRY_AOT_INITIAL_LAYOUT}` decode failed ({e}); \
                             falling back to ComputeFirstLayout"
                        );
                        None
                    }
                }
            }
            None => {
                eprintln!(
                    "jian: warning — manifest lists `{ENTRY_AOT_INITIAL_LAYOUT}` but the \
                     entry is absent from the zip; falling back to ComputeFirstLayout"
                );
                None
            }
        }
    } else {
        None
    };
    let state_inventoried = manifest
        .entries
        .iter()
        .any(|e| e == ENTRY_AOT_DEFAULT_STATE);
    let default_state = if state_inventoried {
        match by_name.get(ENTRY_AOT_DEFAULT_STATE) {
            Some(&idx) => {
                let bytes =
                    read_entry_bytes(&mut zr, idx, ENTRY_AOT_DEFAULT_STATE, AOT_STATE_MAX_BYTES)?;
                match DefaultStateSnapshot::read_bytes(&bytes) {
                    Ok(snap) => Some(snap),
                    Err(e) => {
                        eprintln!(
                            "jian: warning — `{ENTRY_AOT_DEFAULT_STATE}` decode failed ({e}); \
                             falling back to schema-default state seed"
                        );
                        None
                    }
                }
            }
            None => {
                eprintln!(
                    "jian: warning — manifest lists `{ENTRY_AOT_DEFAULT_STATE}` but the \
                     entry is absent from the zip; falling back to schema-default state seed"
                );
                None
            }
        }
    } else {
        None
    };

    let expressions_inventoried = manifest.entries.iter().any(|e| e == ENTRY_AOT_EXPRESSIONS);
    let expressions = if expressions_inventoried {
        match by_name.get(ENTRY_AOT_EXPRESSIONS) {
            Some(&idx) => {
                let bytes =
                    read_entry_bytes(&mut zr, idx, ENTRY_AOT_EXPRESSIONS, AOT_EXPRS_MAX_BYTES)?;
                match ExpressionsSnapshot::read_bytes(&bytes) {
                    Ok(snap) => Some(snap),
                    Err(e) => {
                        eprintln!(
                            "jian: warning — `{ENTRY_AOT_EXPRESSIONS}` decode failed ({e}); \
                             falling back to JIT compile"
                        );
                        None
                    }
                }
            }
            None => {
                eprintln!(
                    "jian: warning — manifest lists `{ENTRY_AOT_EXPRESSIONS}` but the \
                     entry is absent from the zip; falling back to JIT compile"
                );
                None
            }
        }
    } else {
        None
    };

    Ok(PackContents {
        schema,
        initial_layout,
        default_state,
        expressions,
    })
}

/// True if `path` looks like a Jian pack (filename ends in
/// `.op.pack`, ASCII case-insensitive). Used by `jian player` to
/// branch between raw-`.op` and pack-archive load paths.
///
/// We deliberately don't probe the file contents here — opening the
/// zip is the player's job and `read_op_pack` returns a clear error
/// if the contents disagree. The extension check is a routing
/// hint, not a security boundary.
pub fn looks_like_op_pack(path: &Path) -> bool {
    let s = match path.to_str() {
        Some(s) => s,
        None => return false,
    };
    let lc = s.to_ascii_lowercase();
    lc.ends_with(".op.pack")
}

/// Returns entries in `snap` that don't satisfy
/// "key present in baseline AND value's JSON kind matches baseline's"
/// — i.e. either the key is unknown to the active schema (ghost
/// signal) OR the snapshot has type-drifted relative to what the
/// schema seeds (e.g. `count: int(0)` baseline vs `count: "0"`
/// snapshot, codex round 2 MEDIUM #1).
///
/// `baseline` is typically `Runtime::state.dump_default_state()`
/// captured immediately after `SeedStateGraph` ran, so it represents
/// "what the current schema seeds". An empty return value means the
/// snapshot's entries are subset-with-matching-types — safe to
/// restore. Otherwise the caller skips the restore and keeps the
/// schema-fresh seed.
///
/// Returns a list of human-readable `"scope.key (mismatch)"` strings
/// for the caller to log; the `(mismatch)` qualifier appears for
/// type-mismatched entries, plain `"scope.key"` for unknown keys.
pub fn snapshot_extra_keys(
    snap: &DefaultStateSnapshot,
    baseline: &DefaultStateSnapshot,
) -> Vec<String> {
    let mut extras = Vec::new();
    flag_map(&snap.app, &baseline.app, "$app", &mut extras);
    for (page, fields) in &snap.page {
        let empty_inner = std::collections::BTreeMap::new();
        let baseline_page = baseline.page.get(page).unwrap_or(&empty_inner);
        flag_map(fields, baseline_page, &format!("$page.{page}"), &mut extras);
    }
    for (node, fields) in &snap.self_node {
        let empty_inner = std::collections::BTreeMap::new();
        let baseline_self = baseline.self_node.get(node).unwrap_or(&empty_inner);
        flag_map(fields, baseline_self, &format!("$self.{node}"), &mut extras);
    }
    flag_map(&snap.route, &baseline.route, "$route", &mut extras);
    flag_map(&snap.storage, &baseline.storage, "$storage", &mut extras);
    flag_map(&snap.vars, &baseline.vars, "$vars", &mut extras);
    extras
}

fn flag_map(
    snap_map: &std::collections::BTreeMap<String, serde_json::Value>,
    baseline_map: &std::collections::BTreeMap<String, serde_json::Value>,
    scope_label: &str,
    extras: &mut Vec<String>,
) {
    for (k, v) in snap_map {
        match baseline_map.get(k) {
            None => extras.push(format!("{scope_label}.{k}")),
            Some(bv) if !same_json_kind(v, bv) => {
                extras.push(format!("{scope_label}.{k} (type mismatch)"));
            }
            Some(_) => {}
        }
    }
}

/// Recursive structural-kind compatibility for state values. Codex
/// round 2 + 3 MEDIUM: the outer-only check let nested type drift
/// pass (e.g. `{"x": 1}` vs `{"x": "one"}` — outer Object matches
/// but the inner `x` flipped Number→String). The recursive variant
/// walks Object (shared-key value-kind match; snapshot can NOT
/// introduce new nested keys without flagging) and Array (length +
/// element-wise kind) so leaf-level type drift is always caught.
///
/// Depth bound mirrors `pack::default_state::MAX_CANONICALIZE_DEPTH`
/// so a maliciously deeply-nested snapshot can't blow the stack
/// during validation.
fn same_json_kind(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    same_json_kind_at(a, b, 1)
}

const MAX_KIND_CHECK_DEPTH: usize = 64;

fn same_json_kind_at(a: &serde_json::Value, b: &serde_json::Value, depth: usize) -> bool {
    if depth > MAX_KIND_CHECK_DEPTH {
        // Too deep to safely check — treat as mismatch so the
        // surrounding skip-restore path triggers. Conservative
        // default; a depth bomb means the snapshot is suspect
        // regardless.
        return false;
    }
    use serde_json::Value::*;
    match (a, b) {
        (Null, Null) | (Bool(_), Bool(_)) | (Number(_), Number(_)) | (String(_), String(_)) => true,
        (Array(av), Array(bv)) => {
            if av.len() != bv.len() {
                return false;
            }
            av.iter()
                .zip(bv.iter())
                .all(|(x, y)| same_json_kind_at(x, y, depth + 1))
        }
        (Object(am), Object(bm)) => {
            // Codex round 4 MEDIUM: nested objects need EXACT key
            // match, not subset. `restore_default_state` overwrites
            // each top-level scope value wholesale (`app_set` /
            // etc.), so a stale snapshot whose nested object drops
            // a baseline key would silently lose that key from the
            // restored signal value. Both directions checked: every
            // baseline key must appear in the snapshot AND every
            // snapshot key must appear in the baseline.
            if am.len() != bm.len() {
                return false;
            }
            for (k, v) in am.iter() {
                let Some(bv) = bm.get(k) else {
                    return false;
                };
                if !same_json_kind_at(v, bv, depth + 1) {
                    return false;
                }
            }
            // Length parity above means every baseline key is
            // covered too — no need for a second pass.
            true
        }
        _ => false,
    }
}

/// Per-entry decompressed-size ceilings. Codex round 4 MEDIUM:
/// `read_to_end` would grow the buffer past any prealloc cap if the
/// compressed entry decompresses to gigabytes. We bound the read
/// itself via `Read::take(limit + 1)` and reject any entry that
/// fills the +1 slot. Limits picked to fit the largest realistic
/// document each entry needs to carry — single-digit MiB for AOT
/// payloads, double-digit for the canonical JSON document.
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024; //       1 MiB — manifest.json
const APP_OP_MAX_BYTES: u64 = 32 * 1024 * 1024; //   32 MiB — schema document
const AOT_LAYOUT_MAX_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB — initial_layout.bin
const AOT_STATE_MAX_BYTES: u64 = 8 * 1024 * 1024; //  8 MiB — default_state.bin
const AOT_EXPRS_MAX_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB — expressions.bin
                                                   //         (compiled bytecode + intern pools per source can
                                                   //         dominate a large doc; a tighter cap would push real
                                                   //         packs over the limit before format expansion lands)

/// Smaller prealloc cap than the per-entry limit so we don't
/// allocate up front against an attacker-declared size in the zip
/// header. The actual buffer grows via `read_to_end` up to the
/// per-entry limit, then refuses to grow further.
const PREALLOC_CAP: u64 = 1024 * 1024;

/// Read a single zip entry's bytes by index, bounded at
/// `max_bytes`. Wrapped so caller sites don't repeat the
/// read-into-Vec idiom + error context. Returns `Err` when the
/// entry decompresses past the limit so a maliciously inflated
/// pack can't drive the runtime to OOM.
fn read_entry_bytes(
    zr: &mut zip::ZipArchive<File>,
    index: usize,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let entry = zr
        .by_index(index)
        .with_context(|| format!("open entry `{name}`"))?;
    let prealloc = std::cmp::min(entry.size(), PREALLOC_CAP) as usize;
    let mut buf = Vec::with_capacity(prealloc);
    // Read at most `max_bytes + 1` so we can detect a "filled all
    // the way to the cap" case as overrun (the +1 byte couldn't fit
    // a real entry that respects the limit). Codex round 4 MEDIUM.
    let mut limited = entry.take(max_bytes + 1);
    limited
        .read_to_end(&mut buf)
        .with_context(|| format!("read entry `{name}`"))?;
    if buf.len() as u64 > max_bytes {
        return Err(anyhow!(
            "entry `{name}` exceeds {max_bytes}-byte limit; refusing to read further"
        ));
    }
    Ok(buf)
}

/// Component-walk variant of `commands::unpack::safe_entry_path`.
/// Used to validate every name in the central directory before we
/// trust it for indexed lookup. Mirrors the unpack guard so a future
/// change to the policy lands in one place.
fn safe_entry_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        return Err(anyhow!("empty entry name"));
    }
    if name.contains('\0') {
        return Err(anyhow!("entry name contains NUL byte"));
    }
    let p = Path::new(name);
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            Component::ParentDir => return Err(anyhow!("parent-directory component in `{name}`")),
            Component::RootDir => return Err(anyhow!("absolute path in `{name}`")),
            Component::Prefix(_) => return Err(anyhow!("drive / UNC prefix in `{name}`")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(anyhow!("entry `{name}` has no normal components"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::pack::manifest::DefaultViewport;
    use jian_ops_schema::pack::{InitialLayoutSnapshot, PackedRect};
    use std::collections::BTreeMap;
    use std::io::Write;

    const FIXTURE_OP: &str = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "pkr-fix",
      "app": { "name": "PkrFix", "version": "1", "id": "pkr.fix" },
      "state": { "count": { "type": "int", "default": 7 } },
      "children": [
        { "type": "frame", "id": "root", "width": 320, "height": 240, "x": 0, "y": 0,
          "children": [
            { "type": "rectangle", "id": "btn", "x": 16, "y": 16, "width": 100, "height": 40 }
          ]
        }
      ]
    }"##;

    fn write_minimal_pack(
        path: &Path,
        with_layout: Option<&InitialLayoutSnapshot>,
        with_state: Option<&DefaultStateSnapshot>,
    ) {
        write_pack_full(path, with_layout, with_state, None);
    }

    fn write_pack_full(
        path: &Path,
        with_layout: Option<&InitialLayoutSnapshot>,
        with_state: Option<&DefaultStateSnapshot>,
        with_exprs: Option<&ExpressionsSnapshot>,
    ) {
        let file = std::fs::File::create(path).expect("create pack file");
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let mut entries: Vec<&str> = vec![ENTRY_APP_OP];
        if with_layout.is_some() {
            entries.push(ENTRY_AOT_INITIAL_LAYOUT);
        }
        if with_state.is_some() {
            entries.push(ENTRY_AOT_DEFAULT_STATE);
        }
        if with_exprs.is_some() {
            entries.push(ENTRY_AOT_EXPRESSIONS);
        }
        let mut manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": { "id": "pkr.fix", "name": "PkrFix", "version": "1" },
            "capabilities": [],
            "entries": entries,
        });
        // The reader requires an `aot` block with a recognised
        // `measurement_backend` before it consumes
        // `aot/initial_layout.bin`. Mirror what `jian pack --aot`
        // emits today so the test fixture round-trips through both
        // gates.
        if with_layout.is_some() || with_state.is_some() {
            let mut aot = serde_json::Map::new();
            if with_layout.is_some() {
                aot.insert(
                    "initial_layout".into(),
                    serde_json::Value::String(ENTRY_AOT_INITIAL_LAYOUT.into()),
                );
                aot.insert(
                    "default_viewport".into(),
                    serde_json::json!({ "width": 320.0, "height": 240.0 }),
                );
                aot.insert(
                    "measurement_backend".into(),
                    serde_json::Value::String(READER_EXPECTED_BACKEND.into()),
                );
            }
            if with_state.is_some() {
                aot.insert(
                    "default_state".into(),
                    serde_json::Value::String(ENTRY_AOT_DEFAULT_STATE.into()),
                );
            }
            if with_exprs.is_some() {
                aot.insert(
                    "expressions".into(),
                    serde_json::Value::String(ENTRY_AOT_EXPRESSIONS.into()),
                );
            }
            manifest
                .as_object_mut()
                .unwrap()
                .insert("aot".into(), serde_json::Value::Object(aot));
        }

        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        zw.write_all(serde_json::to_vec_pretty(&manifest).unwrap().as_slice())
            .unwrap();

        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();

        if let Some(snap) = with_layout {
            zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts).unwrap();
            zw.write_all(&snap.write_bytes().unwrap()).unwrap();
        }
        if let Some(snap) = with_state {
            zw.start_file(ENTRY_AOT_DEFAULT_STATE, opts).unwrap();
            zw.write_all(&snap.write_bytes().unwrap()).unwrap();
        }
        if let Some(snap) = with_exprs {
            zw.start_file(ENTRY_AOT_EXPRESSIONS, opts).unwrap();
            zw.write_all(&snap.write_bytes().unwrap()).unwrap();
        }
        zw.finish().unwrap();
    }

    #[test]
    fn extension_detection_case_insensitive() {
        assert!(looks_like_op_pack(Path::new("app.op.pack")));
        assert!(looks_like_op_pack(Path::new("APP.OP.PACK")));
        assert!(looks_like_op_pack(Path::new("/abs/path/foo.op.pack")));
        assert!(!looks_like_op_pack(Path::new("app.op")));
        assert!(!looks_like_op_pack(Path::new("foo.zip")));
        // Trailing-suffix variant — `.op.pack.bak` is NOT a pack we
        // can route automatically.
        assert!(!looks_like_op_pack(Path::new("app.op.pack.bak")));
    }

    #[test]
    fn read_pack_with_no_aot_returns_schema_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("plain.op.pack");
        write_minimal_pack(&pack_path, None, None);

        let contents = read_op_pack(&pack_path).expect("read");
        assert!(contents.initial_layout.is_none());
        assert!(contents.default_state.is_none());
        assert_eq!(contents.schema.id.as_deref(), Some("pkr-fix"));
    }

    #[test]
    fn read_pack_with_aot_round_trips_both_snapshots() {
        let mut rects = BTreeMap::new();
        rects.insert(
            "root".to_owned(),
            PackedRect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 240.0,
            },
        );
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 16.0,
                y: 16.0,
                w: 100.0,
                h: 40.0,
            },
        );
        let layout_snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };
        let mut state_snap = DefaultStateSnapshot::default();
        state_snap.app.insert("count".into(), serde_json::json!(7));

        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("withaot.op.pack");
        write_minimal_pack(&pack_path, Some(&layout_snap), Some(&state_snap));

        let contents = read_op_pack(&pack_path).expect("read");
        let layout = contents.initial_layout.expect("layout present");
        assert_eq!(layout.viewport.width, 320.0);
        assert_eq!(layout.rects.get("btn").unwrap().w, 100.0);
        let state = contents.default_state.expect("state present");
        assert_eq!(state.app.get("count").unwrap(), &serde_json::json!(7));
        // Without an `expressions` entry in the manifest the reader
        // returns `None` for that slot — gating exactly mirrors the
        // existing layout / state slot.
        assert!(contents.expressions.is_none());
    }

    fn nonempty_exprs_snap() -> ExpressionsSnapshot {
        use jian_ops_schema::pack::{PackedChunk, PackedOpCode};
        let mut entries = BTreeMap::new();
        entries.insert(
            "$app.count + 1".to_owned(),
            PackedChunk {
                ops: vec![
                    PackedOpCode::PushScopeRef(0),
                    PackedOpCode::PushNum(1.0),
                    PackedOpCode::Add,
                    PackedOpCode::Return,
                ],
                strings: vec![],
                scope_paths: vec!["$app.count".into()],
            },
        );
        ExpressionsSnapshot { entries }
    }

    #[test]
    fn read_pack_with_expressions_round_trips_snapshot() {
        // End-to-end: writer-shape pack with `aot/expressions.bin`
        // round-trips back through the reader and the
        // PackContents::expressions slot carries the same chunk we
        // wrote.
        let exprs = nonempty_exprs_snap();
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("withexprs.op.pack");
        write_pack_full(&pack_path, None, None, Some(&exprs));

        let contents = read_op_pack(&pack_path).expect("read");
        let snap = contents.expressions.expect("exprs present");
        assert_eq!(snap.len(), 1);
        let chunk = snap.entries.get("$app.count + 1").expect("source key");
        assert_eq!(chunk.scope_paths, vec!["$app.count".to_owned()]);
        assert_eq!(chunk.ops.len(), 4);
    }

    #[test]
    fn read_pack_with_garbled_expressions_drops_to_none() {
        // A reader that hits a corrupt `aot/expressions.bin` must
        // warn + drop the snapshot, matching the layout/state
        // "fall back rather than misparse" contract. The runtime
        // then JIT-compiles every binding source on first
        // evaluation.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("garbled.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"pkr.fix","name":"PkrFix","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP, ENTRY_AOT_EXPRESSIONS],
            "aot": { "expressions": ENTRY_AOT_EXPRESSIONS },
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.start_file(ENTRY_AOT_EXPRESSIONS, opts).unwrap();
        zw.write_all(b"NOPE-not-an-OPE1-frame").unwrap();
        zw.finish().unwrap();

        let contents = read_op_pack(&pack_path).expect("read still succeeds");
        assert!(
            contents.expressions.is_none(),
            "garbled expressions.bin must drop to None"
        );
    }

    #[test]
    fn read_pack_skips_uninventoried_expressions_entry() {
        // An orphan `aot/expressions.bin` file in the zip but NOT
        // listed in `manifest.entries` MUST NOT drive the runtime
        // preload — that's the same gate every other AOT entry
        // honours. Mirrors the codex round 1 MEDIUM #5 fix on the
        // layout slot.
        let exprs = nonempty_exprs_snap();
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("orphan.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        // Manifest deliberately omits ENTRY_AOT_EXPRESSIONS from
        // `entries` even though we'll write the file below.
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"pkr.fix","name":"PkrFix","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP],
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.start_file(ENTRY_AOT_EXPRESSIONS, opts).unwrap();
        zw.write_all(&exprs.write_bytes().unwrap()).unwrap();
        zw.finish().unwrap();

        let contents = read_op_pack(&pack_path).expect("read");
        assert!(
            contents.expressions.is_none(),
            "uninventoried expressions.bin MUST NOT be consumed"
        );
    }

    #[test]
    fn read_pack_rejects_zip_without_manifest() {
        // Hand-build a zip with only `app.op` — no manifest.json. The
        // reader must refuse rather than treat it as a Jian pack.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("nomanifest.zip");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.finish().unwrap();

        let err = read_op_pack(&pack_path).unwrap_err();
        assert!(err.to_string().contains("manifest.json"), "{err}");
    }

    #[test]
    fn read_pack_rejects_wrong_format_field() {
        // Build a manifest with format != "op.pack". The reader
        // should refuse — that's the OS-level signal "this isn't a
        // Jian pack" and silently treating it as one would surprise.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("wrongformat.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let bad = serde_json::json!({
            "format": "some-other-zip",
            "version": "0.1",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP],
        });
        zw.write_all(serde_json::to_vec(&bad).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.finish().unwrap();

        let err = read_op_pack(&pack_path).unwrap_err();
        assert!(err.to_string().contains("op.pack"), "{err}");
    }

    #[test]
    fn read_pack_rejects_unsupported_version() {
        // Codex round 1 MEDIUM: a `.op.pack` whose manifest declares
        // a future / unknown `version` must refuse to load rather
        // than silently route into the current hardcoded readers.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("future.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "99.0",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP],
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.finish().unwrap();

        let err = read_op_pack(&pack_path).unwrap_err();
        assert!(err.to_string().contains("99.0"), "{err}");
    }

    #[test]
    fn read_pack_rejects_manifest_entries_missing_app_op() {
        // Codex round 1 MEDIUM: the manifest's `entries` list is
        // load-bearing — callers downstream of the manifest may
        // route off it. Refuse a manifest that doesn't declare
        // `app.op`, even if the zip happens to contain a copy.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("badentries.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": ["random.bin"],
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.finish().unwrap();

        let err = read_op_pack(&pack_path).unwrap_err();
        assert!(err.to_string().contains(ENTRY_APP_OP), "{err}");
    }

    #[test]
    fn read_pack_skips_uninventoried_aot_entries() {
        // Codex round 1 MEDIUM: an orphan `aot/initial_layout.bin`
        // present in the zip but not listed in `manifest.entries`
        // must NOT drive the runtime preload — it's a packing bug
        // at minimum, a tampering signal at worst.
        let mut rects = BTreeMap::new();
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        );
        let layout_snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };

        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("orphan.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        // Manifest deliberately omits the AOT entry from `entries`.
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP],
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        // Add an orphan aot/initial_layout.bin — present in zip,
        // absent from manifest.entries.
        zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts).unwrap();
        zw.write_all(&layout_snap.write_bytes().unwrap()).unwrap();
        zw.finish().unwrap();

        let contents = read_op_pack(&pack_path).expect("loads cleanly");
        assert!(
            contents.initial_layout.is_none(),
            "uninventoried AOT entry must NOT drive the preload"
        );
    }

    #[test]
    fn snapshot_extra_keys_detects_stale_state_snapshot() {
        // Snapshot carries `count` (in baseline) + `legacy` (NOT in
        // baseline). The extras list should report only `legacy`.
        let mut baseline = DefaultStateSnapshot::default();
        baseline.app.insert("count".into(), serde_json::json!(0));
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("count".into(), serde_json::json!(7));
        snap.app.insert("legacy".into(), serde_json::json!("ghost"));

        let extras = snapshot_extra_keys(&snap, &baseline);
        assert_eq!(extras, vec!["$app.legacy"]);
    }

    #[test]
    fn snapshot_extra_keys_flags_type_mismatch() {
        // Codex round 2 MEDIUM: a snapshot whose value's JSON kind
        // differs from baseline (e.g. baseline `count: int(0)`,
        // snapshot `count: "0"`) must be flagged for skip-restore,
        // even when the key itself is shared.
        let mut baseline = DefaultStateSnapshot::default();
        baseline.app.insert("count".into(), serde_json::json!(0));
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("count".into(), serde_json::json!("0"));

        let extras = snapshot_extra_keys(&snap, &baseline);
        assert_eq!(extras.len(), 1);
        assert!(extras[0].contains("type mismatch"), "{}", extras[0]);
    }

    #[test]
    fn read_pack_drops_layout_when_backend_mismatch() {
        // Codex round 2 MEDIUM: a manifest whose
        // `aot.measurement_backend` doesn't match the reader's
        // expected backend tag must NOT drive the runtime preload.
        // A pack baked under a different shaper would land mis-
        // shaped rects on the live render.
        let mut rects = BTreeMap::new();
        rects.insert(
            "btn".to_owned(),
            PackedRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        );
        let layout_snap = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects,
        };

        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("wrong_backend.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP, ENTRY_AOT_INITIAL_LAYOUT],
            "aot": {
                "initial_layout": ENTRY_AOT_INITIAL_LAYOUT,
                "default_viewport": { "width": 320.0, "height": 240.0 },
                "measurement_backend": "future_skia_shaper",
            },
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts).unwrap();
        zw.write_all(&layout_snap.write_bytes().unwrap()).unwrap();
        zw.finish().unwrap();

        let contents = read_op_pack(&pack_path).expect("loads cleanly");
        assert!(
            contents.initial_layout.is_none(),
            "backend-mismatched layout snapshot must drop to None"
        );
    }

    // NOTE: a "duplicate canonical name in zip central directory"
    // test isn't expressible here — the `zip` 2.x writer refuses
    // `start_file` on a name it has already emitted (returns
    // `InvalidArchive("Duplicate filename")`). The reader's
    // by_name `contains_key`-then-`insert` guard still defends
    // against a zip hand-crafted at the byte level (a real-world
    // tampering scenario that the `zip` crate's read path passes
    // through), so the code path stays even without a test
    // fixture.

    #[test]
    fn snapshot_extra_keys_flags_nested_type_drift() {
        // Codex round 3 MEDIUM: outer-Object match used to mask
        // inner type flips like `{"x": int}` → `{"x": "string"}`.
        // The recursive variant catches them.
        let mut baseline = DefaultStateSnapshot::default();
        baseline
            .app
            .insert("user".into(), serde_json::json!({"x": 1}));
        let mut snap = DefaultStateSnapshot::default();
        snap.app
            .insert("user".into(), serde_json::json!({"x": "one"}));

        let extras = snapshot_extra_keys(&snap, &baseline);
        assert_eq!(extras.len(), 1);
        assert!(extras[0].contains("type mismatch"));
    }

    #[test]
    fn snapshot_extra_keys_flags_nested_extra_object_key() {
        // A nested object with a key the baseline doesn't carry is
        // also flagged — schema-versioned drift could add new
        // sub-fields the runtime isn't ready for.
        let mut baseline = DefaultStateSnapshot::default();
        baseline
            .app
            .insert("user".into(), serde_json::json!({"name": "x"}));
        let mut snap = DefaultStateSnapshot::default();
        snap.app
            .insert("user".into(), serde_json::json!({"name": "x", "ghost": 9}));

        assert!(!snapshot_extra_keys(&snap, &baseline).is_empty());
    }

    #[test]
    fn snapshot_extra_keys_flags_nested_object_baseline_extra_key() {
        // Codex round 4 MEDIUM: a stale snapshot dropping a key the
        // baseline carries (`{name, role}` baseline vs `{name}`
        // snapshot) used to pass under subset semantics, then the
        // top-level scope `set` would overwrite the whole object
        // and lose `role`. With exact-match nested-Object kind
        // checks, this case is now flagged.
        let mut baseline = DefaultStateSnapshot::default();
        baseline.app.insert(
            "user".into(),
            serde_json::json!({"name":"x","role":"guest"}),
        );
        let mut snap = DefaultStateSnapshot::default();
        snap.app
            .insert("user".into(), serde_json::json!({"name":"old"}));

        assert!(
            !snapshot_extra_keys(&snap, &baseline).is_empty(),
            "stale snapshot dropping a nested-object key must be flagged"
        );
    }

    #[test]
    fn snapshot_extra_keys_array_length_mismatch_is_drift() {
        let mut baseline = DefaultStateSnapshot::default();
        baseline
            .app
            .insert("items".into(), serde_json::json!([1, 2, 3]));
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("items".into(), serde_json::json!([1, 2]));

        assert!(!snapshot_extra_keys(&snap, &baseline).is_empty());
    }

    #[test]
    fn snapshot_extra_keys_subset_returns_empty() {
        // A snapshot strictly ⊆ baseline returns no extras —
        // safe to restore.
        let mut baseline = DefaultStateSnapshot::default();
        baseline.app.insert("count".into(), serde_json::json!(0));
        baseline.app.insert("name".into(), serde_json::json!("x"));
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("count".into(), serde_json::json!(7));

        assert!(snapshot_extra_keys(&snap, &baseline).is_empty());
    }

    #[test]
    fn read_pack_with_garbled_aot_layout_drops_to_none() {
        // Write a manifest + app.op + a deliberately-garbled
        // initial_layout.bin (wrong magic). The reader's contract is
        // to fall back to None + a stderr warning, not fail the
        // whole pack load.
        let dir = tempfile::TempDir::new().unwrap();
        let pack_path = dir.path().join("garbledaot.op.pack");
        let file = std::fs::File::create(&pack_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file(ENTRY_MANIFEST, opts).unwrap();
        let manifest = serde_json::json!({
            "format": PACK_FORMAT,
            "version": "0.1",
            "app": {"id":"x","name":"x","version":"1"},
            "capabilities": [],
            "entries": [ENTRY_APP_OP, ENTRY_AOT_INITIAL_LAYOUT],
        });
        zw.write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        zw.start_file(ENTRY_APP_OP, opts).unwrap();
        zw.write_all(FIXTURE_OP.as_bytes()).unwrap();
        zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts).unwrap();
        zw.write_all(b"NOT_A_REAL_OPL1_SNAPSHOT").unwrap();
        zw.finish().unwrap();

        let contents = read_op_pack(&pack_path).expect("pack still loads");
        assert!(
            contents.initial_layout.is_none(),
            "garbled snapshot must drop to None, not poison the load"
        );
        assert_eq!(contents.schema.id.as_deref(), Some("pkr-fix"));
    }
}
