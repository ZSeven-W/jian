use super::Runtime;
use crate::geometry::{Affine2, Rect};
use crate::gesture::{Modifiers, PointerEvent, ScrollMode, SemanticEvent, WheelEvent};
use crate::render::ScenePaintCommand;

impl Runtime {
    /// Opt a standalone player into viewport scrolling. Input passed through
    /// the player entry points is in logical surface coordinates.
    pub fn enable_standalone_scroll(&mut self) {
        self.enable_standalone_motion();
        if let Some(playback) = self.playback.as_mut() {
            playback.scroll_y.get_or_insert(0.0);
        }
    }

    pub fn standalone_scroll_y(&self) -> f32 {
        self.playback
            .as_ref()
            .and_then(|p| p.scroll_y)
            .unwrap_or(0.0)
    }

    pub fn dispatch_standalone_pointer(&mut self, mut event: PointerEvent) -> Vec<SemanticEvent> {
        event.position.y += self.standalone_scroll_y();
        self.dispatch_pointer(event)
    }

    /// Deliver authored onScroll handlers at the visible point before moving
    /// the viewport. Authored handlers still receive the original wheel units.
    pub fn dispatch_standalone_wheel(&mut self, mut event: WheelEvent) {
        let old = self.standalone_scroll_y();
        event.position.y += old;
        let delta = wheel_pixels(&event, self.viewport.size.height);
        self.dispatch_wheel(event);
        if !self.input_frozen() && delta != 0.0 && self.update_standalone_scroll(old + delta) {
            // Start in-view clips at the scroll event's time, including when
            // several wheel events arrive before the next presented frame.
            self.tick_playback();
        }
    }

    pub fn standalone_surface_rect(&self, mut scene: Rect) -> Rect {
        scene.origin.y -= self.standalone_scroll_y();
        scene
    }

    pub(in crate::runtime) fn reset_standalone_scroll(&mut self) {
        if let Some(offset) = self.playback.as_mut().and_then(|p| p.scroll_y.as_mut()) {
            *offset = 0.0;
        }
    }

    pub(in crate::runtime) fn clamp_standalone_scroll(&mut self) {
        let offset = self.standalone_scroll_y();
        if offset > 0.0 {
            self.update_standalone_scroll(offset);
        }
    }

    fn update_standalone_scroll(&mut self, desired: f32) -> bool {
        if self.playback.as_ref().and_then(|p| p.scroll_y).is_none() {
            return false;
        }
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let theme = crate::render::WidgetTheme::default();
        let focus = self.focused_widget_id();
        let widgets = self.playback_widget_context(&theme, focus.as_deref());
        let height = self.viewport.size.height;
        let bottom =
            crate::render::scene_commands::content_bottom(doc, &self.layout, &self.state, &widgets);
        let next = desired.clamp(0.0, (bottom - height).max(0.0));
        let offset = self
            .playback
            .as_mut()
            .and_then(|p| p.scroll_y.as_mut())
            .expect("scroll enabled");
        if *offset == next {
            return false;
        }
        *offset = next;
        self.mark_dirty();
        true
    }

    pub(super) fn scroll_paint_commands(
        &self,
        mut commands: Vec<ScenePaintCommand>,
    ) -> Vec<ScenePaintCommand> {
        let offset = self.standalone_scroll_y();
        if offset != 0.0 {
            commands.insert(
                0,
                ScenePaintCommand::PushTransform(Affine2::translation(0.0, -offset)),
            );
            commands.push(ScenePaintCommand::Pop);
        }
        commands
    }
}

fn wheel_pixels(event: &WheelEvent, viewport_height: f32) -> f32 {
    if !event.delta.y.is_finite()
        || event
            .modifiers
            .intersects(Modifiers::CTRL | Modifiers::CMD | Modifiers::SHIFT)
    {
        return 0.0;
    }
    let scale = match event.mode {
        ScrollMode::Pixel => 1.0,
        ScrollMode::Line => 16.0,
        ScrollMode::Page => viewport_height,
    };
    -event.delta.y * scale
}
