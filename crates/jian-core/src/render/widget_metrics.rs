//! Shared anatomy of labelled first-class widgets.
//!
//! Layout measurement (`layout::measure_text_leaf`) and every painter
//! (jian's `render::scene` and host design canvases) must agree on where a
//! labelled checkbox's label starts, or a `fit_content` control clips its
//! own text. Keeping the numbers and the box-side rule here gives them one
//! source of truth.

/// Intrinsic indicator side for a labelled checkbox whose height is itself
/// content-sized (no authored height).
pub const CHECKBOX_INDICATOR_MIN: f32 = 18.0;
/// Horizontal gap between the indicator and its adjacent label.
pub const CHECKBOX_LABEL_GAP: f32 = 8.0;
/// Font size of a checkbox / radio adjacent label (default family, 400).
pub const WIDGET_LABEL_FONT_SIZE: f32 = 14.0;
/// Font weight of a checkbox / radio adjacent label.
pub const WIDGET_LABEL_FONT_WEIGHT: u16 = 400;

/// Side of a labelled checkbox's square indicator inside a `width × height`
/// control. The indicator fills the control height (it never exceeds the
/// width), so an authored `height: 22` yields a 22px box, not the 18px
/// intrinsic one.
pub fn labelled_checkbox_indicator_side(width: f32, height: f32) -> f32 {
    width.min(height).max(0.0)
}

/// Content-sized width of a labelled checkbox: indicator + gap + label.
/// `control_height` is the resolved (or authored) control height, which
/// fixes the indicator side per [`labelled_checkbox_indicator_side`].
pub fn labelled_checkbox_fit_width(control_height: f32, label_width: f32) -> f32 {
    control_height.max(0.0) + CHECKBOX_LABEL_GAP + label_width.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_width_leaves_room_for_the_painted_label() {
        let height = 22.0;
        let label = 84.0;
        let width = labelled_checkbox_fit_width(height, label);
        let side = labelled_checkbox_indicator_side(width, height);
        // The painter starts the label at `side + gap` and clips at `width`.
        assert!(width - (side + CHECKBOX_LABEL_GAP) >= label);
    }
}
