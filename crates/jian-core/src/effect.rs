//! Effect — a subscriber that runs when any Signal it read changes.
//!
//! Each Effect holds a closure that reads some signals and performs side effects
//! (usually writing to the scene graph). Effects are registered with an
//! [`EffectRegistry`] owned by the runtime; the registry drives the Scheduler
//! flush function.

use crate::signal::scheduler::Scheduler;
use crate::signal::tracker::{SubscriberId, TrackerGuard};
use slotmap::{DefaultKey, Key, SlotMap};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct Effect {
    pub(crate) f: Box<dyn Fn()>,
    pub(crate) id: SubscriberId,
}

pub struct EffectRegistry {
    effects: RefCell<SlotMap<DefaultKey, Effect>>,
}

impl EffectRegistry {
    pub fn new() -> Rc<Self> {
        Rc::new(EffectRegistry {
            effects: RefCell::new(SlotMap::new()),
        })
    }

    /// Register an Effect. Runs it once immediately to establish subscriptions.
    pub fn register(self: &Rc<Self>, f: impl Fn() + 'static) -> EffectHandle {
        // Reserve a slot with a placeholder; derive a stable SubscriberId from the key.
        let key = self.effects.borrow_mut().insert(Effect {
            f: Box::new(|| ()),
            id: 0,
        });
        let id = slotkey_to_subscriber(key);
        {
            let mut e = self.effects.borrow_mut();
            *e.get_mut(key).unwrap() = Effect { f: Box::new(f), id };
        }
        self.run(id);
        EffectHandle {
            registry: Rc::downgrade(self),
            key,
        }
    }

    pub fn run(&self, id: SubscriberId) {
        // Collect the effect fn first to avoid re-entrant borrow if the effect mutates the registry.
        let effects = self.effects.borrow();
        let target = effects.iter().find(|(_, e)| e.id == id);
        if let Some((_, effect)) = target {
            let _g = TrackerGuard::push(id);
            (effect.f)();
        }
    }

    pub fn len(&self) -> usize {
        self.effects.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Install self as the flush target on the scheduler.
    pub fn install_on(self: &Rc<Self>, scheduler: &Rc<Scheduler>) {
        let weak = Rc::downgrade(self);
        scheduler.set_flush_fn(move |id| {
            if let Some(strong) = weak.upgrade() {
                strong.run(id);
            }
        });
    }
}

pub struct EffectHandle {
    registry: std::rc::Weak<EffectRegistry>,
    key: DefaultKey,
}

impl Drop for EffectHandle {
    fn drop(&mut self) {
        if let Some(r) = self.registry.upgrade() {
            r.effects.borrow_mut().remove(self.key);
        }
    }
}

fn slotkey_to_subscriber(k: DefaultKey) -> SubscriberId {
    k.data().as_ffi() as SubscriberId
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::{scheduler::Scheduler, Signal};
    use std::cell::Cell;

    #[test]
    fn effect_runs_once_on_register() {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);

        let counter = Rc::new(Cell::new(0));
        {
            let c = counter.clone();
            let _h = reg.register(move || c.set(c.get() + 1));
        }
        assert_eq!(counter.get(), 1);
    }

    #[test]
    fn effect_reruns_when_signal_changes() {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);
        let s = Signal::new(0_i32, sched.clone());

        let last = Rc::new(Cell::new(-1));
        let _handle = {
            let s2 = s.clone();
            let l = last.clone();
            reg.register(move || l.set(s2.get()))
        };
        assert_eq!(last.get(), 0);

        s.set(42);
        sched.flush();
        assert_eq!(last.get(), 42);
    }

    #[test]
    fn disposed_effect_does_not_rerun() {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);
        let s = Signal::new(0_i32, sched.clone());

        let runs = Rc::new(Cell::new(0));
        {
            let s2 = s.clone();
            let r = runs.clone();
            let _h = reg.register(move || {
                let _ = s2.get();
                r.set(r.get() + 1);
            });
            // _h dropped here
        }
        let before = runs.get();
        s.set(99);
        sched.flush();
        assert_eq!(
            runs.get(),
            before,
            "effect should not run after handle is dropped"
        );
    }
}
