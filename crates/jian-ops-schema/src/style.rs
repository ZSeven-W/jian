use serde::{Deserialize, Serialize};

// --- Blend mode ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Normal,
    Darken,
    Multiply,
    Screen,
    Overlay,
    Lighten,
    Difference,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

// --- Fills ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageOriginalSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageTransform {
    pub m00: f32,
    pub m01: f32,
    pub m02: f32,
    pub m10: f32,
    pub m11: f32,
    pub m12: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFillMode {
    Fill,
    Fit,
    Crop,
    Tile,
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PenFill {
    Solid(SolidFillBody),
    LinearGradient(LinearGradientBody),
    RadialGradient(RadialGradientBody),
    Image(ImageFillBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolidFillBody {
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearGradientBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    pub stops: Vec<GradientStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadialGradientBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cx: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cy: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    pub stops: Vec<GradientStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFillBody {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ImageFillMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_size: Option<ImageOriginalSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ImageTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<f32>,
}

// --- Stroke ---

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidedThickness {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrokeThickness {
    Uniform(f32),
    PerSide([f32; 4]),
    Sided(SidedThickness),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    Inside,
    Center,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    Miter,
    Bevel,
    Round,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    None,
    Round,
    Square,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenStroke {
    pub thickness: StrokeThickness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<StrokeAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<StrokeJoin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<StrokeCap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_pattern: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_offset: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
}

// --- Effect ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PenEffect {
    Blur(BlurBody),
    BackgroundBlur(BlurBody),
    Shadow(ShadowBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlurBody {
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<bool>,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: String,
}

// --- Styled text segment ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontStyleKind {
    Normal,
    Italic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyledTextSegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_style: Option<FontStyleKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_mode_roundtrip() {
        let json = r#""multiply""#;
        let m: BlendMode = serde_json::from_str(json).unwrap();
        assert_eq!(m, BlendMode::Multiply);
        assert_eq!(serde_json::to_string(&m).unwrap(), json);
    }

    #[test]
    fn solid_fill_roundtrip() {
        let json = r##"{"type":"solid","color":"#ff0000","opacity":0.5}"##;
        let f: PenFill = serde_json::from_str(json).unwrap();
        match &f {
            PenFill::Solid(body) => {
                assert_eq!(body.color, "#ff0000");
                assert_eq!(body.opacity, Some(0.5));
            }
            _ => panic!("wrong variant"),
        }
        let s = serde_json::to_string(&f).unwrap();
        let f2: PenFill = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }

    #[test]
    fn linear_gradient_roundtrip() {
        let json = r##"{"type":"linear_gradient","angle":90.0,"stops":[{"offset":0.0,"color":"#000"},{"offset":1.0,"color":"#fff"}]}"##;
        let f: PenFill = serde_json::from_str(json).unwrap();
        assert!(matches!(f, PenFill::LinearGradient(_)));
        let s = serde_json::to_string(&f).unwrap();
        let f2: PenFill = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }

    #[test]
    fn image_fill_roundtrip() {
        let json = r#"{"type":"image","url":"data:image/png;base64,...","mode":"crop"}"#;
        let f: PenFill = serde_json::from_str(json).unwrap();
        assert!(matches!(f, PenFill::Image(_)));
    }

    #[test]
    fn stroke_thickness_uniform() {
        let s: PenStroke = serde_json::from_str(r#"{"thickness":2.0}"#).unwrap();
        assert!(matches!(s.thickness, StrokeThickness::Uniform(2.0)));
    }

    #[test]
    fn stroke_thickness_per_side() {
        let s: PenStroke =
            serde_json::from_str(r#"{"thickness":[1.0,2.0,3.0,4.0]}"#).unwrap();
        assert!(matches!(
            s.thickness,
            StrokeThickness::PerSide([1.0, 2.0, 3.0, 4.0])
        ));
    }

    #[test]
    fn shadow_effect_roundtrip() {
        let json = r##"{"type":"shadow","offsetX":2.0,"offsetY":4.0,"blur":8.0,"spread":0.0,"color":"#00000080"}"##;
        let e: PenEffect = serde_json::from_str(json).unwrap();
        assert!(matches!(e, PenEffect::Shadow(_)));
    }

    #[test]
    fn blur_effect_roundtrip() {
        let json = r#"{"type":"blur","radius":10.0}"#;
        let e: PenEffect = serde_json::from_str(json).unwrap();
        assert!(matches!(e, PenEffect::Blur(_)));
    }
}
