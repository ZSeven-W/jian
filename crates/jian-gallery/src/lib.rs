//! Interactive gallery for `jian-widgets`.

pub mod app;
#[cfg(all(feature = "desktop", not(target_os = "android")))]
pub mod desktop;
pub mod skia_painter;
pub mod touch;

pub use app::{GalleryApp, GalleryHit, GalleryLayout};
pub use touch::{GalleryGesture, TouchArena, TouchInput, TouchPhase};

#[cfg(test)]
mod tests {
    use super::*;
    use jian_widgets::{Color, Density, Painter, Point2D, Rect, TextLayout};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct RecordingPainter {
        texts: Vec<String>,
        fills: usize,
        clips: usize,
    }

    impl Painter for RecordingPainter {
        fn begin_frame(&mut self) {}
        fn end_frame(&mut self) {}
        fn fill_rect(&mut self, _rect: Rect, _color: Color) {
            self.fills += 1;
        }
        fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}
        fn draw_text(&mut self, layout: &TextLayout, _origin: Point2D) {
            if let Some(run) = layout.runs().first() {
                self.texts.push(run.content.clone());
            }
        }
        fn clip_rect(&mut self, _rect: Rect) {
            self.clips += 1;
        }
        fn stroke_line(&mut self, _from: Point2D, _to: Point2D, _color: Color, _width: f32) {}
        fn fill_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color) {
            self.fills += 1;
        }
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

    #[test]
    fn gallery_uses_touch_density_and_lays_out_every_component_family() {
        let app = GalleryApp::new();
        let viewport = Rect::xywh(0.0, 0.0, 800.0, 720.0);
        let layout = app.layout(viewport);

        assert_eq!(app.tokens().density, Density::Touch);
        assert_eq!(layout.buttons.len(), 8);
        assert!(layout.text_input.size.y >= 44.0);
        assert!(layout.text_area.size.y >= 88.0);
        assert!(layout.select_anchor.size.y >= 44.0);
        assert!(layout.menu_trigger.size.y >= 44.0);
        assert!(layout.scroll_view.size.y >= 132.0);
        assert_eq!(layout.switches.len(), 2);
        assert!(layout
            .switch_hits(&app.tokens())
            .iter()
            .all(|r| { r.size.x >= 44.0 && r.size.y >= 44.0 }));
        assert!(layout.dialog_button.size.y >= 44.0);
    }

    #[test]
    fn gallery_paints_all_component_families() {
        let mut app = GalleryApp::new();
        app.open_dialog();
        app.open_menu(Point2D::new(220.0, 260.0));
        app.open_select();
        let mut painter = RecordingPainter::default();

        app.paint(&mut painter, Rect::xywh(0.0, 0.0, 800.0, 720.0), 0);

        let joined = painter.texts.join("|");
        assert!(joined.contains("Primary"));
        assert!(joined.contains("设计输入"));
        assert!(joined.contains("Select option 01"));
        assert!(joined.contains("Menu item"));
        assert!(joined.contains("Scroll row 01"));
        assert!(joined.contains("Touch Gallery Dialog"));
        assert!(painter.clips >= 3);
        assert!(painter.fills > 20);
    }

    #[test]
    fn touch_arena_maps_tap_pan_and_long_press() {
        let start = Instant::now();
        let mut tap = TouchArena::new();
        assert_eq!(
            tap.handle(TouchInput::new(
                1,
                TouchPhase::Started,
                Point2D::new(10.0, 10.0),
                start,
            )),
            vec![GalleryGesture::Press(Point2D::new(10.0, 10.0))]
        );
        assert_eq!(
            tap.handle(TouchInput::new(
                1,
                TouchPhase::Ended,
                Point2D::new(11.0, 10.0),
                start + Duration::from_millis(80),
            )),
            vec![GalleryGesture::Tap(Point2D::new(11.0, 10.0))]
        );

        let mut pan = TouchArena::new();
        let _ = pan.handle(TouchInput::new(
            2,
            TouchPhase::Started,
            Point2D::new(0.0, 0.0),
            start,
        ));
        assert_eq!(
            pan.handle(TouchInput::new(
                2,
                TouchPhase::Moved,
                Point2D::new(0.0, 16.0),
                start + Duration::from_millis(40),
            )),
            vec![GalleryGesture::PanStart {
                position: Point2D::new(0.0, 16.0),
                delta: Point2D::new(0.0, 16.0),
            }]
        );
        assert_eq!(
            pan.handle(TouchInput::new(
                2,
                TouchPhase::Moved,
                Point2D::new(0.0, 30.0),
                start + Duration::from_millis(80),
            )),
            vec![GalleryGesture::PanDelta(Point2D::new(0.0, 14.0))]
        );

        let mut long = TouchArena::new();
        let _ = long.handle(TouchInput::new(
            3,
            TouchPhase::Started,
            Point2D::new(5.0, 6.0),
            start,
        ));
        assert_eq!(
            long.tick(start + Duration::from_millis(500)),
            vec![GalleryGesture::LongPress(Point2D::new(5.0, 6.0))]
        );
    }

    #[test]
    fn skia_widget_painter_renders_gallery_to_nonblank_surface() {
        let mut app = GalleryApp::new();
        let mut surface = jian_skia::SkiaSurface::new_raster(320, 240);
        {
            let mut painter = crate::skia_painter::SkiaWidgetPainter::new(&mut surface, 1.0);
            app.paint(&mut painter, Rect::xywh(0.0, 0.0, 320.0, 240.0), 0);
        }

        let mut rgba = vec![0; 320 * 240 * 4];
        assert!(surface.read_rgba8(&mut rgba));
        assert!(rgba
            .chunks_exact(4)
            .any(|px| px[0] != 0 || px[1] != 0 || px[2] != 0));
    }
}
