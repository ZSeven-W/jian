//! `arboard`-backed `ClipboardService` — only compiled under the
//! `clipboard` feature so CI can skip the native dep when it's not
//! needed.
//!
//! Wrapping is thread-local because `arboard::Clipboard` is `!Send`
//! and the Jian runtime is single-threaded.

use async_trait::async_trait;
use jian_core::action::services::ClipboardService;
use std::cell::RefCell;

pub struct DesktopClipboard {
    inner: RefCell<arboard::Clipboard>,
}

impl DesktopClipboard {
    pub fn new() -> Result<Self, arboard::Error> {
        Ok(Self {
            inner: RefCell::new(arboard::Clipboard::new()?),
        })
    }
}

#[async_trait(?Send)]
impl ClipboardService for DesktopClipboard {
    async fn read_text(&self) -> Option<String> {
        self.inner.borrow_mut().get_text().ok()
    }
    async fn write_text(&self, text: &str) {
        let _ = self.inner.borrow_mut().set_text(text);
    }
}
