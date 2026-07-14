use super::Runtime;
use crate::widget_state::{WidgetState, WidgetStateStore};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeControlOp {
    Commit,
    Cancel,
}

pub trait ImeHost {
    fn request_ime_control(&mut self, op: ImeControlOp, request_id: u64) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImeSnapshot {
    pub field_key: (String, String),
    pub region: (usize, usize),
    pub text: String,
    pub durable_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeConfirmOutcome {
    Applied,
    Dropped,
    NoOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Active,
    Detached,
    Voided,
    Complete,
}

#[derive(Debug, Clone)]
struct ImeRequest {
    snapshot: ImeSnapshot,
    state: RequestState,
}

#[derive(Debug, Default)]
pub(crate) struct ImeRegistry {
    next_id: u64,
    requests: BTreeMap<u64, ImeRequest>,
}

impl ImeRegistry {
    pub fn issue(&mut self, snapshot: ImeSnapshot) -> u64 {
        for request in self.requests.values_mut() {
            if request.snapshot.field_key == snapshot.field_key
                && matches!(request.state, RequestState::Active | RequestState::Detached)
            {
                request.state = RequestState::Voided;
            }
        }
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.requests.insert(
            self.next_id,
            ImeRequest {
                snapshot,
                state: RequestState::Active,
            },
        );
        self.next_id
    }

    pub fn detach(&mut self, id: u64) {
        if let Some(request) = self.requests.get_mut(&id) {
            if request.state == RequestState::Active {
                request.state = RequestState::Detached;
            }
        }
    }

    pub fn confirm_commit(
        &mut self,
        id: u64,
        replacement: &str,
        store: &mut WidgetStateStore,
    ) -> ImeConfirmOutcome {
        self.apply(id, replacement, store)
    }

    pub fn confirm_cancel(&mut self, id: u64, store: &mut WidgetStateStore) -> ImeConfirmOutcome {
        self.apply(id, "", store)
    }

    fn apply(
        &mut self,
        id: u64,
        replacement: &str,
        store: &mut WidgetStateStore,
    ) -> ImeConfirmOutcome {
        let Some(request) = self.requests.get_mut(&id) else {
            return ImeConfirmOutcome::NoOp;
        };
        if !matches!(request.state, RequestState::Active | RequestState::Detached) {
            return ImeConfirmOutcome::NoOp;
        }
        request.state = RequestState::Complete;
        let (page, node) = &request.snapshot.field_key;
        let Some(WidgetState::TextInput(field)) = store.get_for_page_mut(page, node) else {
            return ImeConfirmOutcome::Dropped;
        };
        let (start, end) = request.snapshot.region;
        let durable_matches = field.text() == request.snapshot.durable_text;
        // A confirmation consumes the source composition even when its
        // durable context moved while the host request was outstanding.
        field.clear_composition();
        if !durable_matches || start > end || end > field.text().len() {
            return ImeConfirmOutcome::Dropped;
        }
        field.replace_range(start, end, replacement, 0);
        ImeConfirmOutcome::Applied
    }
}

impl Runtime {
    pub fn begin_ime_handshake(&mut self, snapshot: ImeSnapshot) -> u64 {
        self.ime_registry.issue(snapshot)
    }

    pub fn confirm_ime_commit(&mut self, request_id: u64, text: &str) -> ImeConfirmOutcome {
        let outcome = self
            .ime_registry
            .confirm_commit(request_id, text, &mut self.widget_states);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    pub fn confirm_ime_cancel(&mut self, request_id: u64) -> ImeConfirmOutcome {
        let outcome = self
            .ime_registry
            .confirm_cancel(request_id, &mut self.widget_states);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    pub(super) fn active_ime_snapshot(&self) -> Option<ImeSnapshot> {
        self.widget_states.iter().find_map(|(node_id, state)| {
            let WidgetState::TextInput(field) = state else {
                return None;
            };
            let composition = field.composition()?;
            let region = composition
                .region
                .unwrap_or_else(|| (field.caret(), field.caret()));
            Some(ImeSnapshot {
                field_key: (self.active_page_key.clone(), node_id.to_owned()),
                region,
                text: composition.text.clone(),
                durable_text: field.text().to_owned(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::scheduler::Scheduler;
    use crate::state::StateGraph;
    use crate::widget_state::{WidgetState, WidgetStateStore};
    use jian_ops_schema::node::PenNode;
    use std::rc::Rc;

    fn store_with(page: &str, id: &str, text: &str) -> WidgetStateStore {
        let mut store = WidgetStateStore::default();
        store.set_page_key(page);
        let node: PenNode = serde_json::from_str(&format!(
            r#"{{"type":"text_input","id":"{id}","value":"{text}"}}"#
        ))
        .unwrap();
        let state = StateGraph::new(Rc::new(Scheduler::new()));
        let _ = store.get_or_init(&node, &state);
        store
    }

    #[test]
    fn snapshot_commit_applies_only_while_region_matches() {
        let mut registry = ImeRegistry::default();
        let mut store = store_with("p", "field", "abz");
        if let Some(WidgetState::TextInput(field)) = store.get_for_page_mut("p", "field") {
            field.set_caret(2, 0);
            field.set_composition("IME", 3, 0);
        }
        let id = registry.issue(ImeSnapshot {
            field_key: ("p".into(), "field".into()),
            region: (2, 2),
            text: "IME".into(),
            durable_text: "abz".into(),
        });
        assert_eq!(
            registry.confirm_commit(id, "OK", &mut store),
            ImeConfirmOutcome::Applied
        );
        match store.get_for_page("p", "field") {
            Some(WidgetState::TextInput(text)) => assert_eq!(text.text(), "abOKz"),
            _ => panic!(),
        }
        assert_eq!(
            registry.confirm_commit(id, "again", &mut store),
            ImeConfirmOutcome::NoOp
        );
    }

    #[test]
    fn moved_region_drops_and_new_same_field_voids_old() {
        let mut registry = ImeRegistry::default();
        let mut store = store_with("p", "field", "abz");
        if let Some(WidgetState::TextInput(field)) = store.get_for_page_mut("p", "field") {
            field.set_composition("IME", 3, 0);
        }
        let old = registry.issue(ImeSnapshot {
            field_key: ("p".into(), "field".into()),
            region: (3, 3),
            text: "IME".into(),
            durable_text: "abz".into(),
        });
        let current = registry.issue(ImeSnapshot {
            field_key: ("p".into(), "field".into()),
            region: (3, 3),
            text: "IME".into(),
            durable_text: "abz".into(),
        });
        assert_eq!(
            registry.confirm_cancel(old, &mut store),
            ImeConfirmOutcome::NoOp
        );
        if let Some(WidgetState::TextInput(text)) = store.get_for_page_mut("p", "field") {
            text.replace_range(1, 2, "X", 0);
        }
        assert_eq!(
            registry.confirm_commit(current, "Y", &mut store),
            ImeConfirmOutcome::Dropped
        );
    }
}
