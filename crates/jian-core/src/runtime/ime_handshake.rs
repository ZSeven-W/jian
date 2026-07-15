use super::Runtime;
use crate::widget_state::{WidgetState, WidgetStateStore};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeControlOp {
    Commit,
    Cancel,
    Dismiss,
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

    fn field_key(&self, id: u64) -> Option<(String, String)> {
        self.requests
            .get(&id)
            .map(|request| request.snapshot.field_key.clone())
    }

    fn snapshot(&self, id: u64) -> Option<ImeSnapshot> {
        self.requests
            .get(&id)
            .map(|request| request.snapshot.clone())
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
        self.confirm_ime_commit_with_cursor(request_id, text, 1)
    }

    pub fn confirm_ime_commit_with_cursor(
        &mut self,
        request_id: u64,
        text: &str,
        new_cursor_position: i32,
    ) -> ImeConfirmOutcome {
        let snapshot = self.ime_registry.snapshot(request_id);
        let field = self.ime_registry.field_key(request_id);
        let outcome = self
            .ime_registry
            .confirm_commit(request_id, text, &mut self.widget_states);
        if outcome == ImeConfirmOutcome::Applied {
            if let Some(snapshot) = snapshot.as_ref() {
                self.position_confirmed_cursor(snapshot, text, new_cursor_position);
            }
        }
        self.sync_confirmed_field(field.as_ref(), outcome);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    fn position_confirmed_cursor(
        &mut self,
        snapshot: &ImeSnapshot,
        replacement: &str,
        new_cursor_position: i32,
    ) {
        use crate::render::{byte_to_utf16_offset, utf16_len, utf16_to_byte_offset};

        let (page, node) = &snapshot.field_key;
        let Some(WidgetState::TextInput(field)) = self.widget_states.get_for_page_mut(page, node)
        else {
            return;
        };
        let insertion = i64::from(byte_to_utf16_offset(
            &snapshot.durable_text,
            snapshot.region.0,
        ));
        let replacement_len = i64::from(utf16_len(replacement));
        let requested = if new_cursor_position > 0 {
            insertion + replacement_len + i64::from(new_cursor_position) - 1
        } else {
            insertion + i64::from(new_cursor_position)
        };
        let requested = requested.clamp(0, i64::from(utf16_len(field.text()))) as u32;
        let cursor = utf16_to_byte_offset(field.text(), requested);
        field.set_effective_selection(cursor, cursor, self.now_ms);
    }

    pub fn confirm_ime_cancel(&mut self, request_id: u64) -> ImeConfirmOutcome {
        let field = self.ime_registry.field_key(request_id);
        let outcome = self
            .ime_registry
            .confirm_cancel(request_id, &mut self.widget_states);
        self.sync_confirmed_field(field.as_ref(), outcome);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    fn sync_confirmed_field(
        &mut self,
        field: Option<&(String, String)>,
        outcome: ImeConfirmOutcome,
    ) {
        if outcome == ImeConfirmOutcome::Applied {
            if let Some((page, node)) = field {
                if page == &self.active_page_key {
                    self.sync_widget_binding(node);
                }
            }
        }
    }

    pub fn focused_ime_snapshot(&self) -> Option<ImeSnapshot> {
        let node_id = self.focused_widget_id()?;
        self.ime_snapshot_for(&self.active_page_key, &node_id)
    }

    pub fn cancel_ime_snapshot_locally(&mut self, snapshot: &ImeSnapshot) -> bool {
        let (page, node) = &snapshot.field_key;
        let Some(WidgetState::TextInput(field)) = self.widget_states.get_for_page_mut(page, node)
        else {
            return false;
        };
        let changed = field.cancel_composition(self.now_ms);
        if changed && page == &self.active_page_key {
            self.sync_widget_binding(node);
        }
        changed
    }

    pub(super) fn active_ime_snapshot(&self) -> Option<ImeSnapshot> {
        self.widget_states.iter().find_map(|(node_id, state)| {
            self.ime_snapshot_from_state(&self.active_page_key, node_id, state)
        })
    }

    fn ime_snapshot_for(&self, page: &str, node: &str) -> Option<ImeSnapshot> {
        let state = self.widget_states.get_for_page(page, node)?;
        self.ime_snapshot_from_state(page, node, state)
    }

    fn ime_snapshot_from_state(
        &self,
        page: &str,
        node: &str,
        state: &WidgetState,
    ) -> Option<ImeSnapshot> {
        let WidgetState::TextInput(field) = state else {
            return None;
        };
        let composition = field.composition()?;
        let region = composition
            .region
            .unwrap_or_else(|| (field.caret(), field.caret()));
        Some(ImeSnapshot {
            field_key: (page.to_owned(), node.to_owned()),
            region,
            text: composition.text.clone(),
            durable_text: field.text().to_owned(),
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
