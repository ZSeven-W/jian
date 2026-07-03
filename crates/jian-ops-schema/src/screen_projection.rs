//! Screen projection: flatten explicitly-marked top-level frames
//! (`FrameNode.screen = "/path"`) from every page into one synthetic
//! page per screen + a derived `RoutesConfig`, entry screen at
//! `pages[0]`. Pure; returns `None` when no valid marker exists so
//! callers keep today's single-page behavior for old files.

use crate::document::PenDocument;
use crate::node::PenNode;
use crate::page::PenPage;
use crate::routes::{RouteSpec, RoutesConfig};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionWarning {
    MarkerIgnored { node_id: String, reason: String },
    DuplicatePath { path: String, node_id: String },
    NoEntryScreen { fallback_node_id: String },
    AuthoredRoutesIgnored,
}

impl std::fmt::Display for ProjectionWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MarkerIgnored { node_id, reason } => {
                write!(f, "screen marker on `{node_id}` ignored: {reason}")
            }
            Self::DuplicatePath { path, node_id } => {
                write!(
                    f,
                    "duplicate screen path `{path}` on `{node_id}` ignored (first wins)"
                )
            }
            Self::NoEntryScreen { fallback_node_id } => {
                write!(
                    f,
                    "no screen marked \"/\"; `{fallback_node_id}` used as entry"
                )
            }
            Self::AuthoredRoutesIgnored => {
                write!(
                    f,
                    "authored `routes` ignored: screen markers are the route source"
                )
            }
        }
    }
}

/// One collected screen: the marked frame (cloned, x/y zeroed) + path.
struct Screen {
    path: String,
    frame: PenNode,
    id: String,
    name: String,
}

/// Scan one slice of top-level nodes for `FrameNode.screen` markers,
/// appending valid ones to `screens` and invalid/duplicate ones as
/// warnings. Non-frame nodes and unmarked frames are silently skipped
/// (they are excluded from the projected document).
fn collect_screens(
    children: &[PenNode],
    warnings: &mut Vec<ProjectionWarning>,
    screens: &mut Vec<Screen>,
    seen_paths: &mut BTreeMap<String, String>,
) {
    for node in children {
        let PenNode::Frame(frame) = node else {
            continue;
        };
        let Some(path) = frame.screen.as_deref() else {
            continue;
        };
        if !path.starts_with('/') || path.is_empty() {
            warnings.push(ProjectionWarning::MarkerIgnored {
                node_id: frame.base.id.clone(),
                reason: format!("path `{path}` must start with '/'"),
            });
            continue;
        }
        if seen_paths.contains_key(path) {
            warnings.push(ProjectionWarning::DuplicatePath {
                path: path.to_owned(),
                node_id: frame.base.id.clone(),
            });
            continue;
        }
        seen_paths.insert(path.to_owned(), frame.base.id.clone());
        let mut mounted = frame.clone();
        mounted.base.x = None;
        mounted.base.y = None;
        screens.push(Screen {
            path: path.to_owned(),
            id: frame.base.id.clone(),
            name: frame
                .base
                .name
                .clone()
                .unwrap_or_else(|| frame.base.id.clone()),
            frame: PenNode::Frame(mounted),
        });
    }
}

