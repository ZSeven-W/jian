//! FocusManager — keyboard focus tracking (MVP stub).
//!
//! Full focus-tree traversal and IME integration land in Plan 9 (host-desktop).
//! This stub is enough for Plan 5's gate: request/clear focus, read current.

use crate::document::NodeKey;

#[derive(Debug, Default)]
pub struct FocusManager {
    current: Option<NodeKey>,
}

impl FocusManager {
    pub fn new() -> Self {
        Self { current: None }
    }

    pub fn current(&self) -> Option<NodeKey> {
        self.current
    }

    pub fn request(&mut self, node: NodeKey) -> Option<NodeKey> {
        let prev = self.current;
        self.current = Some(node);
        prev
    }

    pub fn clear(&mut self) -> Option<NodeKey> {
        self.current.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn request_and_clear() {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        let a = sm.insert(0);
        let b = sm.insert(0);
        let mut f = FocusManager::new();
        assert!(f.current().is_none());
        assert!(f.request(a).is_none());
        assert_eq!(f.current(), Some(a));
        assert_eq!(f.request(b), Some(a));
        assert_eq!(f.clear(), Some(b));
        assert!(f.current().is_none());
    }
}
