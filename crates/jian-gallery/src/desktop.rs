use crate::app::GalleryApp;
use crate::skia_painter::SkiaWidgetPainter;
use crate::touch::{GalleryGesture, TouchArena, TouchInput, TouchPhase};
use jian_skia::SkiaSurface;
use jian_widgets::{Painter, Point2D, Rect};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase as WinitTouchPhase};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

pub fn run() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    let mut app = DesktopGallery::default();
    event_loop.run_app(&mut app)
}

#[derive(Default)]
struct DesktopGallery {
    app: GalleryApp,
    window: Option<Rc<Window>>,
    surface: Option<SoftbufferState>,
    size: (u32, u32),
    scale: f64,
    cursor: Point2D,
    mouse_down: bool,
    touch: TouchArena,
    started_at: Option<Instant>,
    last_redraw_ms: u64,
}

struct SoftbufferState {
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    skia: SkiaSurface,
}

impl DesktopGallery {
    fn viewport(&self) -> Rect {
        Rect::xywh(
            0.0,
            0.0,
            self.size.0 as f32 / self.scale as f32,
            self.size.1 as f32 / self.scale as f32,
        )
    }

    fn now_ms(&self) -> u64 {
        self.started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    fn ms_at(&self, instant: Instant) -> u64 {
        self.started_at
            .map(|t| instant.saturating_duration_since(t).as_millis() as u64)
            .unwrap_or(0)
    }

    fn ensure_surface(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let width = self.size.0.max(1);
        let height = self.size.1.max(1);
        let state = self.surface.get_or_insert_with(|| {
            let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
            let surface =
                softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
            SoftbufferState {
                surface,
                skia: SkiaSurface::new_raster(width as i32, height as i32),
            }
        });
        let _ = state.surface.resize(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        );
        if state.skia.width() != width as i32 || state.skia.height() != height as i32 {
            state.skia = SkiaSurface::new_raster(width as i32, height as i32);
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let viewport = self.viewport();
        let now_ms = self.now_ms();
        let Some(state) = self.surface.as_mut() else {
            return;
        };
        {
            let mut painter = SkiaWidgetPainter::new(&mut state.skia, self.scale as f32);
            painter.begin_frame();
            self.app.paint(&mut painter, viewport, now_ms);
            painter.end_frame();
        }
        self.last_redraw_ms = now_ms;

        let (w, h) = self.size;
        let mut rgba = vec![0; w as usize * h as usize * 4];
        if !state.skia.read_rgba8(&mut rgba) {
            return;
        }
        let Ok(mut buffer) = state.surface.buffer_mut() else {
            return;
        };
        for (i, pixel) in buffer.iter_mut().enumerate() {
            let r = rgba[i * 4] as u32;
            let g = rgba[i * 4 + 1] as u32;
            let b = rgba[i * 4 + 2] as u32;
            *pixel = (r << 16) | (g << 8) | b;
        }
        if let Some(window) = self.window.as_ref() {
            window.pre_present_notify();
        }
        let _ = buffer.present();
    }

    fn logical_point(&self, p: PhysicalPosition<f64>) -> Point2D {
        Point2D::new((p.x / self.scale) as f32, (p.y / self.scale) as f32)
    }

    fn handle_touch_gesture(&mut self, gesture: GalleryGesture) {
        let viewport = self.viewport();
        let now_ms = self.now_ms();
        match gesture {
            GalleryGesture::Press(point) => {
                self.cursor = point;
                self.app.press(point, viewport, now_ms);
            }
            GalleryGesture::Tap(point) => {
                self.cursor = point;
                self.app.release(point, viewport);
            }
            GalleryGesture::PanStart { position, delta } => {
                self.cursor = position;
                self.app.cancel_press();
                self.app.scroll_at(position, -delta.y, viewport);
            }
            GalleryGesture::PanDelta(delta) => {
                self.app.scroll_at(self.cursor, -delta.y, viewport);
            }
            GalleryGesture::LongPress(point) => {
                self.cursor = point;
                self.app.cancel_press();
                self.app.open_menu(point);
            }
            GalleryGesture::PanEnd | GalleryGesture::Cancel => {
                self.app.cancel_press();
            }
        }
        self.request_redraw();
    }

    fn handle_touch_input(&mut self, input: TouchInput) {
        self.cursor = input.position;
        for gesture in self.touch.handle(input) {
            self.handle_touch_gesture(gesture);
        }
    }

    fn next_wake_deadline(&self, now: Instant) -> Option<Instant> {
        let mut deadline = self.touch.next_tick_at();
        if let Some(started_at) = self.started_at {
            if let Some(next_blink_ms) = self.app.next_blink_flip_ms(self.ms_at(now)) {
                let next_blink = started_at + Duration::from_millis(next_blink_ms);
                deadline = Some(deadline.map_or(next_blink, |current| current.min(next_blink)));
            }
        }
        deadline
    }

    fn frame_due(&self, now: Instant) -> bool {
        let now_ms = self.ms_at(now);
        self.app
            .next_blink_flip_ms(self.last_redraw_ms)
            .is_some_and(|next_blink_ms| next_blink_ms <= now_ms)
    }

    fn update_control_flow(&self, event_loop: &ActiveEventLoop, now: Instant) {
        if let Some(deadline) = self.next_wake_deadline(now) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

impl ApplicationHandler for DesktopGallery {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        self.started_at = Some(Instant::now());
        let attrs = Window::default_attributes()
            .with_title("jian-widgets gallery")
            .with_inner_size(LogicalSize::new(920.0, 720.0));
        let window = event_loop
            .create_window(attrs)
            .expect("create jian-gallery window");
        self.scale = window.scale_factor();
        let size = window.inner_size();
        self.size = (size.width.max(1), size.height.max(1));
        self.window = Some(Rc::new(window));
        self.ensure_surface();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(size) => {
                self.size = (size.width.max(1), size.height.max(1));
                self.ensure_surface();
                self.request_redraw();
            }
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                self.request_redraw();
            }
            winit::event::WindowEvent::RedrawRequested => self.redraw(),
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.logical_point(position);
                self.app.set_hover(self.cursor, self.viewport());
                self.request_redraw();
            }
            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                let viewport = self.viewport();
                if button == MouseButton::Left {
                    self.mouse_down = state == ElementState::Pressed;
                    if self.mouse_down {
                        self.app.press(self.cursor, viewport, self.now_ms());
                    } else {
                        self.app.release(self.cursor, viewport);
                    }
                    self.request_redraw();
                } else if button == MouseButton::Right && state == ElementState::Released {
                    self.app.open_menu(self.cursor);
                    self.request_redraw();
                }
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 28.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y / self.scale) as f32,
                };
                self.app.scroll_at(self.cursor, dy, self.viewport());
                self.request_redraw();
            }
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed =>
            {
                match event.logical_key {
                    Key::Named(NamedKey::Backspace) => self.app.backspace(self.now_ms()),
                    Key::Character(ref s) if !s.is_empty() => {
                        self.app.type_text(s.as_str(), self.now_ms());
                    }
                    _ => {}
                }
                self.request_redraw();
            }
            winit::event::WindowEvent::Touch(touch) => {
                let phase = match touch.phase {
                    WinitTouchPhase::Started => TouchPhase::Started,
                    WinitTouchPhase::Moved => TouchPhase::Moved,
                    WinitTouchPhase::Ended => TouchPhase::Ended,
                    WinitTouchPhase::Cancelled => TouchPhase::Cancelled,
                };
                self.handle_touch_input(TouchInput::new(
                    touch.id,
                    phase,
                    self.logical_point(touch.location),
                    Instant::now(),
                ));
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        for gesture in self.touch.tick(now) {
            self.handle_touch_gesture(gesture);
        }
        if self.frame_due(now) {
            self.request_redraw();
        }
        self.update_control_flow(event_loop, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_widgets::{Color, Painter, TextLayout};
    use std::time::Duration;

    #[derive(Default)]
    struct TextProbe {
        rows: Vec<(String, Point2D)>,
    }

    impl Painter for TextProbe {
        fn begin_frame(&mut self) {}
        fn end_frame(&mut self) {}
        fn fill_rect(&mut self, _rect: Rect, _color: Color) {}
        fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}
        fn draw_text(&mut self, layout: &TextLayout, origin: Point2D) {
            if let Some(run) = layout.runs().first() {
                self.rows.push((run.content.clone(), origin));
            }
        }
        fn clip_rect(&mut self, _rect: Rect) {}
        fn stroke_line(&mut self, _from: Point2D, _to: Point2D, _color: Color, _width: f32) {}
        fn fill_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color) {}
        fn stroke_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color, _width: f32) {}
        fn stroke_svg_path(
            &mut self,
            _d: &str,
            _top_left: Point2D,
            _size: f32,
            _color: Color,
            _width: f32,
        ) {
        }
        fn save(&mut self) {}
        fn restore(&mut self) {}
        fn translate(&mut self, _offset: Point2D) {}
        fn resize(&mut self, _width: u32, _height: u32) {}
        fn dpi_scale(&self) -> f32 {
            1.0
        }
    }

    fn sized_gallery() -> DesktopGallery {
        DesktopGallery {
            size: (920, 720),
            scale: 1.0,
            ..DesktopGallery::default()
        }
    }

    fn painted_texts(gallery: &mut DesktopGallery) -> Vec<(String, Point2D)> {
        let mut probe = TextProbe::default();
        gallery
            .app
            .paint(&mut probe, gallery.viewport(), gallery.now_ms());
        probe.rows
    }

    fn scroll_row_y(gallery: &mut DesktopGallery, label: &str) -> f32 {
        let rows = painted_texts(gallery);
        rows.iter()
            .find_map(|(text, origin)| text.contains(label).then_some(origin.y))
            .unwrap_or_else(|| panic!("scroll row {label} is painted; got {rows:?}"))
    }

    #[test]
    fn touch_input_uses_touch_position_for_pan_delta_scrolling() {
        let start_time = Instant::now();
        let mut gallery = sized_gallery();
        let scroll_view = gallery.app.layout(gallery.viewport()).scroll_view;
        let start = Point2D::new(scroll_view.origin.x + 24.0, scroll_view.origin.y + 110.0);

        let before = scroll_row_y(&mut gallery, "Scroll row 03");
        gallery.handle_touch_input(TouchInput::new(7, TouchPhase::Started, start, start_time));
        gallery.handle_touch_input(TouchInput::new(
            7,
            TouchPhase::Moved,
            Point2D::new(start.x, start.y - 16.0),
            start_time + Duration::from_millis(40),
        ));
        let after_start = scroll_row_y(&mut gallery, "Scroll row 03");
        gallery.handle_touch_input(TouchInput::new(
            7,
            TouchPhase::Moved,
            Point2D::new(start.x, start.y - 44.0),
            start_time + Duration::from_millis(80),
        ));
        let after_delta = scroll_row_y(&mut gallery, "Scroll row 03");

        assert!(
            after_start < before,
            "pan start should move the scroll content"
        );
        assert!(
            after_delta < after_start - 12.0,
            "pan delta should keep scrolling from the touch position"
        );
    }

    #[test]
    fn active_touch_schedules_event_loop_wake_for_long_press() {
        let start_time = Instant::now();
        let mut gallery = sized_gallery();
        let scroll_view = gallery.app.layout(gallery.viewport()).scroll_view;
        let start = Point2D::new(scroll_view.origin.x + 24.0, scroll_view.origin.y + 80.0);

        gallery.handle_touch_input(TouchInput::new(8, TouchPhase::Started, start, start_time));

        assert_eq!(
            gallery.next_wake_deadline(start_time),
            Some(start_time + Duration::from_millis(500))
        );
    }

    #[test]
    fn focused_text_input_schedules_caret_blink_wake() {
        let start_time = Instant::now();
        let mut gallery = sized_gallery();
        gallery.started_at = Some(start_time);
        gallery.last_redraw_ms = 100;
        let input = gallery.app.layout(gallery.viewport()).text_input;
        let point = Point2D::new(
            input.origin.x + input.size.x * 0.5,
            input.origin.y + input.size.y * 0.5,
        );

        gallery.app.press(point, gallery.viewport(), 100);

        assert_eq!(
            gallery.next_wake_deadline(start_time + Duration::from_millis(100)),
            Some(start_time + Duration::from_millis(600))
        );
        assert!(!gallery.frame_due(start_time + Duration::from_millis(599)));
        assert!(gallery.frame_due(start_time + Duration::from_millis(600)));
    }

    #[test]
    fn pan_start_uses_initial_drag_direction() {
        let start_time = Instant::now();
        let mut gallery = sized_gallery();
        let scroll_view = gallery.app.layout(gallery.viewport()).scroll_view;
        let start = Point2D::new(scroll_view.origin.x + 24.0, scroll_view.origin.y + 80.0);
        gallery.app.scroll_at(start, 80.0, gallery.viewport());

        let before = scroll_row_y(&mut gallery, "Scroll row 03");
        gallery.handle_touch_input(TouchInput::new(9, TouchPhase::Started, start, start_time));
        gallery.handle_touch_input(TouchInput::new(
            9,
            TouchPhase::Moved,
            Point2D::new(start.x, start.y + 16.0),
            start_time + Duration::from_millis(40),
        ));
        let after_start = scroll_row_y(&mut gallery, "Scroll row 03");

        assert!(
            after_start > before,
            "downward drag should move scroll content downward on pan start"
        );
    }

    #[test]
    fn non_tap_touch_end_clears_pressed_target() {
        let mut gallery = sized_gallery();
        let dialog_button = gallery.app.layout(gallery.viewport()).dialog_button;
        let point = Point2D::new(
            dialog_button.origin.x + dialog_button.size.x * 0.5,
            dialog_button.origin.y + dialog_button.size.y * 0.5,
        );

        gallery.handle_touch_gesture(GalleryGesture::Press(point));
        gallery.handle_touch_gesture(GalleryGesture::PanEnd);
        gallery.handle_touch_gesture(GalleryGesture::Tap(point));

        assert!(
            painted_texts(&mut gallery)
                .iter()
                .all(|(text, _)| text != "Touch Gallery Dialog"),
            "a stale pressed target must not activate after a pan end"
        );
    }
}
