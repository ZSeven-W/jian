use super::Runtime;
use crate::error::{CoreError, CoreResult};
use crate::gesture::SemanticEvent;

impl Runtime {
    /// Stable schema id of the currently-focused node, if any.
    pub fn focused_widget_id(&self) -> Option<String> {
        let key = self.focus.current()?;
        let node = self.document.as_ref()?.tree.nodes.get(key)?;
        Some(crate::document::tree::node_schema_id(&node.schema).to_owned())
    }

    /// Move focus forward one step (`Tab`) and emit the corresponding
    /// `FocusLost` / `FocusGained` events.
    pub fn focus_next(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.next();
        Ok(self.emit_focus_change(change))
    }

    /// Move focus backward one step (`Shift+Tab`).
    pub fn focus_previous(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.previous();
        Ok(self.emit_focus_change(change))
    }

    /// Programmatically focus an explicit node. Hosts call this from
    /// click handlers (focus-on-click) or from `jian-action-surface`
    /// when an AI client requests a focus change.
    pub fn focus_request(
        &mut self,
        node: crate::document::NodeKey,
    ) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.request(node);
        Ok(self.emit_focus_change(change))
    }

    /// Drop focus entirely — typically wired to clicking outside any
    /// focusable node, or to the window losing OS focus.
    pub fn focus_clear(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.clear();
        Ok(self.emit_focus_change(change))
    }

    pub(super) fn emit_focus_change(
        &mut self,
        change: crate::gesture::FocusChange,
    ) -> Vec<SemanticEvent> {
        if change.is_noop() {
            return Vec::new();
        }
        let mut emitted = Vec::with_capacity(2);
        if let Some(prev) = change.previous {
            let ev = SemanticEvent::FocusLost { node: prev };
            self.dispatch_semantic_secondary(&ev);
            emitted.push(ev);
        }
        // Re-read focus after FocusLost dispatch because an authored blur
        // handler may have moved it re-entrantly.
        if let Some(next) = change.current {
            if self.focus.current() == Some(next) {
                let ev = SemanticEvent::FocusGained { node: next };
                self.dispatch_semantic_secondary(&ev);
                emitted.push(ev);
            }
        }
        emitted
    }
}