/// Project every explicitly `screen`-marked top-level frame across all
/// pages (or top-level `children` for pageless documents) into a single
/// synthesized `pages` array (one page per screen, entry first) plus a
/// derived `routes` table. Returns `None` when no valid marker exists,
/// so callers can fall back to today's single-page/single-doc behavior.
pub fn project_screens(doc: &PenDocument) -> Option<(PenDocument, Vec<ProjectionWarning>)> {
    let mut warnings = Vec::new();
    let mut screens: Vec<Screen> = Vec::new();
    let mut seen_paths: BTreeMap<String, String> = BTreeMap::new(); // path -> node id

    match &doc.pages {
        Some(pages) if !pages.is_empty() => {
            for page in pages {
                collect_screens(&page.children, &mut warnings, &mut screens, &mut seen_paths);
            }
        }
        _ => collect_screens(&doc.children, &mut warnings, &mut screens, &mut seen_paths),
    }

    if screens.is_empty() {
        return None; // Old files / unmarked docs: caller keeps today's path.
    }

    // Entry = "/" if present, else first collected marker (+ warning).
    let entry_path = if screens.iter().any(|s| s.path == "/") {
        "/".to_owned()
    } else {
        warnings.push(ProjectionWarning::NoEntryScreen {
            fallback_node_id: screens[0].id.clone(),
        });
        screens[0].path.clone()
    };
    // Entry screen's synthetic page must sit at pages[0] (jian's loader
    // mounts pages[0]; the reconcile glue relies on this convention).
    screens.sort_by_key(|s| s.path != entry_path);

    if doc.routes.is_some() {
        warnings.push(ProjectionWarning::AuthoredRoutesIgnored);
    }

    let mut routes = BTreeMap::new();
    let mut pages = Vec::with_capacity(screens.len());
    for s in screens {
        routes.insert(
            s.path.clone(),
            RouteSpec {
                page_id: s.id.clone(),
                preload: None,
                guards: None,
                params: None,
            },
        );
        pages.push(PenPage {
            id: s.id,
            name: s.name,
            children: vec![s.frame],
            state: None,
            lifecycle: None,
        });
    }

    let mut out = doc.clone();
    out.pages = Some(pages);
    out.children = Vec::new();
    out.routes = Some(RoutesConfig {
        entry: entry_path,
        routes,
        transitions: None,
    });
    Some((out, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> PenDocument {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn no_markers_returns_none() {
        let d = doc(r#"{"version":"1.0","children":[{"type":"frame","id":"a"}]}"#);
        assert!(project_screens(&d).is_none());
    }

    #[test]
    fn markers_across_pages_flatten_with_entry_first() {
        let d = doc(r#"{"version":"1.0","pages":[
              {"id":"p1","name":"P1","children":[
                {"type":"frame","id":"detail","x":900,"y":40,"screen":"/detail"},
                {"type":"frame","id":"note"}]},
              {"id":"p2","name":"P2","children":[
                {"type":"frame","id":"home","x":10,"y":20,"screen":"/"}]}
            ]}"#);
        let (out, warnings) = project_screens(&d).unwrap();
        let pages = out.pages.as_ref().unwrap();
        // Entry screen page first; one synthetic page per marked frame;
        // unmarked "note" frame excluded.
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].id, "home");
        assert_eq!(pages[1].id, "detail");
        // Frame x/y zeroed on mount.
        match &pages[0].children[0] {
            PenNode::Frame(f) => {
                assert_eq!(f.base.x, None);
                assert_eq!(f.base.y, None);
            }
            other => panic!("expected frame, got {other:?}"),
        }
        // Derived routes: entry "/" -> home, "/detail" -> detail.
        let routes = out.routes.as_ref().unwrap();
        assert_eq!(routes.entry, "/");
        assert_eq!(routes.routes.get("/").unwrap().page_id, "home");
        assert_eq!(routes.routes.get("/detail").unwrap().page_id, "detail");
        assert!(warnings.is_empty());
    }

    #[test]
    fn invalid_and_duplicate_paths_warn() {
        let d = doc(
            r#"{"version":"1.0","pages":[{"id":"p","name":"P","children":[
              {"type":"frame","id":"a","screen":"/"},
              {"type":"frame","id":"bad","screen":"nope"},
              {"type":"frame","id":"dup","screen":"/"}
            ]}]}"#,
        );
        let (out, warnings) = project_screens(&d).unwrap();
        assert_eq!(out.pages.as_ref().unwrap().len(), 1); // only "a" mounts
        assert!(warnings.iter().any(
            |w| matches!(w, ProjectionWarning::MarkerIgnored { node_id, .. } if node_id == "bad")
        ));
        assert!(warnings.iter().any(
            |w| matches!(w, ProjectionWarning::DuplicatePath { node_id, .. } if node_id == "dup")
        ));
    }

    #[test]
    fn missing_entry_falls_back_to_first_marker() {
        let d = doc(
            r#"{"version":"1.0","pages":[{"id":"p","name":"P","children":[
              {"type":"frame","id":"only","screen":"/detail"}
            ]}]}"#,
        );
        let (out, warnings) = project_screens(&d).unwrap();
        assert_eq!(out.routes.as_ref().unwrap().entry, "/detail");
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ProjectionWarning::NoEntryScreen { .. })));
    }

    #[test]
    fn authored_routes_are_ignored_with_warning_and_doc_state_survives() {
        let d = doc(r#"{"version":"1.0",
              "state":{"count":{"type":"int","default":1}},
              "routes":{"entry":"/","routes":{"/":{"pageId":"p"}}},
              "pages":[{"id":"p","name":"P","children":[
                {"type":"frame","id":"a","screen":"/"}]}]}"#);
        let (out, warnings) = project_screens(&d).unwrap();
        assert!(warnings
            .iter()
            .any(|w| matches!(w, ProjectionWarning::AuthoredRoutesIgnored)));
        // Synthesized table replaces the authored one.
        assert_eq!(
            out.routes
                .as_ref()
                .unwrap()
                .routes
                .get("/")
                .unwrap()
                .page_id,
            "a"
        );
        // Document-level shared fields survive projection.
        assert!(out.state.is_some());
    }

    #[test]
    fn pageless_document_top_level_markers_project() {
        let d = doc(r#"{"version":"1.0","children":[
              {"type":"frame","id":"solo","screen":"/"}]}"#);
        let (out, _) = project_screens(&d).unwrap();
        assert_eq!(out.pages.as_ref().unwrap()[0].id, "solo");
    }
}
