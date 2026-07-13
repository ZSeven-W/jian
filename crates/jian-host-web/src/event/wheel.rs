//! W3C wheel units/sign mapping and DOM coordinate conversion.

use super::pointer;
use jian_core::geometry::{point, Point};
use jian_core::gesture::{Modifiers, ScrollMode, WheelEvent};
use web_sys::{HtmlCanvasElement, WheelEvent as DomWheelEvent};

pub fn map_delta(delta_x: f32, delta_y: f32, delta_z: f32, mode: u32) -> (Point, f32, ScrollMode) {
    (
        point(delta_x, -delta_y),
        delta_z,
        match mode {
            1 => ScrollMode::Line,
            2 => ScrollMode::Page,
            _ => ScrollMode::Pixel,
        },
    )
}

pub fn map_wheel(
    event: &DomWheelEvent,
    canvas: &HtmlCanvasElement,
    logical_width: f32,
    logical_height: f32,
    t_ms: u64,
) -> WheelEvent {
    let (delta, delta_z, mode) = map_delta(
        event.delta_x() as f32,
        event.delta_y() as f32,
        event.delta_z() as f32,
        event.delta_mode(),
    );
    let mut modifiers = Modifiers::empty();
    modifiers.set(Modifiers::SHIFT, event.shift_key());
    modifiers.set(Modifiers::CTRL, event.ctrl_key());
    modifiers.set(Modifiers::ALT, event.alt_key());
    modifiers.set(Modifiers::CMD, event.meta_key());
    WheelEvent {
        position: pointer::position(
            f64::from(event.client_x()),
            f64::from(event.client_y()),
            canvas,
            logical_width,
            logical_height,
        ),
        delta,
        delta_z,
        mode,
        modifiers,
        t_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverses_only_y_and_preserves_dom_units() {
        let (delta, z, mode) = map_delta(3.0, 12.0, 4.0, 1);
        assert_eq!(delta, point(3.0, -12.0));
        assert_eq!(z, 4.0);
        assert_eq!(mode, ScrollMode::Line);
        assert_eq!(map_delta(0.0, 1.0, 0.0, 2).2, ScrollMode::Page);
    }
}
