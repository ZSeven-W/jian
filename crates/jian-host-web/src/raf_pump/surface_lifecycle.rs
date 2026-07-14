//! Viewport metrics, CanvasKit surface ownership, and WebGL context lifecycle.

use super::{request_frame, State};
use crate::viewport::ViewportMetrics;
use jian_core::geometry::size;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;

pub(super) fn apply_metrics(state: &Rc<RefCell<State>>, metrics: ViewportMetrics) {
    let (runtime, logical_changed, dpr_changed, surface_changed, dropped_surface) = {
        let mut host = state.borrow_mut();
        if host.disposed {
            return;
        }
        host.connected = metrics.connected;
        let logical_changed = host.logical != (metrics.width, metrics.height);
        let dpr_changed = (host.dpr - metrics.dpr.max(1.0)).abs() > 0.001;
        host.logical = (metrics.width, metrics.height);
        host.dpr = metrics.dpr.max(1.0);
        if !metrics.connected || metrics.width <= 0.0 || metrics.height <= 0.0 {
            (
                host.runtime.clone(),
                false,
                false,
                false,
                host.surface.take(),
            )
        } else {
            let physical = (
                (metrics.width * host.dpr).round().max(1.0) as u32,
                (metrics.height * host.dpr).round().max(1.0) as u32,
            );
            let surface_changed =
                host.physical != physical || dpr_changed || host.surface.is_none();
            host.physical = physical;
            (
                host.runtime.clone(),
                logical_changed,
                dpr_changed,
                surface_changed,
                None,
            )
        }
    };
    drop(dropped_surface);
    if logical_changed {
        let mut live = runtime.take();
        if let Some(target) = live.needs_variant_swap(metrics.width) {
            live.set_viewport_size_without_relayout((metrics.width, metrics.height));
            if let Err(error) = live.switch_variant(&target) {
                live.push_layout_error(format!("breakpoint variant swap failed: {error}"));
                if let Err(error) = live.relayout() {
                    live.push_layout_error(format!("breakpoint fallback relayout failed: {error}"));
                }
            }
        } else {
            live.set_viewport_size((metrics.width, metrics.height));
        }
        runtime.put(live);
    } else if dpr_changed {
        runtime.mark_dirty();
    }
    let surface_ready = if surface_changed {
        recreate_surface(state)
    } else {
        state.borrow().paintable()
    };
    if surface_ready && (logical_changed || dpr_changed || surface_changed) {
        request_frame(state);
    }
}

pub(super) fn recreate_surface(state: &Rc<RefCell<State>>) -> bool {
    let Some((mut backend, dpr, physical, generation, old_surface)) = ({
        let mut host = state.borrow_mut();
        let old_surface = host.surface.take();
        if host.disposed
            || !host.connected
            || host.logical.0 <= 0.0
            || host.logical.1 <= 0.0
            || host.context_lost
        {
            None
        } else {
            host.backend.take().map(|backend| {
                (
                    backend,
                    host.dpr,
                    host.physical,
                    host.backend_generation,
                    old_surface,
                )
            })
        }
    }) else {
        return false;
    };
    drop(old_surface);
    backend.set_dpr(dpr);
    let surface = match backend.try_new_surface(size(physical.0 as f32, physical.1 as f32)) {
        Ok(surface) => surface,
        Err(error) => {
            if state.borrow().backend_generation != generation {
                backend.invalidate_images();
            }
            let report = {
                let mut host = state.borrow_mut();
                if host.disposed {
                    None
                } else {
                    host.backend = Some(backend);
                    host.surface = None;
                    Some((host.runtime.clone(), host.callbacks.clone()))
                }
            };
            if let Some((runtime, callbacks)) = report {
                runtime.mark_dirty();
                callbacks.surface_error(surface_error_message(&error));
            }
            return false;
        }
    };
    let generation_changed = state.borrow().backend_generation != generation;
    if generation_changed {
        backend.invalidate_images();
    }
    let mut surface = Some(surface);
    let installed = {
        let mut host = state.borrow_mut();
        if host.disposed {
            false
        } else {
            host.backend = Some(backend);
            let current = host.connected
                && host.logical.0 > 0.0
                && host.logical.1 > 0.0
                && !host.context_lost
                && host.backend_generation == generation
                && host.physical == physical
                && (host.dpr - dpr).abs() <= 0.001;
            if current {
                host.surface = surface.take();
                true
            } else {
                false
            }
        }
    };
    drop(surface);
    installed
}

pub(super) fn context_lost(state: &Rc<RefCell<State>>) {
    let (runtime, surface, backend, generation) = {
        let mut host = state.borrow_mut();
        if host.disposed || host.context_lost {
            return;
        }
        host.context_lost = true;
        host.backend_generation = host.backend_generation.wrapping_add(1);
        (
            host.runtime.clone(),
            host.surface.take(),
            host.backend.take(),
            host.backend_generation,
        )
    };
    drop(surface);
    if let Some(mut backend) = backend {
        backend.invalidate_images();
        let reinstall = {
            let mut host = state.borrow_mut();
            if host.disposed || host.backend_generation != generation {
                false
            } else {
                host.backend = Some(backend);
                !host.context_lost && host.connected && host.logical.0 > 0.0 && host.logical.1 > 0.0
            }
        };
        if reinstall {
            let _ = recreate_surface(state);
        }
    }
    runtime.mark_dirty();
}

pub(super) fn context_restored(state: &Rc<RefCell<State>>) {
    let runtime = {
        let mut host = state.borrow_mut();
        if host.disposed || !host.context_lost {
            return;
        }
        host.context_lost = false;
        host.runtime.clone()
    };
    runtime.mark_dirty();
    if recreate_surface(state) {
        request_frame(state);
    }
}

fn surface_error_message(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| format!("CanvasKit surface creation failed: {error:?}"))
}
