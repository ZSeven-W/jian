use super::{RuntimeHandle, State};
use jian_core::action::services::{NullRouter, RouteState, Router};
use jian_core::runtime::FrameDirective;
use jian_core::screens::{reconcile_screens_with_layout, ScreenRouter, ScreenTable};
use std::cell::RefCell;
use std::rc::Rc;

pub(super) struct NavigationState {
    router: Rc<ScreenRouter>,
    table: ScreenTable,
    current_path: String,
}

pub(super) fn pump_runtime(
    state: &Rc<RefCell<State>>,
    runtime: &RuntimeHandle,
    now: u64,
) -> FrameDirective {
    reconcile_navigation(state, runtime);
    let mut live = runtime.take();
    let mut directive = live.pump(now);
    let route_pending = state
        .borrow()
        .navigation
        .as_ref()
        .is_some_and(|nav| nav.router.current().path != nav.current_path);
    runtime.put(live);
    if route_pending {
        reconcile_navigation(state, runtime);
        let mut live = runtime.take();
        directive = live.pump(now);
        runtime.put(live);
    }
    directive
}

pub(super) fn refresh_navigation(state: &Rc<RefCell<State>>) {
    let (runtime, previous) = {
        let mut host = state.borrow_mut();
        if host.disposed {
            return;
        }
        (host.runtime.clone(), host.navigation.take())
    };
    let mut live = runtime.take();
    let Some(table) = live.screen_table() else {
        live.nav = Rc::new(NullRouter);
        runtime.put(live);
        return;
    };
    let paths = table.paths();
    let active_path = live
        .active_screen_path()
        .unwrap_or(table.entry_path())
        .to_owned();
    let router = if let Some(previous) = previous {
        let saved = previous.router.current();
        previous.router.restore(saved, &paths);
        previous.router
    } else {
        let router = Rc::new(ScreenRouter::new(table.entry_path(), paths.clone()));
        if active_path != table.entry_path() {
            router.restore(
                RouteState {
                    path: active_path.clone(),
                    params: Default::default(),
                    query: Default::default(),
                    stack: vec![active_path.clone()],
                },
                &paths,
            );
        }
        router
    };
    live.nav = router.clone();
    let mut navigation = NavigationState {
        router,
        table,
        current_path: active_path,
    };
    match reconcile_screens_with_layout(
        &mut live,
        &navigation.router,
        &navigation.table,
        &mut navigation.current_path,
    ) {
        Ok(outcome) => {
            for rejected in outcome.rejections {
                live.push_load_warning(format!(
                    "unknown route `{}` ({}) ignored",
                    rejected.path, rejected.verb
                ));
            }
            if outcome.switched.is_some() {
                live.mark_dirty();
            }
        }
        Err(error) => live.push_layout_error(format!("route refresh failed: {error}")),
    }
    runtime.put(live);
    let mut host = state.borrow_mut();
    if !host.disposed {
        host.navigation = Some(navigation);
    }
}

fn reconcile_navigation(state: &Rc<RefCell<State>>, runtime: &RuntimeHandle) {
    let Some(mut navigation) = ({
        let mut host = state.borrow_mut();
        (!host.disposed).then(|| host.navigation.take()).flatten()
    }) else {
        return;
    };
    let mut switched = false;
    let mut live = runtime.take();
    live.note_time(state.borrow().now_ms());
    match reconcile_screens_with_layout(
        &mut live,
        &navigation.router,
        &navigation.table,
        &mut navigation.current_path,
    ) {
        Ok(outcome) => {
            for rejected in outcome.rejections {
                live.push_load_warning(format!(
                    "unknown route `{}` ({}) ignored",
                    rejected.path, rejected.verb
                ));
            }
            if outcome.switched.is_some() {
                switched = true;
                live.mark_dirty();
            }
        }
        Err(error) => live.push_layout_error(format!("route reconcile failed: {error}")),
    }
    runtime.put(live);
    let mut host = state.borrow_mut();
    if !host.disposed {
        if switched {
            // Route projection replaces the document warning vector, so its
            // host cursor belongs to the previous document generation.
            host.warning_count = 0;
        }
        host.navigation = Some(navigation);
    }
}
