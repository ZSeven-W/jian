use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct RouteState {
    pub path: String,
    pub params: BTreeMap<String, String>,
    pub query: BTreeMap<String, String>,
    pub stack: Vec<String>,
}

pub trait Router {
    fn current(&self) -> RouteState;
    fn push(&self, path: &str);
    fn replace(&self, path: &str);
    fn pop(&self);
    fn reset(&self, path: &str);
    fn restore(&self, state: RouteState, valid_paths: &[String]) {
        let path = if valid_paths.iter().any(|path| path == &state.path) {
            state.path
        } else {
            valid_paths.first().cloned().unwrap_or_else(|| "/".into())
        };
        self.reset(&path);
    }
}
