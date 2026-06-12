//! Scroll-offset math shared by scrollable views.
//!
//! Pure functions over content height, view height, and offset. Widget
//! layers decide policy such as pixels per wheel notch or momentum.

/// Scrollable offset in px. 0 means top.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollState {
    pub offset: f32,
}

/// Scrollbar thumb geometry in track-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thumb {
    pub offset: f32,
    pub len: f32,
}

pub fn max_offset(content: f32, view: f32) -> f32 {
    (content - view).max(0.0)
}

impl ScrollState {
    pub fn clamp(&mut self, content: f32, view: f32) {
        self.offset = self.offset.clamp(0.0, max_offset(content, view));
    }

    pub fn scroll_by(&mut self, delta: f32, content: f32, view: f32) {
        self.offset += delta;
        self.clamp(content, view);
    }

    pub fn thumb(&self, track: f32, content: f32, view: f32, min_len: f32) -> Option<Thumb> {
        if content <= view || view <= 0.0 || track <= 0.0 {
            return None;
        }
        let len = (track * view / content).clamp(min_len.min(track), track);
        let range = track - len;
        let max = max_offset(content, view);
        let t = if max > 0.0 {
            (self.offset / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(Thumb {
            offset: range * t,
            len,
        })
    }

    pub fn set_from_thumb(
        &mut self,
        thumb_top: f32,
        track: f32,
        content: f32,
        view: f32,
        min_len: f32,
    ) {
        if let Some(t) = self.thumb(track, content, view, min_len) {
            let range = (track - t.len).max(f32::EPSILON);
            self.offset = (thumb_top / range).clamp(0.0, 1.0) * max_offset(content, view);
        }
    }

    pub fn reveal(&mut self, item_top: f32, item_bottom: f32, view: f32) {
        if item_top < self.offset {
            self.offset = item_top;
        } else if item_bottom > self.offset + view {
            self.offset = item_bottom - view;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_when_content_fits() {
        let s = ScrollState::default();
        assert_eq!(s.thumb(100.0, 50.0, 80.0, 24.0), None);
    }

    #[test]
    fn thumb_len_proportional_and_clamped() {
        let s = ScrollState::default();
        let t = s.thumb(200.0, 400.0, 100.0, 24.0).unwrap();
        assert!((t.len - 50.0).abs() < 0.01);
        let t = s.thumb(200.0, 10_000.0, 100.0, 24.0).unwrap();
        assert_eq!(t.len, 24.0);
    }

    #[test]
    fn thumb_drag_round_trips() {
        let s = ScrollState { offset: 120.0 };
        let t = s.thumb(200.0, 500.0, 100.0, 24.0).unwrap();
        let mut s2 = ScrollState::default();
        s2.set_from_thumb(t.offset, 200.0, 500.0, 100.0, 24.0);
        assert!((s2.offset - 120.0).abs() < 0.5);
    }

    #[test]
    fn clamp_and_reveal() {
        let mut s = ScrollState { offset: 999.0 };
        s.clamp(300.0, 100.0);
        assert_eq!(s.offset, 200.0);
        s.reveal(10.0, 30.0, 100.0);
        assert_eq!(s.offset, 10.0);
        s.reveal(150.0, 190.0, 100.0);
        assert_eq!(s.offset, 90.0);
    }
}
