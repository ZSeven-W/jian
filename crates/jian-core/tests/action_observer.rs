//! R9 ActionObserver wraps execution, including nested ActionLists.

use jian_core::action::services::ActionObserver;
use jian_core::action::{execute_list_async, ActionContext, ActionResult};
use jian_core::Runtime;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct RecordingObserver {
    next: Cell<u64>,
    events: RefCell<Vec<(String, &'static str, u64)>>,
}

impl ActionObserver for RecordingObserver {
    fn action_started(&self, action: &'static str, _context: &ActionContext) -> u64 {
        let token = self.next.get() + 1;
        self.next.set(token);
        self.events
            .borrow_mut()
            .push(("start".to_owned(), action, token));
        token
    }

    fn action_finished(
        &self,
        token: u64,
        action: &'static str,
        _context: &ActionContext,
        _result: &ActionResult,
    ) {
        self.events
            .borrow_mut()
            .push(("result".to_owned(), action, token));
    }
}

#[test]
fn nested_lists_are_observed_around_execute_not_parse() {
    let observer = Rc::new(RecordingObserver::default());
    let service: Rc<dyn ActionObserver> = observer.clone();
    let mut runtime = Runtime::new();
    runtime.set_action_observer(service);
    let actions = json!([
        {"if":{"expr":"true","then":[{"set":{"$app.a":"1"}}]}},
        {"parallel":[
            [{"set":{"$app.b":"2"}}],
            [{"set":{"$app.c":"3"}}]
        ]}
    ]);
    let context = runtime.make_action_ctx();
    let registry = runtime.actions.borrow();
    let outcome = futures::executor::block_on(execute_list_async(&registry, &actions, &context));
    assert!(outcome.result.is_ok());
    assert_eq!(
        observer.events.borrow().as_slice(),
        &[
            ("start".to_owned(), "if", 1),
            ("start".to_owned(), "set", 2),
            ("result".to_owned(), "set", 2),
            ("result".to_owned(), "if", 1),
            ("start".to_owned(), "parallel", 3),
            ("start".to_owned(), "set", 4),
            ("result".to_owned(), "set", 4),
            ("start".to_owned(), "set", 5),
            ("result".to_owned(), "set", 5),
            ("result".to_owned(), "parallel", 3),
        ]
    );
}
