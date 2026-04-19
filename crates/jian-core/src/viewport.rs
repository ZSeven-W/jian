//! Viewport math — zoom, pan, screen ↔ scene transforms.

use crate::geometry::{point, rect, Affine2, Point, Rect, Size};

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub zoom: f32,
    pub pan: (f32, f32), // scene-coord offset applied before zoom
    pub size: Size,      // screen size in logical pixels
}

impl Viewport {
    pub fn new(size: Size) -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
            size,
        }
    }

    pub fn screen_to_scene(&self, p: Point) -> Point {
        point(p.x / self.zoom - self.pan.0, p.y / self.zoom - self.pan.1)
    }

    pub fn scene_to_screen(&self, p: Point) -> Point {
        point(
            (p.x + self.pan.0) * self.zoom,
            (p.y + self.pan.1) * self.zoom,
        )
    }

    pub fn visible_rect(&self) -> Rect {
        rect(
            -self.pan.0,
            -self.pan.1,
            self.size.width / self.zoom,
            self.size.height / self.zoom,
        )
    }

    pub fn transform(&self) -> Affine2 {
        Affine2::translation(self.pan.0, self.pan.1).then(&Affine2::scale(self.zoom, self.zoom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::size;

    fn vp(zoom: f32, pan: (f32, f32)) -> Viewport {
        Viewport {
            zoom,
            pan,
            size: size(800.0, 600.0),
        }
    }

    #[test]
    fn identity_roundtrip() {
        let v = vp(1.0, (0.0, 0.0));
        let p = point(100.0, 100.0);
        assert_eq!(v.scene_to_screen(v.screen_to_scene(p)), p);
    }

    #[test]
    fn zoom_doubles_scene_coords() {
        let v = vp(2.0, (0.0, 0.0));
        let screen = v.scene_to_screen(point(10.0, 5.0));
        assert_eq!(screen, point(20.0, 10.0));
    }

    #[test]
    fn pan_shifts() {
        let v = vp(1.0, (50.0, 20.0));
        let screen = v.scene_to_screen(point(0.0, 0.0));
        assert_eq!(screen, point(50.0, 20.0));
    }

    #[test]
    fn visible_rect_inverse_zoom() {
        let v = vp(2.0, (0.0, 0.0));
        let r = v.visible_rect();
        assert_eq!(r.size.width, 400.0); // 800 / 2
        assert_eq!(r.size.height, 300.0); // 600 / 2
    }
}
