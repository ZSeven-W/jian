use super::base::PenNodeBase;
use super::container::CornerRadius;
use super::image_src::ImageSrc;
use super::video::VideoMeta;
use crate::sizing::SizingBehavior;
use crate::style::PenEffect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum ImageFitMode {
    Fill,
    Fit,
    Crop,
    Tile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ImageNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    // Image source — an `Arc`-shared string (`data:` URL or path). See
    // `ImageSrc` for why it is reference-counted. On the wire it is a
    // plain string; `with`/`as` keep the schema + TS exports reporting
    // `string` (a non-doc `//` comment avoids adding a schema description,
    // so the tracked `ops.schema.json` stays byte-identical).
    #[schemars(with = "String")]
    #[cfg_attr(feature = "export-ts", ts(as = "String"))]
    pub src: ImageSrc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_fit: Option<ImageFitMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(flatten)]
    pub limits: crate::sizing::SizeLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<CornerRadius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::PenNode;
    use crate::style::BlendMode;

    #[test]
    fn legacy_image_blend_mode_uses_the_shared_node_base_once() {
        let json = r#"{"id":"image-1","src":"data:image/png;base64,AA==","blendMode":"multiply"}"#;
        let image: ImageNode = serde_json::from_str(json).expect("legacy image node");
        assert_eq!(image.base.blend_mode, Some(BlendMode::Multiply));

        let serialized = serde_json::to_string(&image).expect("serialize image node");
        assert_eq!(serialized.matches("\"blendMode\"").count(), 1);
    }

    #[test]
    fn video_metadata_round_trips_all_playback_flags() {
        let json = r#"{
            "type":"image",
            "id":"hero",
            "src":"data:image/png;base64,AA==",
            "video": {
                "src":"https://example.com/hero.mp4",
                "autoplay":true,
                "loop":true,
                "muted":true,
                "holdLastFrame":true,
                "clickToReplay":true,
                "videoPrompt":"cinematic mountain flyover"
            }
        }"#;
        let PenNode::Image(image) = serde_json::from_str(json).expect("video image") else {
            panic!("expected image node");
        };
        let video = image.video.as_ref().expect("video metadata");
        assert_eq!(video.src, "https://example.com/hero.mp4");
        assert!(video.autoplay);
        assert!(video.r#loop);
        assert!(video.muted);
        assert!(video.hold_last_frame);
        assert!(video.click_to_replay);
        assert_eq!(
            video.video_prompt.as_deref(),
            Some("cinematic mountain flyover")
        );
        let serialized = serde_json::to_string(&PenNode::Image(image)).expect("serialize image");
        assert!(serialized.contains("\"video\""));
        assert!(serialized.contains("\"holdLastFrame\":true"));
        assert!(serialized.contains("\"clickToReplay\":true"));
    }

    #[test]
    fn absent_or_null_video_metadata_is_not_serialized() {
        for suffix in ["", r#", "video": null"#] {
            let json = format!(r#"{{"type":"image","id":"hero","src":"poster.png"{suffix}}}"#);
            let node: PenNode = serde_json::from_str(&json).expect("image node");
            let serialized = serde_json::to_string(&node).expect("serialize image");
            assert!(!serialized.contains("\"video\""), "{serialized}");
        }
    }
}
