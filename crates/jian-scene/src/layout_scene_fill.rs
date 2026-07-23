//! Paint-ready representation of a canonical node's complete fill stack.
//!
//! Canonical fill arrays are ordered front-to-back (the first entry is the
//! topmost layer). Painters therefore traverse this list in reverse.

use crate::layout_scene::{SceneGradient, SceneImageFit, SceneShader};
use jian_widgets::{Color, ImageAdjustments, ImageBlendMode};
use std::sync::Arc;

/// One resolved fill layer, including its own compositing mode.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneFillLayer {
    Solid {
        color: Color,
        blend_mode: ImageBlendMode,
    },
    Gradient {
        gradient: SceneGradient,
        blend_mode: ImageBlendMode,
    },
    Shader {
        shader: SceneShader,
        blend_mode: ImageBlendMode,
    },
    Image {
        src: Arc<str>,
        src_id: u64,
        fit: SceneImageFit,
        transform: Option<[f32; 6]>,
        original_size: Option<[f32; 2]>,
        tile_scale: f32,
        adjustments: ImageAdjustments,
        opacity: f32,
        blend_mode: ImageBlendMode,
    },
}

impl SceneFillLayer {
    /// Compositing mode applied when this layer lands over lower layers.
    pub fn blend_mode(&self) -> ImageBlendMode {
        match self {
            Self::Solid { blend_mode, .. }
            | Self::Gradient { blend_mode, .. }
            | Self::Shader { blend_mode, .. }
            | Self::Image { blend_mode, .. } => *blend_mode,
        }
    }

    /// Update a primary solid fill during an in-place editor paint patch.
    pub(crate) fn set_solid_color(&mut self, next: Color) -> bool {
        let Self::Solid { color, .. } = self else {
            return false;
        };
        *color = next;
        true
    }
}
