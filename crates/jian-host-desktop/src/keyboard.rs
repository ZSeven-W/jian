//! Keyboard translation — winit `KeyEvent` → `(key_string, Modifiers)`.
//!
//! jian-core doesn't ship a canonical keyboard event type yet (Plan 5
//! scope kept it at `SemanticEvent::KeyDown { key: String, modifiers }`),
//! so the translator returns the tuple directly. `key_string` follows
//! the web-ish convention: printable chars as themselves
//! (`"a"`, `"Enter"`, `"ArrowLeft"`, etc.).

use jian_core::gesture::Modifiers;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn modifiers_from_winit(m: ModifiersState) -> Modifiers {
    let mut out = Modifiers::empty();
    if m.shift_key() {
        out |= Modifiers::SHIFT;
    }
    if m.control_key() {
        out |= Modifiers::CTRL;
    }
    if m.alt_key() {
        out |= Modifiers::ALT;
    }
    if m.super_key() {
        out |= Modifiers::CMD;
    }
    out
}

/// Translate a winit `KeyEvent` to a `(key, modifiers, pressed)` tuple.
/// Returns `None` for releases if the caller only cares about presses.
pub fn translate_key(event: &KeyEvent, mods: ModifiersState) -> Option<(String, Modifiers)> {
    if event.state != ElementState::Pressed {
        return None;
    }
    let key = match &event.logical_key {
        Key::Named(named) => named_key_string(*named).to_owned(),
        Key::Character(s) => s.to_string(),
        Key::Unidentified(_) => return None,
        Key::Dead(_) => return None,
    };
    Some((key, modifiers_from_winit(mods)))
}

fn named_key_string(k: NamedKey) -> &'static str {
    use NamedKey::*;
    match k {
        Enter => "Enter",
        Tab => "Tab",
        Space => "Space",
        Escape => "Escape",
        Backspace => "Backspace",
        Delete => "Delete",
        ArrowUp => "ArrowUp",
        ArrowDown => "ArrowDown",
        ArrowLeft => "ArrowLeft",
        ArrowRight => "ArrowRight",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Shift => "Shift",
        Control => "Control",
        Alt => "Alt",
        Super => "Meta",
        _ => "Unidentified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_cover_all_four_bits() {
        let mut m = ModifiersState::empty();
        m.insert(ModifiersState::SHIFT);
        m.insert(ModifiersState::CONTROL);
        m.insert(ModifiersState::ALT);
        m.insert(ModifiersState::SUPER);
        let j = modifiers_from_winit(m);
        assert!(j.contains(Modifiers::SHIFT));
        assert!(j.contains(Modifiers::CTRL));
        assert!(j.contains(Modifiers::ALT));
        assert!(j.contains(Modifiers::CMD));
    }

    #[test]
    fn empty_modifiers_is_empty() {
        let j = modifiers_from_winit(ModifiersState::empty());
        assert!(j.is_empty());
    }

    #[test]
    fn named_key_mapping_is_stable() {
        assert_eq!(named_key_string(NamedKey::Enter), "Enter");
        assert_eq!(named_key_string(NamedKey::ArrowLeft), "ArrowLeft");
        assert_eq!(named_key_string(NamedKey::Space), "Space");
    }
}
