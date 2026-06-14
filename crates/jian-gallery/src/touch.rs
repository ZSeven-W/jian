use jian_core::document::{NodeKey, NodeTree, RuntimeDocument};
use jian_core::geometry::point;
use jian_core::gesture::arena::Arena;
use jian_core::gesture::pointer::{
    Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase,
};
use jian_core::gesture::recognizers::{LongPressRecognizer, PanRecognizer, TapRecognizer};
use jian_core::gesture::semantic::SemanticEvent;
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::node::PenNode;
use jian_widgets::Point2D;
use std::time::Duration;
use std::time::Instant;

const LONG_PRESS_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Debug, Clone, Copy)]
pub struct TouchInput {
    pub id: u64,
    pub phase: TouchPhase,
    pub position: Point2D,
    pub timestamp: Instant,
}

impl TouchInput {
    pub fn new(id: u64, phase: TouchPhase, position: Point2D, timestamp: Instant) -> Self {
        Self {
            id,
            phase,
            position,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GalleryGesture {
    Press(Point2D),
    Tap(Point2D),
    PanStart { position: Point2D, delta: Point2D },
    PanDelta(Point2D),
    PanEnd,
    LongPress(Point2D),
    Cancel,
}

pub struct TouchArena {
    arena: Option<Arena>,
    doc: RuntimeDocument,
    node: NodeKey,
    start_position: Option<Point2D>,
    next_tick_at: Option<Instant>,
}

impl TouchArena {
    pub fn new() -> Self {
        let (doc, node) = touch_document();
        Self {
            arena: None,
            doc,
            node,
            start_position: None,
            next_tick_at: None,
        }
    }

    pub fn handle(&mut self, input: TouchInput) -> Vec<GalleryGesture> {
        let mut out = Vec::new();
        if input.phase == TouchPhase::Started {
            self.start_position = Some(input.position);
            self.next_tick_at = Some(input.timestamp + LONG_PRESS_DURATION);
            self.arena = Some(Arena::new(vec![
                Box::new(TapRecognizer::new(1, self.node)),
                Box::new(PanRecognizer::new(2, self.node)),
                Box::new(LongPressRecognizer::new(3, self.node)),
            ]));
            out.push(GalleryGesture::Press(input.position));
        }

        let event = pointer_event(input);
        if let Some(arena) = self.arena.as_mut() {
            arena.dispatch(&event, &self.doc);
            let gestures = map_semantic(arena.drain_emitted(), self.start_position);
            if gestures.iter().any(|gesture| {
                matches!(
                    gesture,
                    GalleryGesture::PanStart { .. } | GalleryGesture::LongPress(_)
                )
            }) {
                self.next_tick_at = None;
            }
            out.extend(gestures);
        }

        if matches!(input.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            if input.phase == TouchPhase::Cancelled {
                out.push(GalleryGesture::Cancel);
            }
            self.arena = None;
            self.start_position = None;
            self.next_tick_at = None;
        }

        out
    }

    pub fn tick(&mut self, timestamp: Instant) -> Vec<GalleryGesture> {
        let Some(arena) = self.arena.as_mut() else {
            return Vec::new();
        };
        arena.tick(timestamp);
        let gestures = map_semantic(arena.drain_emitted(), self.start_position);
        if self
            .next_tick_at
            .is_some_and(|deadline| timestamp >= deadline)
            || gestures
                .iter()
                .any(|gesture| matches!(gesture, GalleryGesture::LongPress(_)))
        {
            self.next_tick_at = None;
        }
        gestures
    }

    pub fn next_tick_at(&self) -> Option<Instant> {
        self.next_tick_at
    }
}

impl Default for TouchArena {
    fn default() -> Self {
        Self::new()
    }
}

fn pointer_event(input: TouchInput) -> PointerEvent {
    PointerEvent {
        id: PointerId(input.id as u32),
        kind: PointerKind::Touch,
        phase: match input.phase {
            TouchPhase::Started => PointerPhase::Down,
            TouchPhase::Moved => PointerPhase::Move,
            TouchPhase::Ended => PointerPhase::Up,
            TouchPhase::Cancelled => PointerPhase::Cancel,
        },
        position: point(input.position.x, input.position.y),
        pressure: 1.0,
        buttons: MouseButtons::LEFT,
        modifiers: Modifiers::empty(),
        tilt: None,
        timestamp: input.timestamp,
    }
}

fn map_semantic(
    events: Vec<SemanticEvent>,
    start_position: Option<Point2D>,
) -> Vec<GalleryGesture> {
    events
        .into_iter()
        .filter_map(|event| match event {
            SemanticEvent::Tap { position, .. } => {
                Some(GalleryGesture::Tap(Point2D::new(position.x, position.y)))
            }
            SemanticEvent::LongPress { position, .. } => Some(GalleryGesture::LongPress(
                Point2D::new(position.x, position.y),
            )),
            SemanticEvent::PanStart { position, .. } => {
                let position = Point2D::new(position.x, position.y);
                let start = start_position.unwrap_or(position);
                Some(GalleryGesture::PanStart {
                    position,
                    delta: Point2D::new(position.x - start.x, position.y - start.y),
                })
            }
            SemanticEvent::PanUpdate { delta, .. } => {
                Some(GalleryGesture::PanDelta(Point2D::new(delta.x, delta.y)))
            }
            SemanticEvent::PanEnd { .. } => Some(GalleryGesture::PanEnd),
            _ => None,
        })
        .collect()
}

fn touch_document() -> (RuntimeDocument, NodeKey) {
    let node = dummy_node();
    let mut tree = NodeTree::new();
    let key = tree.insert_subtree(node.clone(), None);
    let schema = PenDocument {
        version: "0.0.1".to_owned(),
        name: Some("jian-gallery-touch-arena".to_owned()),
        themes: None,
        variables: None,
        pages: None,
        children: vec![node],
        format_version: None,
        id: None,
        app: None,
        routes: None,
        state: None,
        lifecycle: None,
        logic_modules: None,
        design_md: None,
    };
    (
        RuntimeDocument {
            schema,
            tree,
            active_page: None,
        },
        key,
    )
}

fn dummy_node() -> PenNode {
    serde_json::from_value(serde_json::json!({
        "type": "rectangle",
        "id": "gallery-touch-root"
    }))
    .expect("static gallery touch node is valid")
}
