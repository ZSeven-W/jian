//! Step 1b §3.2 P0.5A — KeyEvent / ImeEvent / FocusEvent / WheelEvent.mode
//! field shape + serde roundtrip + UTF-8 byte index semantics.

use jian_core::gesture::{
    FocusEvent, ImeEvent, ImeKind, KeyCode, KeyEvent, KeyLocation, KeyState, KeyValue, Modifiers,
    NamedKey, ScrollMode, WheelEvent,
};

#[test]
fn key_event_carries_all_w3c_fields() {
    let event = KeyEvent {
        key: KeyValue::Char('a'),
        code: KeyCode::KeyA,
        location: KeyLocation::Standard,
        modifiers: Modifiers::SHIFT,
        state: KeyState::Pressed,
        repeat: true,
        is_composing: false,
    };

    assert_eq!(event.key, KeyValue::Char('a'));
    assert_eq!(event.code, KeyCode::KeyA);
    assert_eq!(event.location, KeyLocation::Standard);
    assert!(event.modifiers.contains(Modifiers::SHIFT));
    assert_eq!(event.state, KeyState::Pressed);
    assert!(event.repeat);
    assert!(!event.is_composing);
}

#[test]
fn key_event_is_composing_during_ime_composition() {
    let event = KeyEvent {
        key: KeyValue::Char('a'),
        code: KeyCode::KeyA,
        location: KeyLocation::Standard,
        modifiers: Modifiers::empty(),
        state: KeyState::Pressed,
        repeat: false,
        is_composing: true,
    };
    assert!(event.is_composing);
}

#[test]
fn key_value_unidentified_escape_hatch_works() {
    let event = KeyValue::Unidentified("ContextMenu".to_string());
    let json = serde_json::to_string(&event).expect("serialize");
    let decoded: KeyValue = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, KeyValue::Unidentified("ContextMenu".to_string()));
}

#[test]
fn key_code_unknown_escape_hatch_works() {
    let event = KeyCode::Unknown("AudioVolumeUp".to_string());
    let json = serde_json::to_string(&event).expect("serialize");
    let decoded: KeyCode = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, KeyCode::Unknown("AudioVolumeUp".to_string()));
}

#[test]
fn named_key_round_trips_through_serde() {
    let event = KeyEvent {
        key: KeyValue::Named(NamedKey::Enter),
        code: KeyCode::Enter,
        location: KeyLocation::Standard,
        modifiers: Modifiers::empty(),
        state: KeyState::Released,
        repeat: false,
        is_composing: false,
    };

    let json = serde_json::to_string(&event).expect("serialize key event");
    let decoded: KeyEvent = serde_json::from_str(&json).expect("deserialize key event");
    assert_eq!(decoded.key, KeyValue::Named(NamedKey::Enter));
    assert_eq!(decoded.state, KeyState::Released);
    assert!(!decoded.is_composing);
}

#[test]
fn ime_update_selection_uses_utf8_byte_offsets() {
    // "你a好" — `你` is 3 UTF-8 bytes, `a` is 1, `好` is 3. Selection
    // 0..3 should cover "你" exactly when the host translator has done
    // its UTF-16→UTF-8 remap.
    let text = "你a好".to_string();
    let event = ImeEvent {
        kind: ImeKind::CompositionUpdate {
            selection: Some(0..3),
        },
        text,
    };

    match event.kind {
        ImeKind::CompositionUpdate { selection } => assert_eq!(selection, Some(0..3)),
        ImeKind::CompositionStart | ImeKind::CompositionEnd => panic!("expected update"),
    }
    assert_eq!(&event.text[0..3], "你");
}

#[test]
fn ime_event_round_trips_through_serde() {
    let event = ImeEvent {
        kind: ImeKind::CompositionUpdate {
            selection: Some(0..3),
        },
        text: "你a好".to_string(),
    };
    let json = serde_json::to_string(&event).expect("serialize ime event");
    let decoded: ImeEvent = serde_json::from_str(&json).expect("deserialize ime event");
    assert_eq!(decoded.text, "你a好");
    match decoded.kind {
        ImeKind::CompositionUpdate { selection } => assert_eq!(selection, Some(0..3)),
        _ => panic!("expected update"),
    }
}

#[test]
fn focus_event_can_target_window_or_node() {
    let window = FocusEvent {
        gained: true,
        node_id_hint: None,
        related_node_id_hint: None,
    };
    let node = FocusEvent {
        gained: false,
        node_id_hint: Some(42),
        related_node_id_hint: Some(7),
    };

    assert!(window.gained);
    assert_eq!(node.node_id_hint, Some(42));
    assert_eq!(node.related_node_id_hint, Some(7));
}

#[test]
fn focus_event_round_trips_through_serde() {
    let event = FocusEvent {
        gained: true,
        node_id_hint: Some(42),
        related_node_id_hint: Some(7),
    };
    let json = serde_json::to_string(&event).expect("serialize focus event");
    let decoded: FocusEvent = serde_json::from_str(&json).expect("deserialize focus event");
    assert!(decoded.gained);
    assert_eq!(decoded.node_id_hint, Some(42));
    assert_eq!(decoded.related_node_id_hint, Some(7));
}

#[test]
fn wheel_event_carries_w3c_delta_mode_and_z() {
    use jian_core::geometry::point;
    let mut event = WheelEvent::simple(point(10.0, 20.0), point(0.0, 120.0));
    assert_eq!(event.mode, ScrollMode::Pixel);
    assert_eq!(event.delta_z, 0.0);

    // Override for line / page deltaMode hosts.
    event.mode = ScrollMode::Line;
    assert_eq!(event.mode, ScrollMode::Line);
    event.mode = ScrollMode::Page;
    assert_eq!(event.mode, ScrollMode::Page);

    // Z-axis scroll (rare, e.g. 3D tracker).
    event.delta_z = 5.0;
    assert_eq!(event.delta_z, 5.0);
}

#[test]
fn scroll_mode_round_trips_through_serde() {
    for mode in [ScrollMode::Pixel, ScrollMode::Line, ScrollMode::Page] {
        let json = serde_json::to_string(&mode).expect("serialize");
        let decoded: ScrollMode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, mode);
    }
}
