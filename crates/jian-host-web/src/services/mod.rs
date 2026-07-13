//! Browser implementations of Jian's fallible host-service traits.

mod clipboard;
mod image_resolver;
pub(crate) mod network;
pub mod platform;
mod storage;

pub(crate) use image_resolver::{AssetPolicy, WebImageResolver};

use jian_core::Runtime;
use js_sys::{Array, Function, Object, Promise};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use web_sys::{AbortController, AbortSignal};

type DeferredCleanup = Box<dyn FnOnce()>;

#[derive(Clone, Default)]
pub(super) struct BrowserCleanup {
    defer_depth: Rc<Cell<u32>>,
    pending: Rc<RefCell<Vec<DeferredCleanup>>>,
}

impl BrowserCleanup {
    pub(super) fn begin(&self) {
        self.defer_depth
            .set(self.defer_depth.get().saturating_add(1));
    }

    pub(super) fn finish(&self) {
        let depth = self.defer_depth.get();
        debug_assert!(depth > 0, "deferred browser cleanup was not active");
        self.defer_depth.set(depth.saturating_sub(1));
        if self.defer_depth.get() != 0 {
            return;
        }
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        for cleanup in pending {
            cleanup();
        }
    }

    pub(super) fn run(&self, cleanup: impl FnOnce() + 'static) {
        if self.defer_depth.get() == 0 {
            cleanup();
        } else {
            self.pending.borrow_mut().push(Box::new(cleanup));
        }
    }
}

pub(crate) async fn await_with_wake(
    promise: Promise,
    wake: &Rc<dyn Fn()>,
) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
    let observer = promise.clone();
    let settle_wake = wake.clone();
    let cancelled = Object::new();
    let cancel_resolve = Rc::new(RefCell::new(None::<Function>));
    let cancel_promise = {
        let cancel_resolve = cancel_resolve.clone();
        Promise::new(&mut move |resolve, _reject| {
            *cancel_resolve.borrow_mut() = Some(resolve);
        })
    };
    let race_inputs = Array::new();
    race_inputs.push(&observer);
    race_inputs.push(&cancel_promise);
    let race = Promise::race(&race_inputs);
    let observer_cancelled = cancelled.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(race).await {
            Ok(value) if Object::is(&value, observer_cancelled.as_ref()) => {}
            _ => settle_wake(),
        }
    });
    let lease = PromiseObserverLease {
        resolve: cancel_resolve.borrow_mut().take(),
        cancelled,
    };
    let result = wasm_bindgen_futures::JsFuture::from(promise).await;
    drop(lease);
    result
}

/// Cancels the detached settlement observer when its owning service future is
/// dropped. The browser Promise itself may be unabortable (clipboard is the
/// important case), but no host wake future is allowed to outlive disposal.
struct PromiseObserverLease {
    resolve: Option<Function>,
    cancelled: Object,
}

impl Drop for PromiseObserverLease {
    fn drop(&mut self) {
        if let Some(resolve) = self.resolve.take() {
            let _ = resolve.call1(&wasm_bindgen::JsValue::UNDEFINED, self.cancelled.as_ref());
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct AbortRegistry {
    next: Rc<Cell<u64>>,
    controllers: Rc<RefCell<BTreeMap<u64, AbortController>>>,
    cleanup: BrowserCleanup,
}

impl AbortRegistry {
    pub(crate) fn lease(&self) -> Result<AbortLease, wasm_bindgen::JsValue> {
        let id = self.next.get().wrapping_add(1).max(1);
        self.next.set(id);
        let controller = AbortController::new()?;
        self.controllers.borrow_mut().insert(id, controller.clone());
        Ok(AbortLease {
            registry: self.clone(),
            id,
            controller: Some(controller),
        })
    }

    pub(crate) fn abort_all(&self) {
        let controllers = {
            let mut guard = self.controllers.borrow_mut();
            std::mem::take(&mut *guard)
        };
        for (_, controller) in controllers {
            self.cleanup.run(move || controller.abort());
        }
    }

    pub(crate) fn cleanup(&self) -> BrowserCleanup {
        self.cleanup.clone()
    }
}

pub(crate) struct AbortLease {
    registry: AbortRegistry,
    id: u64,
    controller: Option<AbortController>,
}

impl AbortLease {
    pub(crate) fn signal(&self) -> AbortSignal {
        self.controller
            .as_ref()
            .expect("active abort lease")
            .signal()
    }

    pub(crate) fn controller(&self) -> AbortController {
        self.controller
            .as_ref()
            .expect("active abort lease")
            .clone()
    }

    pub(crate) fn complete(mut self) {
        self.registry.controllers.borrow_mut().remove(&self.id);
        self.controller = None;
    }
}

impl Drop for AbortLease {
    fn drop(&mut self) {
        let registered = {
            let mut controllers = self.registry.controllers.borrow_mut();
            controllers.remove(&self.id)
        };
        let controller = registered.or_else(|| self.controller.take());
        if let Some(controller) = controller {
            self.registry.cleanup.run(move || controller.abort());
        }
    }
}

pub(crate) struct WebServices {
    pub aborts: AbortRegistry,
}

impl WebServices {
    pub(crate) fn install(
        runtime: &mut Runtime,
        asset_base: Option<&str>,
        aborts: AbortRegistry,
        wake: Rc<dyn Fn()>,
    ) -> Result<Self, String> {
        runtime.clipboard = Rc::new(clipboard::WebClipboard::new(wake.clone()));
        runtime.storage = Rc::new(storage::WebStorage::new(wake.clone()));
        runtime.network = Rc::new(network::WebNetwork::new(aborts.clone(), wake.clone()));
        runtime.platform = Rc::new(platform::WebPlatform);
        let policy = asset_base.map(AssetPolicy::parse).transpose()?;
        runtime.image_resolver = Rc::new(WebImageResolver::new(policy, aborts.clone(), wake));
        Ok(Self { aborts })
    }
}

impl Drop for WebServices {
    fn drop(&mut self) {
        self.aborts.abort_all();
    }
}
