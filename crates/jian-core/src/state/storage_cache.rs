use crate::signal::{scheduler::Scheduler, Signal};
use crate::value::RuntimeValue;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum StorageEntryState {
    Unhydrated,
    Hydrating { gen: u64 },
    Present(Signal<RuntimeValue>),
    Failed,
}

#[derive(Clone)]
pub struct StorageEntry {
    pub state: StorageEntryState,
    pub gen: u64,
}

pub struct StorageCache {
    entries: RefCell<BTreeMap<String, StorageEntry>>,
    requests: RefCell<Vec<(String, u64)>>,
    pub scope_version: Signal<u64>,
    scheduler: Rc<Scheduler>,
}

impl StorageCache {
    pub fn new(scheduler: Rc<Scheduler>) -> Self {
        Self {
            entries: RefCell::new(BTreeMap::new()),
            requests: RefCell::new(Vec::new()),
            scope_version: Signal::new(0, scheduler.clone()),
            scheduler,
        }
    }

    pub fn read(&self, key: &str) -> RuntimeValue {
        let mut entries = self.entries.borrow_mut();
        let entry = entries.entry(key.to_owned()).or_insert(StorageEntry {
            state: StorageEntryState::Unhydrated,
            gen: 0,
        });
        match &entry.state {
            StorageEntryState::Present(signal) => signal.get(),
            StorageEntryState::Unhydrated => {
                let gen = entry.gen;
                entry.state = StorageEntryState::Hydrating { gen };
                self.requests.borrow_mut().push((key.to_owned(), gen));
                RuntimeValue::null()
            }
            StorageEntryState::Hydrating { .. } | StorageEntryState::Failed => RuntimeValue::null(),
        }
    }

    pub fn take_requests(&self) -> Vec<(String, u64)> {
        std::mem::take(&mut *self.requests.borrow_mut())
    }

    pub fn complete(&self, key: &str, gen: u64, value: Result<Option<Value>, String>) {
        let mut entries = self.entries.borrow_mut();
        let Some(entry) = entries.get_mut(key) else {
            return;
        };
        if entry.gen != gen
            || !matches!(entry.state, StorageEntryState::Hydrating { gen: active } if active == gen)
        {
            return;
        }
        entry.state = match value {
            Ok(Some(value)) => {
                StorageEntryState::Present(Signal::new(RuntimeValue(value), self.scheduler.clone()))
            }
            Ok(None) | Err(_) => StorageEntryState::Failed,
        };
        self.scope_version
            .update(|version| *version = version.wrapping_add(1));
    }

    pub fn set_local(&self, key: &str, value: Value) {
        let mut entries = self.entries.borrow_mut();
        let next_gen = entries
            .get(key)
            .map_or(1, |entry| entry.gen.wrapping_add(1));
        entries.insert(
            key.to_owned(),
            StorageEntry {
                state: StorageEntryState::Present(Signal::new(
                    RuntimeValue(value),
                    self.scheduler.clone(),
                )),
                gen: next_gen,
            },
        );
        self.scope_version
            .update(|version| *version = version.wrapping_add(1));
    }

    pub fn snapshot(&self) -> Value {
        let _ = self.scope_version.get();
        let map = self
            .entries
            .borrow()
            .iter()
            .filter_map(|(key, entry)| match &entry.state {
                StorageEntryState::Present(signal) => Some((key.clone(), signal.get().0)),
                _ => None,
            })
            .collect();
        Value::Object(map)
    }

    pub fn purge(&self) {
        self.entries.borrow_mut().clear();
        self.requests.borrow_mut().clear();
        self.scope_version
            .update(|version| *version = version.wrapping_add(1));
    }

    /// Cancellation compensation for hydration tasks. Entries whose request
    /// future was dropped must be readable again on the next evaluation.
    pub fn cancel_hydrations(&self) {
        let mut entries = self.entries.borrow_mut();
        for entry in entries.values_mut() {
            if matches!(entry.state, StorageEntryState::Hydrating { .. }) {
                entry.gen = entry.gen.wrapping_add(1);
                entry.state = StorageEntryState::Unhydrated;
            }
        }
        self.requests.borrow_mut().clear();
    }

    pub fn remove(&self, key: &str) {
        if let Some(entry) = self.entries.borrow_mut().remove(key) {
            if let StorageEntryState::Present(signal) = entry.state {
                signal.set(RuntimeValue::null());
            }
            self.scope_version
                .update(|version| *version = version.wrapping_add(1));
        }
    }

    pub fn clear_present(&self) {
        let keys: Vec<String> = self.entries.borrow().keys().cloned().collect();
        for key in keys {
            self.remove(&key);
        }
    }
}
