use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}
