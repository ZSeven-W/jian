//! Normative PointerEvent coordinate and field mapping.

use jian_core::geometry::point;
use jian_core::gesture::{
    Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase,
};
use web_sys::{HtmlCanvasElement, PointerEvent as DomPointerEvent};

pub fn position(
    client_x: f64,
    client_y: f64,
    canvas: &HtmlCanvasElement,
    logical_width: f32,
    logical_height: f32,
) -> jian_core::geometry::Point {
    let bounds = canvas.get_bounding_client_rect();
    let scale_x = if bounds.width() > 0.0 {
        f64::from(logical_width) / bounds.width()
    } else {
        1.0
    };
    let scale_y = if bounds.height() > 0.0 {
        f64::from(logical_height) / bounds.height()
    } else {
        1.0
    };
    point(
        ((client_x - bounds.left()) * scale_x) as f32,
        ((client_y - bounds.top()) * scale_y) as f32,
    )
}

pub fn modifiers(event: &DomPointerEvent) -> Modifiers {
    let mut result = Modifiers::empty();
    result.set(Modifiers::SHIFT, event.shift_key());
    result.set(Modifiers::CTRL, event.ctrl_key());
    result.set(Modifiers::ALT, event.alt_key());
    result.set(Modifiers::CMD, event.meta_key());
    result
}

pub fn map_pointer(
    event: &DomPointerEvent,
    canvas: &HtmlCanvasElement,
    logical_width: f32,
    logical_height: f32,
    requested_phase: PointerPhase,
    t_ms: u64,
) -> PointerEvent {
    let kind = match event.pointer_type().as_str() {
        "mouse" => PointerKind::Mouse,
        "pen" => PointerKind::Pen,
        "touch" => PointerKind::Touch,
        _ => PointerKind::Stylus,
    };
    let phase = if requested_phase == PointerPhase::Move
        && kind == PointerKind::Mouse
        && event.buttons() == 0
    {
        PointerPhase::Hover
    } else {
        requested_phase
    };
    PointerEvent {
        id: PointerId(event.pointer_id().max(0) as u32),
        kind,
        phase,
        position: position(
            f64::from(event.client_x()),
            f64::from(event.client_y()),
            canvas,
            logical_width,
            logical_height,
        ),
        pressure: event.pressure(),
        buttons: MouseButtons::from_bits_truncate(event.buttons() as u32),
        modifiers: modifiers(event),
        tilt: Some((event.tilt_x() as f32, event.tilt_y() as f32)),
        t_ms,
    }
}
