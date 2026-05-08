//! Browser / desktop keyboard input. Field semantics align with W3C
//! UI Events L4 `KeyboardEvent`:
//!
//! - `key`: logical character or named key (IME-aware; "a", "ArrowLeft", "Enter")
//! - `code`: physical key location (layout-independent; "KeyA", "ArrowLeft")
//! - `location`: standard / left / right / numpad
//! - `state`: Pressed (keydown) / Released (keyup)
//! - `repeat`: true on auto-repeat keydown
//!
//! Native (winit) and web (shell-web) hosts both translate their
//! native event into this shared shape so widget code consumes a
//! single contract.

use crate::gesture::Modifiers;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: KeyValue,
    pub code: KeyCode,
    pub location: KeyLocation,
    pub modifiers: Modifiers,
    pub state: KeyState,
    pub repeat: bool,
    /// Mirror of W3C `KeyboardEvent.isComposing`. `true` while the IME
    /// is composing a character (between `compositionstart` and
    /// `compositionend`). Widgets that consume keys directly should
    /// usually skip dispatch when `isComposing == true` and let the
    /// `ImeEvent` path handle the composition instead.
    pub is_composing: bool,
}

/// Logical key value. `Char` carries a single Unicode scalar so simple
/// printable keys do not require the closed `NamedKey` enum to grow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyValue {
    Char(char),
    Named(NamedKey),
    /// Browser-supplied raw `KeyboardEvent.key` for keys outside the
    /// closed `NamedKey` set. Spec §2.4 NIT-R1-1 escape hatch.
    Unidentified(String),
}

/// Closed enum for navigation / control / function keys most widgets
/// react to. Add new variants only when a widget needs to match on
/// them; otherwise the host falls back to `KeyValue::Unidentified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamedKey {
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Shift,
    Control,
    Alt,
    Meta,
    CapsLock,
}

/// Physical key code subset. Mirror of `KeyboardEvent.code` for the
/// Step 1b inspector slice (~30 codes). `Unknown(String)` carries the
/// browser-supplied raw `code` for anything outside the closed set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyCode {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    MetaLeft,
    MetaRight,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Pressed,
    Released,
}
