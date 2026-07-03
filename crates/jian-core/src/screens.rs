//! Screen routing glue: a validating `Router` + the per-frame
//! reconcile helper that swaps the mounted screen when the route
//! changes. Shared by the OP editor preview and `jian player` so both
//! run the same code path.

use crate::action::services::{RouteState, Router};
use jian_ops_schema::PenDocument;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// A navigation the router refused because the target path is not a
/// known screen. Drained by `reconcile_screens` and surfaced as a
/// host-side warning ('Router' has no warn hook of its own).
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedNav {
    pub verb: &'static str,
    pub path: String,
}

/// Route-table-validating router: mutates only toward known screen
/// paths, so the mounted-screen reconcile never has to roll back.
pub struct ScreenRouter {
    known: BTreeSet<String>,
    stack: RefCell<Vec<String>>,
    /// Accumulates until drained by `take_rejections()`.
    /// `reconcile_screens` is the designated drainer — it calls
    /// `take_rejections()` once per pass so hosts see every rejected
    /// nav exactly once, even across frames where the tip didn't move.
    rejections: RefCell<Vec<RejectedNav>>,
}

impl ScreenRouter {
    pub fn new(entry: &str, known: impl IntoIterator<Item = String>) -> Self {
        Self {
            known: known.into_iter().collect(),
            stack: RefCell::new(vec![entry.to_owned()]),
            rejections: RefCell::new(Vec::new()),
        }
    }

    pub fn take_rejections(&self) -> Vec<RejectedNav> {
        std::mem::take(&mut self.rejections.borrow_mut())
    }

    fn check(&self, verb: &'static str, path: &str) -> bool {
        if self.known.contains(path) {
            true
        } else {
            self.rejections
                .borrow_mut()
                .push(RejectedNav {
                    verb,
                    path: path.to_owned(),
                });
            false
        }
    }
}

impl Router for ScreenRouter {
    fn current(&self) -> RouteState {
        let stack = self.stack.borrow();
        RouteState {
            path: stack.last().cloned().unwrap_or_else(|| "/".to_owned()),
            params: Default::default(),
            query: Default::default(),
            stack: stack.clone(),
        }
    }

    fn push(&self, path: &str) {
        if self.check("push", path) {
            self.stack.borrow_mut().push(path.to_owned());
        }
    }

    fn replace(&self, path: &str) {
        if self.check("replace", path) {
            let mut s = self.stack.borrow_mut();
            if let Some(last) = s.last_mut() {
                *last = path.to_owned();
            } else {
                s.push(path.to_owned());
            }
        }
    }

    fn pop(&self) {
        let mut s = self.stack.borrow_mut();
        if s.len() > 1 {
            s.pop();
        }
    }

    fn reset(&self, path: &str) {
        if self.check("reset", path) {
            let mut s = self.stack.borrow_mut();
            s.clear();
            s.push(path.to_owned());
        }
    }
}

/// Route table + the normalized multi-screen document it was derived
/// from. `doc_for_path` clones out a single-page document for
/// `Runtime::replace_document` (the target page becomes `pages[0]`).
pub struct ScreenTable {
    doc: PenDocument,
    routes: BTreeMap<String, String>, // path -> synthetic page id
    entry: String,
}

impl ScreenTable {
    /// Build from a normalized document (as emitted by
    /// `jian_ops_schema::screen_projection::project_screens`). Returns
    /// `None` when the document carries no routes config.
    pub fn from_document(doc: PenDocument) -> Option<Self> {
        let cfg = doc.routes.clone()?;
        let routes = cfg
            .routes
            .iter()
            .map(|(path, spec)| (path.clone(), spec.page_id.clone()))
            .collect();
        Some(Self {
            doc,
            routes,
            entry: cfg.entry,
        })
    }

    pub fn entry_path(&self) -> &str {
        &self.entry
    }

