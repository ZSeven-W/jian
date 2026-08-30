//! Single-threaded runtime ownership with ordered reentrant DOM input.

use jian_core::gesture::{ImeEvent, Modifiers, PointerEvent, WheelEvent};
use jian_core::Runtime;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::VecDeque;

enum PendingInput {
    Pointer(PointerEvent),
    Wheel(WheelEvent),
    Keyboard {
        key: String,
        modifiers: Modifiers,
        now_ms: u64,
    },
    Ime {
        event: ImeEvent,
        now_ms: u64,
    },
    Text {
        text: String,
        now_ms: u64,
    },
}

/// The runtime is taken out while the pump polls authored service futures.
/// Browser callbacks therefore never contend with a live `RefMut`; input that
/// re-enters during that window is retained and applied in DOM arrival order.
pub(crate) struct RuntimeSlot {
    runtime: RefCell<Option<Runtime>>,
    pending: RefCell<VecDeque<PendingInput>>,
    cancel_on_put: Cell<bool>,
    dirty_on_put: Cell<bool>,
    viewport: Cell<(f32, f32)>,
}

impl RuntimeSlot {
    pub(crate) fn new(runtime: Runtime) -> Self {
        let viewport = runtime.viewport.size;
        Self {
            runtime: RefCell::new(Some(runtime)),
            pending: RefCell::new(VecDeque::new()),
            cancel_on_put: Cell::new(false),
            dirty_on_put: Cell::new(false),
            viewport: Cell::new((viewport.width, viewport.height)),
        }
    }

    pub(crate) fn borrow(&self) -> Ref<'_, Runtime> {
        Ref::map(self.runtime.borrow(), |runtime| {
            runtime.as_ref().expect("runtime is being pumped")
        })
    }

    pub(crate) fn borrow_mut(&self) -> RefMut<'_, Runtime> {
        RefMut::map(self.runtime.borrow_mut(), |runtime| {
            runtime.as_mut().expect("runtime is being pumped")
        })
    }

    pub(crate) fn take(&self) -> Runtime {
        let runtime = {
            let mut slot = self.runtime.borrow_mut();
            slot.take()
        };
        runtime.expect("runtime ownership is already detached")
    }

    pub(crate) fn put(&self, mut runtime: Runtime) {
        loop {
            if self.cancel_on_put.replace(false) {
                self.pending.borrow_mut().clear();
                runtime.cancel_all_tasks();
                break;
            }
            let inputs = std::mem::take(&mut *self.pending.borrow_mut());
            if inputs.is_empty() {
                break;
            }
            for input in inputs {
                apply_input(&mut runtime, input);
            }
        }
        let viewport = runtime.viewport.size;
        if self.dirty_on_put.replace(false) {
            runtime.mark_dirty();
        }
        self.viewport.set((viewport.width, viewport.height));
        *self.runtime.borrow_mut() = Some(runtime);
    }

    pub(crate) fn viewport_size(&self) -> (f32, f32) {
        self.viewport.get()
    }

    pub(crate) fn cancel_all_tasks(&self) {
        let runtime = {
            let mut slot = self.runtime.borrow_mut();
            slot.take()
        };
        if let Some(mut runtime) = runtime {
            // Dropping a task can abort a browser request synchronously. The
            // abort handler is allowed to dispatch DOM input, dispose, or
            // start a document replacement, so the Runtime must be owned
            // locally before cancellation begins. Reentrant input sees an
            // empty slot and is queued until `put` restores ownership.
            runtime.cancel_all_tasks();
            self.put(runtime);
        } else {
            self.cancel_on_put.set(true);
        }
    }

    pub(crate) fn mark_dirty(&self) {
        let mut slot = self.runtime.borrow_mut();
        if let Some(runtime) = slot.as_mut() {
            runtime.mark_dirty();
        } else {
            self.dirty_on_put.set(true);
        }
    }

    pub(crate) fn dispatch_pointer(&self, event: PointerEvent) {
        self.dispatch_or_queue(PendingInput::Pointer(event));
    }

    pub(crate) fn dispatch_wheel(&self, event: WheelEvent) {
        self.dispatch_or_queue(PendingInput::Wheel(event));
    }

    pub(crate) fn dispatch_keyboard(
        &self,
        key: String,
        modifiers: Modifiers,
        now_ms: u64,
    ) -> Option<bool> {
        let runtime = {
            let mut slot = self.runtime.borrow_mut();
            slot.take()
        };
        let Some(mut runtime) = runtime else {
            self.pending.borrow_mut().push_back(PendingInput::Keyboard {
                key,
                modifiers,
                now_ms,
            });
            return None;
        };
        runtime.set_now_ms(now_ms);
        let consumed = !runtime
            .dispatch_keyboard(key.clone(), key, false, modifiers)
            .is_empty();
        self.put(runtime);
        Some(consumed)
    }

    pub(crate) fn dispatch_ime(&self, event: ImeEvent, now_ms: u64) {
        self.dispatch_or_queue(PendingInput::Ime { event, now_ms });
    }

    pub(crate) fn dispatch_text(&self, text: String, now_ms: u64) {
        self.dispatch_or_queue(PendingInput::Text { text, now_ms });
    }

    fn dispatch_or_queue(&self, input: PendingInput) {
        let runtime = {
            let mut slot = self.runtime.borrow_mut();
            slot.take()
        };
        let Some(mut runtime) = runtime else {
            self.pending.borrow_mut().push_back(input);
            return;
        };
        apply_input(&mut runtime, input);
        self.put(runtime);
    }
}

fn apply_input(runtime: &mut Runtime, input: PendingInput) {
    match input {
        PendingInput::Pointer(event) => {
            runtime.dispatch_pointer(event);
        }
        PendingInput::Wheel(event) => {
            runtime.dispatch_wheel(event);
        }
        PendingInput::Keyboard {
            key,
            modifiers,
            now_ms,
        } => {
            runtime.set_now_ms(now_ms);
            runtime.dispatch_keyboard(key.clone(), key, false, modifiers);
        }
        PendingInput::Ime { event, now_ms } => {
            runtime.set_now_ms(now_ms);
            let _ = runtime.dispatch_ime(event);
        }
        PendingInput::Text { text, now_ms } => {
            runtime.set_now_ms(now_ms);
            let _ = runtime.dispatch_text_input(&text);
        }
    }
}
