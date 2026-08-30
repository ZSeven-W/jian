//! Who may spend the pending activation id.
//!
//! An activation certifies the input the host is dispatching NOW. The
//! original wiring took it inside `make_action_ctx`, which also builds
//! contexts for due timers, websocket pumps and lifecycle hooks — so on
//! a pointer dispatch the due-timer delivery (which runs FIRST) burned
//! the id and the user's own tap ran uncertified. These tests pin the
//! ownership rules: input paths consume, everything else sees `None`
//! and leaves the id alone.

use jian_core::action::services::effect_sink::{EffectOutcome, EffectRequest, EffectSink};
use jian_core::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

/// Records every effect with the activation its chain carried.
#[derive(Default)]
struct ActivationLog {
    seen: RefCell<Vec<Option<u64>>>,
}

impl EffectSink for ActivationLog {
    fn request(
        &self,
        ctx: &jian_core::action::context::EffectRequestContext,
        _request: &EffectRequest,
    ) -> EffectOutcome {
        self.seen.borrow_mut().push(ctx.activation);
        EffectOutcome::Accepted
    }
}

/// A tappable frame whose handler emits one effect, so the sink can see
/// which activation the chain ran under.
const TAP_DOC: &str = r##"{
    "version": "1.1", "formatVersion": "1.1", "id": "x",
    "app": { "name": "x", "version": "1", "id": "x",
             "capabilities": ["clipboard"] },
    "children": [
        { "type": "frame", "id": "btn", "x": 0, "y": 0, "width": 200, "height": 200,
          "events": { "onTap": [ { "copy": { "text": "'hi'" } } ] } }
    ]
}"##;

fn runtime_with(log: &Rc<ActivationLog>) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(TAP_DOC).expect("load doc");
    rt.build_layout((800.0, 600.0)).expect("layout");
    rt.rebuild_spatial();
    rt.set_effect_sink(log.clone() as Rc<dyn EffectSink>);
    rt
}

fn tap(rt: &mut Runtime, t_down: u64, t_up: u64) {
    use jian_core::geometry::point;
    use jian_core::gesture::pointer::{PointerEvent, PointerPhase};
    let mut down = PointerEvent::simple_at(1, PointerPhase::Down, point(100.0, 100.0), t_down);
    down.kind = jian_core::gesture::pointer::PointerKind::Touch;
    let mut up = PointerEvent::simple_at(1, PointerPhase::Up, point(100.0, 100.0), t_up);
    up.kind = jian_core::gesture::pointer::PointerKind::Touch;
    rt.dispatch_pointer(down);
    rt.dispatch_pointer(up);
}

/// The certified id reaches the chain the user's own input started.
#[test]
fn the_input_chain_runs_under_the_certified_id() {
    let log = Rc::new(ActivationLog::default());
    let mut rt = runtime_with(&log);
    rt.set_activation(Some(42));
    tap(&mut rt, 10, 20);
    assert_eq!(
        log.seen.borrow().as_slice(),
        &[Some(42)],
        "the tap's effect must carry the id the host certified for it"
    );
}

/// A pump tick with no input in flight must not spend the id: whatever
/// the host certified is still there for the NEXT dispatch.
#[test]
fn a_pump_tick_leaves_the_pending_activation_alone() {
    let log = Rc::new(ActivationLog::default());
    let mut rt = runtime_with(&log);
    rt.set_activation(Some(7));
    let _ = rt.pump(50);
    let _ = rt.pump(100);
    assert_eq!(
        rt.take_activation(),
        Some(7),
        "clock ticks are not user input and may not consume the certification"
    );
}

/// A lifecycle hook's context is built outside any input dispatch, so it
/// runs uncertified AND leaves the pending id untouched.
#[test]
fn a_lifecycle_hook_neither_carries_nor_consumes_the_id() {
    let log = Rc::new(ActivationLog::default());
    let mut rt = runtime_with(&log);
    rt.set_activation(Some(9));
    let spawned = rt.spawn_lifecycle(
        "onMount",
        serde_json::json!([{ "copy": { "text": "'mounted'" } }]),
        None,
        serde_json::json!({}),
    );
    assert!(spawned, "the hook list parses and runs");
    assert_eq!(
        log.seen.borrow().as_slice(),
        &[None],
        "a lifecycle chain is not user input and runs uncertified"
    );
    assert_eq!(
        rt.take_activation(),
        Some(9),
        "and the user's pending id is still there for the real dispatch"
    );
}

/// One certification per physical input: the id is spent by the tap that
/// consumed it, and a second tap without a fresh certification runs as
/// `None` rather than inheriting the first one's.
#[test]
fn a_second_tap_does_not_inherit_the_first_certification() {
    let log = Rc::new(ActivationLog::default());
    let mut rt = runtime_with(&log);
    rt.set_activation(Some(1));
    tap(&mut rt, 10, 20);
    tap(&mut rt, 900, 910);
    assert_eq!(
        log.seen.borrow().as_slice(),
        &[Some(1), None],
        "certification is per input, never carried over"
    );
}
