//! Screen projection: flatten explicitly-marked top-level frames
//! (`FrameNode.screen = "/path"`) from every page into one synthetic
//! page per screen + a derived `RoutesConfig`, entry screen at
//! `pages[0]`. Pure; returns `None` when no valid marker exists so
//! callers keep today's single-page behavior for old files.

use crate::breakpoint::BreakpointRange;
use crate::document::PenDocument;
use crate::node::PenNode;
use crate::page::PenPage;
use crate::routes::{RouteSpec, RoutesConfig};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionWarning {
    MarkerIgnored {
        node_id: String,
        reason: String,
    },
    DuplicatePath {
        path: String,
        node_id: String,
    },
    NoEntryScreen {
        fallback_node_id: String,
    },
    AuthoredRoutesIgnored,
    InvalidRangeStripped {
        node_id: String,
    },
    DuplicateDefault {
        path: String,
        node_id: String,
    },
    PromotedDefault {
        path: String,
        page_id: String,
    },
    InteriorOverlap {
        path: String,
        first: String,
        second: String,
    },
    BreakpointWithoutScreen {
        node_id: String,
    },
    PageIdRekeyed {
        from: String,
        to: String,
    },
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
            Self::InvalidRangeStripped { node_id } => {
                write!(f, "invalid breakpoint on `{node_id}` stripped")
            }
            Self::DuplicateDefault { path, node_id } => {
                write!(f, "duplicate default for `{path}` on `{node_id}` ignored")
            }
            Self::PromotedDefault { path, page_id } => {
                write!(f, "variant `{page_id}` promoted to default for `{path}`")
            }
            Self::InteriorOverlap {
                path,
                first,
                second,
            } => {
                write!(
                    f,
                    "breakpoint variants `{first}` and `{second}` overlap on `{path}`"
                )
            }
            Self::BreakpointWithoutScreen { node_id } => {
                write!(f, "breakpoint on `{node_id}` ignored without a screen path")
            }
            Self::PageIdRekeyed { from, to } => {
                write!(f, "page id `{from}` re-keyed to `{to}`")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantEntry {
    pub range: BreakpointRange,
    pub page_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenVariants {
    pub default_page_id: String,
    pub ranged: Vec<VariantEntry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenVariantTable(pub BTreeMap<String, ScreenVariants>);

/// One collected screen: the marked frame (cloned, x/y zeroed) + path.
struct Screen {
    path: String,
    frame: PenNode,
    id: String,
    name: String,
    range: Option<BreakpointRange>,
    order: usize,
    normalized_id: Option<String>,
    stripped_range: bool,
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
    responsive: bool,
) {
    for node in children {
        let PenNode::Frame(frame) = node else {
            continue;
        };
        let Some(path) = frame.screen.as_deref() else {
            if responsive && frame.breakpoint.is_some() {
                warnings.push(ProjectionWarning::BreakpointWithoutScreen {
                    node_id: frame.base.id.clone(),
                });
            }
            continue;
        };
        if !path.starts_with('/') || path.is_empty() {
            warnings.push(ProjectionWarning::MarkerIgnored {
                node_id: frame.base.id.clone(),
                reason: format!("path `{path}` must start with '/'"),
            });
            continue;
        }
        if !responsive && seen_paths.contains_key(path) {
            warnings.push(ProjectionWarning::DuplicatePath {
                path: path.to_owned(),
                node_id: frame.base.id.clone(),
            });
            continue;
        }
        seen_paths
            .entry(path.to_owned())
            .or_insert_with(|| frame.base.id.clone());
        let mut stripped_range = false;
        let range = if responsive {
            frame.breakpoint.and_then(|range| {
                if range.validate().is_ok() {
                    Some(range)
                } else {
                    stripped_range = true;
                    warnings.push(ProjectionWarning::InvalidRangeStripped {
                        node_id: frame.base.id.clone(),
                    });
                    None
                }
            })
        } else {
            None
        };
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
            range,
            order: screens.len(),
            normalized_id: None,
            stripped_range,
        });
    }
}

/// Project every explicitly `screen`-marked top-level frame across all
/// pages (or top-level `children` for pageless documents) into a single
/// synthesized `pages` array (one page per screen, entry first) plus a
/// derived `routes` table. Returns `None` when no valid marker exists,
/// so callers can fall back to today's single-page/single-doc behavior.
pub fn project_screens(
    doc: &PenDocument,
) -> (
    Option<(PenDocument, ScreenVariantTable)>,
    Vec<ProjectionWarning>,
) {
    let mut warnings = Vec::new();
    let mut screens: Vec<Screen> = Vec::new();
    let mut seen_paths: BTreeMap<String, String> = BTreeMap::new(); // path -> node id

    match &doc.pages {
        Some(pages) if !pages.is_empty() => {
            for page in pages {
                collect_screens(
                    &page.children,
                    &mut warnings,
                    &mut screens,
                    &mut seen_paths,
                    doc.is_responsive(),
                );
            }
        }
        _ => collect_screens(
            &doc.children,
            &mut warnings,
            &mut screens,
            &mut seen_paths,
            doc.is_responsive(),
        ),
    }

    if screens.is_empty() {
        return (None, warnings);
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
    if doc.is_responsive() {
        normalize_screen_ids(&mut screens, &mut warnings);
    }
    let table = if doc.is_responsive() {
        build_variant_table(&mut screens, &mut warnings)
    } else {
        ScreenVariantTable::default()
    };
    screens.sort_by_key(|screen| screen.path != entry_path);

    if doc.routes.is_some() {
        warnings.push(ProjectionWarning::AuthoredRoutesIgnored);
    }

    let mut routes = BTreeMap::new();
    let mut pages = Vec::with_capacity(screens.len());
    for s in screens {
        let page_id = variant_page_id(&s);
        routes.entry(s.path.clone()).or_insert_with(|| RouteSpec {
            page_id: table.0.get(&s.path).map_or_else(
                || page_id.clone(),
                |variants| variants.default_page_id.clone(),
            ),
            preload: None,
            guards: None,
            params: None,
        });
        pages.push(PenPage {
            id: page_id,
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
    (Some((out, table)), warnings)
}

fn render_bound(value: Option<f64>, open: &str) -> String {
    value.map_or_else(
        || open.to_owned(),
        |value| {
            let normalized = if value == 0.0 { 0.0 } else { value };
            normalized.to_string()
        },
    )
}

fn variant_page_id(screen: &Screen) -> String {
    if let Some(id) = &screen.normalized_id {
        return id.clone();
    }
    match screen.range {
        Some(range) => format!(
            "{}@{}-{}",
            screen.id,
            render_bound(range.min_width.or(Some(0.0)), "0"),
            render_bound(range.max_width, "inf")
        ),
        None => screen.id.clone(),
    }
}

fn normalize_screen_ids(screens: &mut [Screen], warnings: &mut Vec<ProjectionWarning>) {
    let mut pages: Vec<PenPage> = screens
        .iter()
        .map(|screen| PenPage {
            id: variant_page_id(screen),
            name: screen.name.clone(),
            children: Vec::new(),
            state: None,
            lifecycle: None,
        })
        .collect();
    let mut routes = RoutesConfig {
        entry: String::new(),
        routes: BTreeMap::new(),
        transitions: None,
    };
    crate::page_ids::normalize_page_ids(&mut pages, &mut routes, warnings);
    for (screen, page) in screens.iter_mut().zip(pages) {
        screen.normalized_id = Some(page.id);
    }
}

fn build_variant_table(
    screens: &mut [Screen],
    warnings: &mut Vec<ProjectionWarning>,
) -> ScreenVariantTable {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, screen) in screens.iter().enumerate() {
        groups.entry(screen.path.clone()).or_default().push(index);
    }
    let mut table = BTreeMap::new();
    for (path, indices) in groups {
        let ranged: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| screens[*index].range.is_some())
            .collect();
        for (position, &left) in ranged.iter().enumerate() {
            let a = screens[left].range.unwrap();
            for &right in &ranged[position + 1..] {
                let b = screens[right].range.unwrap();
                if a.min_width.unwrap_or(0.0) < b.max_width.unwrap_or(f64::INFINITY)
                    && b.min_width.unwrap_or(0.0) < a.max_width.unwrap_or(f64::INFINITY)
                {
                    warnings.push(ProjectionWarning::InteriorOverlap {
                        path: path.clone(),
                        first: variant_page_id(&screens[left]),
                        second: variant_page_id(&screens[right]),
                    });
                }
            }
        }
        let authored_defaults: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| screens[*index].range.is_none() && !screens[*index].stripped_range)
            .collect();
        let stripped_defaults: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| screens[*index].range.is_none() && screens[*index].stripped_range)
            .collect();
        let default_index = if let Some((&first, extras)) = authored_defaults.split_first() {
            for &extra in extras.iter().chain(stripped_defaults.iter()) {
                warnings.push(ProjectionWarning::DuplicateDefault {
                    path: path.clone(),
                    node_id: screens[extra].id.clone(),
                });
            }
            first
        } else if let Some((&first, extras)) = stripped_defaults.split_first() {
            for &extra in extras {
                warnings.push(ProjectionWarning::DuplicateDefault {
                    path: path.clone(),
                    node_id: screens[extra].id.clone(),
                });
            }
            first
        } else {
            let promoted = *ranged
                .iter()
                .min_by(|&&left, &&right| {
                    screens[left]
                        .range
                        .unwrap()
                        .min_width
                        .unwrap_or(0.0)
                        .total_cmp(&screens[right].range.unwrap().min_width.unwrap_or(0.0))
                        .then_with(|| screens[left].order.cmp(&screens[right].order))
                })
                .expect("group has at least one screen");
            warnings.push(ProjectionWarning::PromotedDefault {
                path: path.clone(),
                page_id: variant_page_id(&screens[promoted]),
            });
            promoted
        };
        let mut entries: Vec<(usize, VariantEntry)> = ranged
            .into_iter()
            .filter(|index| *index != default_index)
            .map(|index| {
                (
                    screens[index].order,
                    VariantEntry {
                        range: screens[index].range.unwrap(),
                        page_id: variant_page_id(&screens[index]),
                    },
                )
            })
            .collect();
        entries.sort_by(|(left_order, left), (right_order, right)| {
            left.range
                .min_width
                .unwrap_or(0.0)
                .total_cmp(&right.range.min_width.unwrap_or(0.0))
                .then_with(|| left_order.cmp(right_order))
        });
        table.insert(
            path,
            ScreenVariants {
                default_page_id: variant_page_id(&screens[default_index]),
                ranged: entries.into_iter().map(|(_, entry)| entry).collect(),
            },
        );
    }
    ScreenVariantTable(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> PenDocument {
        serde_json::from_str(json).unwrap()
    }

    fn responsive_doc(children: &str) -> PenDocument {
        doc(&format!(
            r#"{{"version":"1.2","responsive":true,"children":{children}}}"#
        ))
    }

    #[test]
    fn variants_group_promote_and_render_canonical_ids() {
        let document = responsive_doc(
            r#"[
              {"type":"frame","id":"home","screen":"/home","width":1280,"height":800},
              {"type":"frame","id":"home-m","screen":"/home","width":390,"height":800,
               "breakpoint":{"minWidth":-0.0,"maxWidth":480}},
              {"type":"frame","id":"home-t","screen":"/home","width":800,"height":800,
               "breakpoint":{"minWidth":480.5,"maxWidth":1024.0}}]"#,
        );
        let (out, warnings) = project_screens(&document);
        let (_, table) = out.unwrap();
        let variants = &table.0["/home"];
        assert_eq!(variants.default_page_id, "home");
        assert_eq!(variants.ranged[0].page_id, "home-m@0-480");
        assert_eq!(variants.ranged[1].page_id, "home-t@480.5-1024");
        assert_eq!(
            warnings
                .iter()
                .filter(|warning| !matches!(warning, ProjectionWarning::NoEntryScreen { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn only_ranged_promotes_smallest_min_and_keeps_range_id() {
        let document = responsive_doc(
            r#"[
              {"type":"frame","id":"a","screen":"/x","breakpoint":{"minWidth":0,"maxWidth":480}},
              {"type":"frame","id":"b","screen":"/x","breakpoint":{"minWidth":481}}]"#,
        );
        let (out, warnings) = project_screens(&document);
        let (_, table) = out.unwrap();
        let variants = &table.0["/x"];
        assert_eq!(variants.default_page_id, "a@0-480");
        assert_eq!(variants.ranged.len(), 1);
        assert!(warnings
            .iter()
            .any(|warning| matches!(warning, ProjectionWarning::PromotedDefault { .. })));
    }

    #[test]
    fn invalid_strip_overlap_duplicate_default_and_orphan_warn() {
        let document = responsive_doc(
            r#"[
              {"type":"frame","id":"orphan","breakpoint":{"minWidth":0}},
              {"type":"frame","id":"a","screen":"/y"},
              {"type":"frame","id":"bad","screen":"/y","breakpoint":{"minWidth":500,"maxWidth":480}},
              {"type":"frame","id":"c","screen":"/y","breakpoint":{"minWidth":0,"maxWidth":300}},
              {"type":"frame","id":"d","screen":"/y","breakpoint":{"minWidth":200,"maxWidth":400}}]"#,
        );
        let (_, warnings) = project_screens(&document);
        assert!(warnings
            .iter()
            .any(|warning| matches!(warning, ProjectionWarning::InvalidRangeStripped { .. })));
        assert!(warnings
            .iter()
            .any(|warning| matches!(warning, ProjectionWarning::DuplicateDefault { .. })));
        assert!(warnings
            .iter()
            .any(|warning| matches!(warning, ProjectionWarning::InteriorOverlap { .. })));
        assert!(warnings
            .iter()
            .any(|warning| matches!(warning, ProjectionWarning::BreakpointWithoutScreen { .. })));
    }

    #[test]
    fn authored_default_wins_when_invalid_stripped_candidate_comes_first() {
        let document = responsive_doc(
            r#"[
              {"type":"frame","id":"bad","screen":"/","breakpoint":{"minWidth":500,"maxWidth":400}},
              {"type":"frame","id":"authored","screen":"/"}]"#,
        );
        let (projected, _) = project_screens(&document);
        let (_, table) = projected.unwrap();
        assert_eq!(table.0["/"].default_page_id, "authored");
    }

    #[test]
    fn no_markers_returns_none() {
        let d = doc(r#"{"version":"1.0","children":[{"type":"frame","id":"a"}]}"#);
        assert!(project_screens(&d).0.is_none());
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
        let (projected, warnings) = project_screens(&d);
        let (out, _) = projected.unwrap();
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
        let (projected, warnings) = project_screens(&d);
        let (out, _) = projected.unwrap();
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
        let (projected, warnings) = project_screens(&d);
        let (out, _) = projected.unwrap();
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
        let (projected, warnings) = project_screens(&d);
        let (out, _) = projected.unwrap();
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
        let (projected, _) = project_screens(&d);
        let (out, _) = projected.unwrap();
        assert_eq!(out.pages.as_ref().unwrap()[0].id, "solo");
    }
}
