//! Clock, action sink, and screen compositing for standalone render hosts.

use super::Runtime;
use crate::action::services::{AnimationSink, Router};
use crate::binding::InvalidationKind;
use crate::geometry::{rect, Affine2};
use crate::gesture::pointer::{PointerEvent, PointerKind, PointerPhase};
use crate::motion::screen_transition::TransitionEffect;
use crate::motion::{
    InViewWindow, MotionActionSink, MotionOverrides, MotionSceneView, MotionTriggers,
    NavigationKind, ResolvedScreenTransition, TimedTransition, TransitionScreen,
};
use crate::render::scene_motion::{PlaybackInteraction, RuntimeMotionView, SceneMotion};
use crate::render::{ScenePaintCommand, WidgetRenderCtx, WidgetTheme};
use jian_ops_schema::motion::MotionPreference;
use std::rc::Rc;

pub(super) struct Playback {
    sink: MotionActionSink,
    triggers: MotionTriggers,
    generation: Option<u64>,
    reduced: bool,
    window: Option<InViewWindow>,
    presentation_size: Option<(f32, f32)>,
    transition: Option<ScreenChange>,
    stack_depth: Option<usize>,
    interaction: PlaybackInteraction,
    has_style_transitions: bool,
}

pub(crate) struct ScreenChange {
    resolved: ResolvedScreenTransition,
    clock: TimedTransition,
    outgoing: Vec<ScenePaintCommand>,
    painted_at: Option<u64>,
}

impl Runtime {
    /// Enable document motion and install its action sink. The host drives it
    /// with the same monotonic clock as the rest of Runtime.
    pub fn enable_standalone_motion(&mut self) {
        if self.playback.is_none() {
            let playback = Playback {
                sink: Default::default(),
                triggers: Default::default(),
                generation: None,
                reduced: false,
                window: None,
                presentation_size: None,
                transition: None,
                stack_depth: None,
                interaction: Default::default(),
                has_style_transitions: false,
            };
            self.set_animation_sink(Rc::new(playback.sink.clone()));
            self.playback = Some(playback);
        }
    }

    pub fn set_standalone_reduce_motion(&mut self, reduced: bool) {
        if let Some(playback) = &mut self.playback {
            if playback.reduced != reduced {
                playback.reduced = reduced;
                playback.generation = None;
                playback.transition = None;
                self.mark_dirty();
            }
        }
    }

    /// Supply the physical host's logical viewport independently of legacy layout.
    pub fn set_standalone_presentation_size(&mut self, size: (f32, f32)) {
        if let Some(playback) = &mut self.playback {
            if playback.presentation_size != Some(size) {
                playback.presentation_size = Some(size);
                self.mark_dirty();
            }
        }
    }

    pub fn set_standalone_in_view_window(&mut self, window: Option<InViewWindow>) {
        if let Some(playback) = &mut self.playback {
            if playback.window != window {
                playback.window = window;
                self.mark_dirty();
            }
        }
    }

    pub fn standalone_motion_overrides(&self) -> MotionOverrides {
        self.playback
            .as_ref()
            .map(|p| p.sink.timeline.overrides())
            .unwrap_or_default()
    }

    /// When no overlay is active, hosts with a legacy flat renderer can keep
    /// their original steady frame path.
    pub fn standalone_has_motion_paint(&self) -> bool {
        self.playback.as_ref().is_some_and(|p| {
            p.transition.is_some() || p.has_style_transitions || p.sink.timeline.has_visual_state()
        })
    }

    pub(crate) fn tick_playback(&mut self) {
        let Some(mut playback) = self.playback.take() else {
            return;
        };
        if self.document.as_ref().is_some_and(|doc| {
            doc.tree
                .roots
                .iter()
                .any(|key| self.layout.node_rect(*key).is_some())
        }) && playback.tick(self)
        {
            self.mark_dirty();
        }
        self.playback = Some(playback);
    }

