//! Shared authoring metadata for the action DSL.

/// Stable metadata consumed by Preview authoring surfaces and validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDescriptor {
    pub name: &'static str,
    pub category: &'static str,
    pub body_shape: &'static str,
    pub required_capability: Option<&'static str>,
    pub preview_authorable: bool,
}

const fn descriptor(
    name: &'static str,
    category: &'static str,
    body_shape: &'static str,
    required_capability: Option<&'static str>,
    preview_authorable: bool,
) -> ActionDescriptor {
    ActionDescriptor {
        name,
        category,
        body_shape,
        required_capability,
        preview_authorable,
    }
}

/// Ordered catalog. Preview-authorable entries come first in the exact policy
/// order; compatibility-only actions follow and are never authorable.
pub static ACTION_DESCRIPTORS: &[ActionDescriptor] = &[
    descriptor("set", "state", "object", None, true),
    descriptor("toggle", "state", "writable_bool_path", None, true),
    descriptor("delete", "state", "state_path", None, true),
    descriptor("reset", "state", "scope_or_route_expression", None, true),
    descriptor("if", "control-flow", "condition_branches", None, true),
    descriptor("delay", "control-flow", "duration", None, true),
    descriptor("parallel", "control-flow", "action_lists", None, true),
    descriptor("push", "navigation", "route_expression", None, true),
    descriptor("replace", "navigation", "route_expression", None, true),
    descriptor("pop", "navigation", "empty", None, true),
    descriptor("show", "ui", "node_id", None, true),
    descriptor("hide", "ui", "node_id", None, true),
    descriptor("toggle_visibility", "ui", "node_id", None, true),
    descriptor("focus", "ui", "node_target", Some("focus"), true),
    descriptor("blur", "ui", "empty", Some("focus"), true),
    descriptor("scroll_to", "ui", "scroll_target", None, true),
    descriptor("animate", "ui", "animation", None, true),
    descriptor(
        "toast",
        "feedback",
        "message_options",
        Some("notifications"),
        true,
    ),
    descriptor(
        "alert",
        "feedback",
        "title_message",
        Some("notifications"),
        true,
    ),
    descriptor(
        "confirm",
        "feedback",
        "title_message_branches",
        Some("notifications"),
        true,
    ),
    descriptor(
        "open_url",
        "system-effect",
        "url_expression",
        Some("open_url"),
        true,
    ),
    descriptor(
        "copy",
        "system-effect",
        "text_expression",
        Some("clipboard"),
        true,
    ),
    descriptor(
        "share",
        "system-effect",
        "share_payload",
        Some("share"),
        true,
    ),
    descriptor(
        "haptic",
        "system-effect",
        "haptic_options",
        Some("haptics"),
        true,
    ),
    descriptor(
        "dismiss_keyboard",
        "system-effect",
        "empty",
        Some("dismiss_keyboard"),
        true,
    ),
    // Registered compatibility vocabulary, unavailable to Preview authors.
    descriptor("abort", "control-flow", "empty", None, false),
    descriptor("for_each", "control-flow", "iteration", None, false),
    descriptor("race", "control-flow", "action_lists", None, false),
    descriptor(
        "paste",
        "system-effect",
        "state_target",
        Some("clipboard"),
        false,
    ),
    descriptor(
        "storage_set",
        "system-effect",
        "storage_values",
        Some("storage"),
        false,
    ),
    descriptor(
        "storage_clear",
        "system-effect",
        "storage_target",
        Some("storage"),
        false,
    ),
    descriptor(
        "storage_wipe",
        "system-effect",
        "confirmation_actions",
        Some("storage"),
        false,
    ),
    descriptor("fetch", "system-effect", "request", Some("network"), false),
    descriptor(
        "ws_connect",
        "system-effect",
        "websocket_connect",
        Some("network"),
        false,
    ),
    descriptor(
        "ws_send",
        "system-effect",
        "websocket_message",
        Some("network"),
        false,
    ),
    descriptor(
        "ws_close",
        "system-effect",
        "websocket_target",
        Some("network"),
        false,
    ),
    descriptor(
        "vibrate",
        "system-effect",
        "vibration_options",
        Some("haptics"),
        false,
    ),
    descriptor(
        "notify",
        "feedback",
        "notification_options",
        Some("notifications"),
        false,
    ),
    descriptor(
        "call",
        "system-effect",
        "logic_call",
        Some("logic_provider"),
        false,
    ),
];

pub fn preview_action_descriptors() -> &'static [ActionDescriptor] {
    ACTION_DESCRIPTORS
}
