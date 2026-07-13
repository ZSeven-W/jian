//! Keyboard fields consumed by Runtime's focus-aware entry point.

use jian_core::gesture::Modifiers;
use web_sys::KeyboardEvent;

pub fn key(event: &KeyboardEvent) -> String {
    event.key()
}

pub fn modifiers(event: &KeyboardEvent) -> Modifiers {
    let mut result = Modifiers::empty();
    result.set(Modifiers::SHIFT, event.shift_key());
    result.set(Modifiers::CTRL, event.ctrl_key());
    result.set(Modifiers::ALT, event.alt_key());
    result.set(Modifiers::CMD, event.meta_key());
    result
}