    /// The next animation sample, separate from gesture and caret timers.
    pub fn standalone_motion_deadline_ms(&self) -> Option<u64> {
        let p = self.playback.as_ref()?;
        [
            p.sink.timeline.next_deadline_ms(),
            p.transition
                .as_ref()
                .and_then(|t| t.clock.next_deadline_ms(self.now_ms)),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn motion_preference(&self, reduced: bool) -> MotionPreference {
        self.document
            .as_ref()
            .and_then(|doc| doc.schema.motion)
            .unwrap_or(MotionPreference::Full)
            .more_reduced(if reduced {
                MotionPreference::Reduced
            } else {
                MotionPreference::Full
            })
    }

    pub fn standalone_paint_commands(&self) -> Vec<ScenePaintCommand> {
        let entering = self.standalone_steady_commands();
        let Some(transition) = self.playback.as_ref().and_then(|p| p.transition.as_ref()) else {
            return entering;
        };
        let viewport = self
            .playback
            .as_ref()
            .and_then(|p| p.presentation_size)
            .unwrap_or((self.viewport.size.width, self.viewport.size.height));
        let mut frame = Vec::new();
        frame.push(ScenePaintCommand::PushClip(rect(
            0.0, 0.0, viewport.0, viewport.1,
        )));
        for layer in transition
            .resolved
            .layers_at(transition.clock.linear_t(self.now_ms), viewport)
        {
            if layer.alpha <= 0.0 {
                continue;
            }
            let scope = layer.dx != 0.0 || layer.dy != 0.0;
            if scope {
                frame.push(ScenePaintCommand::PushTransform(Affine2::translation(
                    layer.dx, layer.dy,
                )));
            }
            if layer.alpha < 1.0 {
                frame.push(ScenePaintCommand::PushOpacity(layer.alpha));
            }
            match layer.screen {
                TransitionScreen::Outgoing => frame.extend_from_slice(&transition.outgoing),
                TransitionScreen::Entering => frame.extend_from_slice(&entering),
            }
            if layer.alpha < 1.0 {
                frame.push(ScenePaintCommand::PopLayer);
            }
            if scope {
                frame.push(ScenePaintCommand::Pop);
            }
        }
        frame.push(ScenePaintCommand::Pop);
        frame
    }

    fn standalone_steady_commands(&self) -> Vec<ScenePaintCommand> {
        let Some(doc) = &self.document else {
            return Vec::new();
        };
        let theme = WidgetTheme::default();
        let focus = self.focused_widget_id();
        let widgets = self.playback_widget_context(&theme, focus.as_deref());
        let overrides = self.standalone_motion_overrides();
        let motion = self
            .playback
            .as_ref()
            .filter(|p| p.has_style_transitions || !overrides.is_empty())
            .map(|p| SceneMotion {
                overrides: &overrides,
                interaction: &p.interaction,
            });
        crate::render::scene_commands::collect(
            doc,
            &self.layout,
            &self.state,
            Some(&widgets),
            motion.as_ref(),
        )
    }

    fn playback_widget_context<'a>(
        &'a self,
        theme: &'a WidgetTheme,
        focused_id: Option<&'a str>,
    ) -> WidgetRenderCtx<'a> {
        WidgetRenderCtx {
            states: &self.widget_states,
            theme,
            focused_id,
            now_ms: self.now_ms,
            caret_x: self.focused_caret_rect().map(|r| r.min_x()),
        }
    }

    pub(super) fn note_playback_pointer(&mut self, event: &PointerEvent) {
        let Some(playback) = &mut self.playback else {
            return;
        };
        let Some(doc) = &self.document else { return };
        let interaction = &mut playback.interaction;
        let hit = || crate::gesture::hit::hit_test(&self.spatial, doc, event.position).topmost();
        let changed = match event.phase {
            PointerPhase::Hover if event.kind != PointerKind::Touch => {
                let target = hit();
                std::mem::replace(&mut interaction.hovered, target) != target
            }
            PointerPhase::Down => {
                hit().is_some_and(|key| interaction.pressed.insert(event.id.0, key) != Some(key))
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                interaction.pressed.remove(&event.id.0).is_some()
            }
            _ => false,
        };
        if changed && playback.has_style_transitions {
            self.mark_dirty();
        }
    }

    pub(crate) fn prepare_screen_motion(
        &self,
        router: &dyn Router,
        new_depth: usize,
    ) -> Option<ScreenChange> {
        let playback = self.playback.as_ref()?;
        let old_depth = playback.stack_depth?;
        if self.motion_preference(playback.reduced) == MotionPreference::Reduced {
            return None;
        }
        let kind = match new_depth.cmp(&old_depth) {
            std::cmp::Ordering::Greater => NavigationKind::Push,
            std::cmp::Ordering::Less => NavigationKind::Pop,
            std::cmp::Ordering::Equal => NavigationKind::Replace,
        };
        let resolved = ResolvedScreenTransition::resolve(router.last_transition().as_ref(), kind);
        if resolved.effect == TransitionEffect::Cut || resolved.duration_ms == 0 {
            return None;
        }
        let outgoing = self.standalone_steady_commands();
        Some(ScreenChange {
            clock: TimedTransition::new(self.now_ms, resolved.duration_ms),
            resolved,
            outgoing,
            painted_at: None,
        })
    }

