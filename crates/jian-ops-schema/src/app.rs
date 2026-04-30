use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Storage,
    Network,
    Camera,
    Microphone,
    Location,
    Notifications,
    Clipboard,
    Biometric,
    FileSystem,
    Haptic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub name: String,
    pub version: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<Capability>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// C19 splash-frame config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splash: Option<SplashConfig>,

    /// C18 ASP web handshake postMessage origin allowlist.
    /// Only consulted by the web host; strict match; no wildcards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asp_allowed_origins: Option<Vec<String>>,

    /// Auto-update backend descriptor — same idea as `app.icon`:
    /// the schema declares the source of truth (which release feed
    /// to consult), the host crate translates it into a concrete
    /// `Updater` impl. `None` (the default) means the host's own
    /// fallback wins (typically `NullUpdater`). Only consulted by
    /// hosts that compile with their respective updater feature
    /// (`jian-host-desktop/updater`).
    ///
    /// TS-side, the type widens to a generic discriminated record so
    /// third-party hosts can declare their own kinds without forking
    /// `ops.ts`; the Rust side is the typed `UpdaterConfig` struct
    /// below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "export-ts",
        ts(type = "{ kind: string; [key: string]: unknown } | null")
    )]
    pub updater: Option<UpdaterConfig>,
}

/// Auto-updater feed descriptor — the schema declares **what** to
/// update from; the host crate translates the descriptor into a
/// concrete `Updater` trait impl. The shape is intentionally
/// minimal so this schema doesn't bake in any specific source
/// (GitHub, Sparkle appcast, AppImageUpdate, GitLab Releases,
/// custom HTTPS feed, in-house mirror, …): only `kind` is
/// well-known; everything else goes through `params`.
///
/// Hosts dispatch on `kind` and read backend-specific fields out
/// of `params`. Conventional kinds shipped by `jian-host-desktop`'s
/// `updater` feature:
///
/// - `"github_releases"` — pulls from a public GitHub repo.
///   Reads `params.owner`, `params.repo`, optional `params.target`
///   (falls back to host's detected triple), and optional
///   `params.binName` (falls back to host's binary name).
/// - `"disabled"` — explicit opt-out, even when the host crate is
///   built with the updater feature on. Useful for AppStore / MAS
///   builds whose channel forbids self-update.
///
/// Third-party hosts may add their own kinds (`"sparkle"`,
/// `"my_company_feed"`, …) without forking this schema; unknown
/// kinds fall back to `NullUpdater` at the host with a load-time
/// warning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
// Deliberately NOT exported to TS via ts-rs — the field-level override
// on `AppConfig::updater` widens the TypeScript surface to a generic
// `{ kind: string; [key: string]: unknown }` so TS authors aren't
// boxed into the Rust-side `params: Map` shape, which ts-rs can't
// represent under `serde(flatten)`.
#[serde(rename_all = "camelCase")]
pub struct UpdaterConfig {
    /// Backend selector. See type-level docs for conventional kinds.
    pub kind: String,
    /// Backend-specific params. Schema is opaque here — hosts that
    /// implement a given `kind` document the keys they consume.
    #[serde(default, flatten)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

impl UpdaterConfig {
    /// Conventional `kind` for the GitHub-Releases backend.
    pub const KIND_GITHUB_RELEASES: &'static str = "github_releases";
    /// Conventional `kind` for the explicit-disabled sentinel.
    pub const KIND_DISABLED: &'static str = "disabled";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SplashConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_ms: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_app_config() {
        let json = r#"{"name":"Counter","version":"1.0.0","id":"com.example.counter"}"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(a.name, "Counter");
        assert_eq!(a.id, "com.example.counter");
    }

    #[test]
    fn app_config_with_capabilities() {
        let json = r#"{"name":"X","version":"1","id":"y","capabilities":["storage","network"]}"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        let caps = a.capabilities.unwrap();
        assert_eq!(caps, vec![Capability::Storage, Capability::Network]);
    }

    #[test]
    fn updater_config_github_releases_round_trips() {
        let json = r#"{
          "name":"X","version":"1","id":"y",
          "updater": {
            "kind": "github_releases",
            "owner": "ZSeven-W",
            "repo": "jian",
            "target": "x86_64-apple-darwin",
            "binName": "jian"
          }
        }"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        let cfg = a.updater.unwrap();
        assert_eq!(cfg.kind, UpdaterConfig::KIND_GITHUB_RELEASES);
        assert_eq!(cfg.params["owner"], "ZSeven-W");
        assert_eq!(cfg.params["repo"], "jian");
        assert_eq!(cfg.params["target"], "x86_64-apple-darwin");
        assert_eq!(cfg.params["binName"], "jian");
    }

    #[test]
    fn updater_config_custom_kind_preserves_unknown_fields() {
        // Third-party host can ship its own kind without forking
        // the schema. The flatten map captures every sibling key.
        let json = r#"{
          "name":"X","version":"1","id":"y",
          "updater": {
            "kind": "sparkle",
            "feedUrl": "https://example.com/appcast.xml",
            "publicEdKey": "ABCD..."
          }
        }"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        let cfg = a.updater.unwrap();
        assert_eq!(cfg.kind, "sparkle");
        assert_eq!(cfg.params["feedUrl"], "https://example.com/appcast.xml");
        assert_eq!(cfg.params["publicEdKey"], "ABCD...");
    }

    #[test]
    fn updater_config_disabled_round_trips() {
        let json = r#"{
          "name":"X","version":"1","id":"y",
          "updater": { "kind": "disabled" }
        }"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        let cfg = a.updater.unwrap();
        assert_eq!(cfg.kind, UpdaterConfig::KIND_DISABLED);
        assert!(cfg.params.is_empty());
    }

    #[test]
    fn updater_config_default_is_none() {
        let json = r#"{"name":"X","version":"1","id":"y"}"#;
        let a: AppConfig = serde_json::from_str(json).unwrap();
        assert!(a.updater.is_none());
    }
}