    pub fn paths(&self) -> Vec<String> {
        self.routes.keys().cloned().collect()
    }

    /// Index of the path's synthetic page inside the normalized doc.
    pub fn page_index(&self, path: &str) -> Option<usize> {
        let page_id = self.routes.get(path)?;
        self.doc.pages.as_ref()?.iter().position(|p| &p.id == page_id)
    }

    /// Single-page document for mounting `path`'s screen.
    pub fn doc_for_path(&self, path: &str) -> Option<PenDocument> {
        let idx = self.page_index(path)?;
        let page = self.doc.pages.as_ref()?.get(idx)?.clone();
        let mut out = self.doc.clone();
        out.pages = Some(vec![page]);
        Some(out)
    }
}

/// Result of one reconcile pass.
pub struct ReconcileOutcome {
    /// `Some(new_path)` when the mounted screen changed this pass.
    pub switched: Option<String>,
    /// Unknown-path navigations refused since the previous pass.
    pub rejections: Vec<RejectedNav>,
}

/// Compare the router tip against the mounted screen; on divergence
/// swap the mounted document (`$app` preserved via the loader's
/// `SeedMode::PreserveExisting`) and sync `$route.path` / `$route.stack`.
/// Hosts call this once per frame at their paint entry.
///
/// Deliberately does NOT re-run layout — hosts own layout (OP solves
/// per-root; the player hook calls `build_layout` + `rebuild_spatial`
/// itself after this returns).
pub fn reconcile_screens(
    runtime: &mut crate::Runtime,
    router: &std::rc::Rc<ScreenRouter>,
    table: &ScreenTable,
    current_path: &mut String,
) -> crate::CoreResult<ReconcileOutcome> {
    let rejections = router.take_rejections();
    let state = router.current();
    if state.path == *current_path {
        return Ok(ReconcileOutcome {
            switched: None,
            rejections,
        });
    }
    let Some(doc) = table.doc_for_path(&state.path) else {
        // Validated router should make this unreachable; stay put.
        return Ok(ReconcileOutcome {
            switched: None,
            rejections,
        });
    };
    runtime.replace_document(doc)?;
    runtime
        .state
        .route_set("path", serde_json::json!(state.path));
    runtime
        .state
        .route_set("stack", serde_json::json!(state.stack));
    *current_path = state.path.clone();
    Ok(ReconcileOutcome {
        switched: Some(state.path),
        rejections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_path_push_mutates_stack() {
        let r = ScreenRouter::new("/", ["/".to_owned(), "/detail".to_owned()]);
        r.push("/detail");
        assert_eq!(r.current().path, "/detail");
        assert_eq!(r.current().stack, vec!["/", "/detail"]);
        assert!(r.take_rejections().is_empty());
    }

    #[test]
    fn unknown_path_does_not_mutate_and_records_rejection() {
        let r = ScreenRouter::new("/", ["/".to_owned()]);
        r.push("/nope");
        r.replace("/also-nope");
        assert_eq!(r.current().path, "/");
        let rej = r.take_rejections();
        assert_eq!(rej.len(), 2);
        assert_eq!(rej[0].verb, "push");
        assert_eq!(rej[0].path, "/nope");
        assert!(r.take_rejections().is_empty(), "take drains");
    }

    #[test]
    fn pop_never_empties_stack() {
        let r = ScreenRouter::new("/", ["/".to_owned()]);
        r.pop();
        assert_eq!(r.current().path, "/");
    }

    fn two_screen_doc() -> jian_ops_schema::PenDocument {
        // Normalized shape (as project_screens emits): entry page first,
        // routes derived, screens at origin. Home has a nav button
        // (events.onTap push — expression string literal, escaped quotes)
        // and a bound input; detail has a pop button.
        serde_json::from_str(
            r#"{
          "version":"1.0",
          "routes":{"entry":"/","routes":{"/":{"pageId":"home"},"/detail":{"pageId":"detail"}}},
          "pages":[
            {"id":"home","name":"Home","children":[
              {"type":"frame","id":"home-root","width":400,"height":300,"screen":"/","children":[
                {"type":"text_input","id":"email","width":200,"height":32,
                 "bindings":{"bind:value":"$state.email"}},
                {"type":"frame","id":"go","width":120,"height":40,
                 "semantics":{"role":"button"},
                 "events":{"onTap":[{"push":"\"/detail\""}]}}
              ]}]},
            {"id":"detail","name":"Detail","children":[
              {"type":"frame","id":"detail-root","width":400,"height":300,"screen":"/detail","children":[
                {"type":"frame","id":"back","width":120,"height":40,
                 "semantics":{"role":"button"},
                 "events":{"onTap":[{"pop":null}]}}
              ]}]}
          ]}"#,
        )
        .unwrap()
    }

    /// Locate node `"go"`'s solved rect center — copies the lookup
    /// pattern from `tap_toggles_switch_and_syncs_state_graph`
    /// (`runtime.rs:1548`).
    fn go_center(rt: &crate::Runtime) -> (f32, f32) {
        let key = rt.document.as_ref().unwrap().tree.get("go").unwrap();
        let r = rt.layout.node_rect(key).expect("go button laid out");
        (r.min_x() + r.size.width / 2.0, r.min_y() + r.size.height / 2.0)
    }

    #[test]
    fn reconcile_switches_screen_and_preserves_app_state() {
        use crate::gesture::pointer::{PointerEvent, PointerPhase};
        use crate::geometry::point;
        use std::rc::Rc;

        let doc = two_screen_doc();
        let table = ScreenTable::from_document(doc.clone()).unwrap();
        let router = Rc::new(ScreenRouter::new(table.entry_path(), table.paths()));
        let mut rt = crate::Runtime::new_from_document(doc).unwrap();
        rt.nav = router.clone();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.rebuild_spatial();
        let mut current = table.entry_path().to_owned();

        // Durable state written on the home screen.
        rt.state.app_set("email", serde_json::json!("a@b.c"));

        // Tap the nav button (Down + Up over its center) -> push action.
        let tap = |rt: &mut crate::Runtime, x: f32, y: f32| {
            rt.dispatch_pointer(PointerEvent::simple(0, PointerPhase::Down, point(x, y)));
            rt.dispatch_pointer(PointerEvent::simple(0, PointerPhase::Up, point(x, y)));
        };
        let (cx, cy) = go_center(&rt);
        tap(&mut rt, cx, cy);

        let outcome = reconcile_screens(&mut rt, &router, &table, &mut current).unwrap();
        assert_eq!(outcome.switched.as_deref(), Some("/detail"));
        assert_eq!(current, "/detail");
        // $app survived the swap; $route synced.
        assert_eq!(
            rt.state.app_get("email").unwrap().0,
            serde_json::json!("a@b.c")
        );
        let snap_path = rt.state.dump_default_state().route.get("path").cloned();
        assert_eq!(snap_path, Some(serde_json::json!("/detail")));
        // Mounted tree is now the detail screen.
        let roots = &rt.document.as_ref().unwrap().tree.roots;
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn unknown_push_reports_rejection_and_stays() {
        use std::rc::Rc;
        let doc = two_screen_doc();
        let table = ScreenTable::from_document(doc.clone()).unwrap();
        let router = Rc::new(ScreenRouter::new(table.entry_path(), table.paths()));
        let mut rt = crate::Runtime::new_from_document(doc).unwrap();
        rt.nav = router.clone();
        let mut current = table.entry_path().to_owned();
        router.push("/missing");
        let outcome = reconcile_screens(&mut rt, &router, &table, &mut current).unwrap();
        assert!(outcome.switched.is_none());
        assert_eq!(outcome.rejections.len(), 1);
        assert_eq!(current, "/");
    }
}