    pub(crate) fn mount_screen_motion(
        &mut self,
        stack_depth: usize,
        transition: Option<ScreenChange>,
    ) {
        if let Some(playback) = &mut self.playback {
            playback.stack_depth = Some(stack_depth);
            playback.transition = transition;
        }
    }

    pub(crate) fn note_motion_stack_depth(&mut self, stack_depth: usize) {
        if let Some(playback) = &mut self.playback {
            playback.stack_depth = Some(stack_depth);
        }
    }
}

impl Playback {
    fn tick(&mut self, runtime: &Runtime) -> bool {
        let now = runtime.now_ms;
        let effective = runtime.motion_preference(self.reduced);
        let theme = WidgetTheme::default();
        let focus = runtime.focused_widget_id();
        let widgets = runtime.playback_widget_context(&theme, focus.as_deref());
        let timeline = &self.sink.timeline;
        let document_changed = self.generation != Some(runtime.action_surface_generation);
        if document_changed {
            // Node handles from the prior tree cannot participate in this screen.
            if self.generation.is_some() {
                self.interaction = PlaybackInteraction::default();
            }
            self.has_style_transitions = runtime.document.as_ref().is_some_and(|doc| {
                doc.tree
                    .nodes
                    .values()
                    .any(|node| node.schema.motion_declarations().0.is_some())
            });
        }
        let view = RuntimeMotionView {
            runtime,
            widgets: &widgets,
            interaction: &self.interaction,
        };
        if document_changed {
            // Mount lifecycle actions may already have queued work for this
            // generation. Preserve it when resetting the outgoing tracks.
            let requests = timeline.take_pending();
            timeline.clear();
            for request in requests {
                timeline.request(&request);
            }
            self.triggers.reset_screen();
            self.triggers
                .set_initial_lifecycle_values(runtime, &view, timeline);
            self.generation = Some(runtime.action_surface_generation);
        }

        let transition_changed = self
            .transition
            .as_mut()
            .is_some_and(|transition| transition.painted_at.replace(now) != Some(now));
        if self
            .transition
            .as_ref()
            .is_some_and(|t| !t.clock.is_active(now))
        {
            self.transition = None;
        }
        if self.triggers.begin_lifecycle() {
            self.triggers
                .start_mount_animations(runtime, &view, timeline, effective, now);
        }

        for request in timeline.take_pending() {
            if view.node_rect(&request.target).is_none() {
                continue;
            }
            let base = view.property(&request.target, &request.property);
            let from = request
                .from
                .clone()
                .or_else(|| timeline.current_override(&request.target, &request.property))
                .or_else(|| base.clone());
            if let Some(from) = from {
                timeline.admit(request, from, base);
            }
        }
        let controls = self.sink.take_controls();
        let has_controls = !controls.is_empty();
        for (control, at_ms) in controls {
            self.triggers
                .control(&control, runtime, &view, timeline, effective, at_ms);
        }
        let mut viewport = runtime.viewport;
        if let Some((width, height)) = self.presentation_size {
            viewport.size = crate::geometry::size(width, height);
        }
        let visible = viewport.visible_rect();
        let window = self.window.clone().or_else(|| {
            let root = view.root_rect()?;
            Some(InViewWindow::new(
                (visible.min_y() - root.min_y()).max(0.0),
                0.0,
                (visible.max_y() - visible.min_y().max(root.min_y())).max(0.0),
                Vec::new(),
            ))
        });
        self.triggers.set_in_view_window(window);
        self.triggers
            .observe_transitions(runtime, &view, timeline, effective, now);
        self.triggers
            .observe_toggle_progress(runtime, &view, timeline, effective, now);
        self.triggers
            .observe_in_view(runtime, &view, timeline, effective, now);
        let invalidation = timeline.tick(now);
        document_changed
            || transition_changed
            || has_controls
            || invalidation != InvalidationKind::None
    }
}
