//! Action → Capability mapping table (spec §A.8.2).
//!
//! Source of truth for "what capabilities does this action need?". Action
//! implementations that perform IO call `required_capabilities(name)` and
//! pass each returned capability through the `CapabilityGate` before
//! running.

use super::gate::Capability;

/// Return the list of capabilities an action needs.
///
/// Pure runtime actions (state, control flow, UI feedback, navigation)
/// return an empty slice. Unknown actions also return empty — the
/// registry lookup step will catch them before this mapping is consulted.
///
/// Tier-3 `call` is intentionally empty here: the `LogicProvider`
/// declares its own capability set per-module.
pub fn required_capabilities(action_name: &str) -> &'static [Capability] {
    match action_name {
        "fetch" | "open_url" => &[Capability::Network],
        "ws_connect" | "ws_send" | "ws_close" => &[Capability::Network],
        "share" => &[Capability::Network],

        "storage_set" | "storage_clear" | "storage_wipe" => &[Capability::Storage],

        "copy" | "paste" => &[Capability::Clipboard],

        "vibrate" | "haptic" => &[Capability::Haptic],
        "notify" => &[Capability::Notifications],

        "open_camera" => &[Capability::Camera],
        "open_microphone" => &[Capability::Microphone],
        "request_location" => &[Capability::Location],

        // Pure runtime — no IO.
        "set" | "reset" | "delete" | "toast" | "alert" | "confirm" | "focus" | "blur" | "push"
        | "replace" | "pop" | "if" | "for_each" | "parallel" | "race" | "delay" | "abort" => &[],

        // Tier-3 bridge — LogicProvider declares its own.
        "call" => &[],

        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_actions_map_to_network() {
        for a in [
            "fetch",
            "open_url",
            "ws_connect",
            "ws_send",
            "ws_close",
            "share",
        ] {
            assert_eq!(required_capabilities(a), &[Capability::Network], "{a}");
        }
    }

    #[test]
    fn storage_actions_map_to_storage() {
        for a in ["storage_set", "storage_clear", "storage_wipe"] {
            assert_eq!(required_capabilities(a), &[Capability::Storage], "{a}");
        }
    }

    #[test]
    fn os_sensitive_actions_map_per_table() {
        assert_eq!(required_capabilities("copy"), &[Capability::Clipboard]);
        assert_eq!(required_capabilities("paste"), &[Capability::Clipboard]);
        assert_eq!(required_capabilities("vibrate"), &[Capability::Haptic]);
        assert_eq!(required_capabilities("haptic"), &[Capability::Haptic]);
        assert_eq!(
            required_capabilities("notify"),
            &[Capability::Notifications]
        );
        assert_eq!(required_capabilities("open_camera"), &[Capability::Camera]);
        assert_eq!(
            required_capabilities("open_microphone"),
            &[Capability::Microphone]
        );
        assert_eq!(
            required_capabilities("request_location"),
            &[Capability::Location]
        );
    }

    #[test]
    fn pure_runtime_actions_need_nothing() {
        for a in [
            "set", "reset", "delete", "toast", "alert", "confirm", "focus", "blur", "push",
            "replace", "pop", "if", "for_each", "parallel", "race", "delay", "abort",
        ] {
            assert!(required_capabilities(a).is_empty(), "{a}");
        }
    }

    #[test]
    fn tier3_call_is_empty() {
        assert!(required_capabilities("call").is_empty());
    }

    #[test]
    fn unknown_action_is_empty() {
        assert!(required_capabilities("made_up_action").is_empty());
    }
}
