use serde::{Deserialize, Serialize};

// --- Blend mode ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct GradientStop {
    pub offset: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct ImageOriginalSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct ImageTransform {
    pub m00: f32,
    pub m01: f32,
    pub m02: f32,
    pub m10: f32,
    pub m11: f32,
    pub m12: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum ImageFillMode {
    Fill,
    Fit,
    Crop,
    Tile,
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PenFill {
    Solid(SolidFillBody),
    LinearGradient(LinearGradientBody),
    RadialGradient(RadialGradientBody),
    MeshGradient(MeshGradientBody),
    Shader(ShaderFillBody),
    Image(ImageFillBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

/// A single vertex of a `MeshGradient` grid. `row`/`col` are 0-based
/// indices into a `rows`×`cols` lattice; `color` is the Gouraud-shaded
/// colour anchored at that vertex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct MeshVertexStop {
    pub row: u32,
    pub col: u32,
    pub color: String,
}

/// Uniform-grid mesh gradient (v1). A `rows`×`cols` lattice of
/// `MeshVertexStop`s is Gouraud-interpolated across the node's
/// round-rect. Mirrors the sibling gradient bodies for the shared
/// `explain`/`opacity`/`blend_mode` tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct MeshGradientBody {
    pub rows: u32,
    pub cols: u32,
    pub stops: Vec<MeshVertexStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
}

/// One named uniform value bound into an SkSL shader fill via
/// `RuntimeShaderBuilder` at paint time. `untagged` so the wire form is
/// the bare JSON scalar/array the author wrote — a number is a `float`,
/// a number array is a `vec2`/`vec3`/`vec4`, and a hex string is a
/// `color` mapped to a `vec4` (premultiplied RGBA). The named-uniforms
/// map on `ShaderFillBody` is optional; a shader may take none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum ShaderUniformValue {
    /// `float` uniform.
    Float(f32),
    /// `vec2` / `vec3` / `vec4` uniform — the length picks the SkSL
    /// type (RuntimeShaderBuilder rejects a mismatched arity).
    Vec(Vec<f32>),
    /// `color` uniform — a `#rgb` / `#rrggbb` / `#rrggbbaa` hex string
    /// the backend expands into a `vec4` before binding.
    Color(String),
}

/// Native SkSL shader fill (v1). The `sksl` source is stored RAW and is
/// treated as untrusted: the renderer entrypoint is the SkSL signature
/// `half4 main(float2 fragCoord)`. On compile failure the backend
/// degrades to a visible solid fill (the first `color` uniform, else
/// mid-gray) and never panics. Mirrors the sibling gradient bodies for
/// the shared `opacity`/`blend_mode` tail.
///
/// Pencil-flavoured WebGL-GLSL import is an explicit follow-up, NOT v1;
/// v1 expects SkSL (Skia's GLSL dialect) verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ShaderFillBody {
    /// RAW SkSL source. Entrypoint: `half4 main(float2 fragCoord)`.
    pub sksl: String,
    /// Optional named-uniform map (`float` / `vec*` / `color`). A
    /// shader may declare none; absent or empty both mean "no uniforms".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uniforms: Option<std::collections::BTreeMap<String, ShaderUniformValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<BlendMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ImageFillBody {
    // Image-fill source — an `Arc`-shared string (`data:` URL or path),
    // same rationale as `ImageNode.src` (see `crate::node::ImageSrc`): a
    // multi-MB data URL here is otherwise cloned + content-compared on
    // every scene-cache refresh. Wire form stays a plain string.
    #[schemars(with = "String")]
    #[cfg_attr(feature = "export-ts", ts(as = "String"))]
    pub url: crate::node::ImageSrc,
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum StrokeThickness {
    Uniform(f32),
    PerSide([f32; 4]),
    Sided(SidedThickness),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    Inside,
    Center,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    Miter,
    Bevel,
    Round,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    None,
    Round,
    Square,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PenEffect {
    Blur(BlurBody),
    BackgroundBlur(BlurBody),
    #[serde(alias = "drop-shadow")]
    Shadow(ShadowBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct BlurBody {
    pub radius: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ShadowBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: String,
}

#[cfg(test)]
mod effect_visibility_tests {
    use super::{BlurBody, ShadowBody};

    #[test]
    fn effect_visibility_is_optional_and_round_trips() {
        let blur: BlurBody = serde_json::from_str(r#"{"radius":10}"#).unwrap();
        assert_eq!(blur.visible, None);
        assert_eq!(serde_json::to_string(&blur).unwrap(), r#"{"radius":10.0}"#);

        let shadow: ShadowBody = serde_json::from_str(
            r##"{"inner":false,"visible":false,"offsetX":0,"offsetY":4,"blur":8,"spread":0,"color":"#00000040"}"##,
        )
        .unwrap();
        assert_eq!(shadow.visible, Some(false));
        assert!(serde_json::to_string(&shadow)
            .unwrap()
            .contains(r#""visible":false"#));
    }
}

// --- Styled text segment ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum FontStyleKind {
    Normal,
    Italic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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
    fn pen_effect_accepts_ts_drop_shadow_alias() {
        let effect: PenEffect = serde_json::from_value(serde_json::json!({
            "type": "drop-shadow",
            "offsetX": 0,
            "offsetY": 8,
            "blur": 24,
            "spread": 0,
            "color": "#00000033"
        }))
        .unwrap();

        assert!(matches!(effect, PenEffect::Shadow(_)));
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
    fn mesh_gradient_roundtrip() {
        let json = r##"{"type":"mesh_gradient","rows":2,"cols":2,"stops":[{"row":0,"col":0,"color":"#f00"},{"row":0,"col":1,"color":"#0f0"},{"row":1,"col":0,"color":"#00f"},{"row":1,"col":1,"color":"#ff0"}]}"##;
        let f: PenFill = serde_json::from_str(json).unwrap();
        match &f {
            PenFill::MeshGradient(body) => {
                assert_eq!(body.rows, 2);
                assert_eq!(body.cols, 2);
                assert_eq!(body.stops.len(), 4);
                assert_eq!(body.stops[0].color, "#f00");
                assert_eq!(body.stops[3].row, 1);
                assert_eq!(body.stops[3].col, 1);
            }
            _ => panic!("wrong variant"),
        }
        let s = serde_json::to_string(&f).unwrap();
        let f2: PenFill = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }

    #[test]
    fn shader_fill_roundtrip() {
        let json = r##"{"type":"shader","sksl":"half4 main(float2 p){ return half4(1.0,0.0,0.0,1.0); }","uniforms":{"glow":0.5,"center":[0.5,0.5],"tint":"#ff00aa"}}"##;
        let f: PenFill = serde_json::from_str(json).unwrap();
        match &f {
            PenFill::Shader(body) => {
                assert!(body.sksl.contains("half4 main(float2 p)"));
                let uniforms = body.uniforms.as_ref().expect("uniforms map");
                assert_eq!(uniforms.get("glow"), Some(&ShaderUniformValue::Float(0.5)));
                assert_eq!(
                    uniforms.get("center"),
                    Some(&ShaderUniformValue::Vec(vec![0.5, 0.5]))
                );
                assert_eq!(
                    uniforms.get("tint"),
                    Some(&ShaderUniformValue::Color("#ff00aa".into()))
                );
            }
            _ => panic!("wrong variant"),
        }
        let s = serde_json::to_string(&f).unwrap();
        let f2: PenFill = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }

    #[test]
    fn shader_fill_without_uniforms_roundtrip() {
        let json = r##"{"type":"shader","sksl":"half4 main(float2 p){ return half4(0.0,0.0,0.0,1.0); }"}"##;
        let f: PenFill = serde_json::from_str(json).unwrap();
        match &f {
            PenFill::Shader(body) => assert!(body.uniforms.is_none()),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn shader_fill_rejects_missing_sksl() {
        // `sksl` is required — a shader fill with no source must fail to
        // deserialize rather than silently producing an empty program.
        let json = r##"{"type":"shader","uniforms":{"glow":0.5}}"##;
        let parsed: Result<PenFill, _> = serde_json::from_str(json);
        assert!(
            parsed.is_err(),
            "missing `sksl` must reject, got {parsed:?}"
        );
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
        let s: PenStroke = serde_json::from_str(r#"{"thickness":[1.0,2.0,3.0,4.0]}"#).unwrap();
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
