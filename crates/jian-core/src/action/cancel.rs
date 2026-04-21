//! Cancellation token — clone-shared boolean flag consumed by long-running
//! actions (fetch, delay, ws loops) so that unmounting a node / aborting a
//! chain stops them promptly.

use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Default)]
pub struct CancellationToken {
    flag: Rc<Cell<bool>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.set(true)
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_shares_flag() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }
}
