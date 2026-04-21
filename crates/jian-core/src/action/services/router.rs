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
}
