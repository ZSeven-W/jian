use crate::action::services::RouteState;
use crate::state::StateGraph;
use jian_ops_schema::document::PenDocument;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct PreparedDocument {
    pub(super) mounted: PenDocument,
    pub(super) source: Option<PenDocument>,
    pub(super) variants: jian_ops_schema::screen_projection::ScreenVariantTable,
    pub(super) path: Option<String>,
    pub(super) selected_page_id: Option<String>,
    pub(super) warnings: Vec<String>,
}

pub(super) fn prepare_document(
    mut schema: PenDocument,
    viewport: (f32, f32),
    preferred_path: Option<&str>,
) -> PreparedDocument {
    if !schema.is_responsive() {
        return PreparedDocument {
            mounted: schema,
            source: None,
            variants: Default::default(),
            path: None,
            selected_page_id: None,
            warnings: Vec::new(),
        };
    }
    let (projected, projection_warnings) =
        jian_ops_schema::screen_projection::project_screens(&schema);
    let mut warnings: Vec<String> = projection_warnings
        .into_iter()
        .map(|warning| warning.to_string())
        .collect();
    if let Some((source, variants)) = projected {
        let path = preferred_path
            .filter(|path| variants.0.contains_key(*path))
            .map(str::to_owned)
            .or_else(|| source.routes.as_ref().map(|routes| routes.entry.clone()))
            .unwrap_or_else(|| "/".to_owned());
        let selected_page_id = select_variant_page(&variants, &path, viewport.0);
        let mut mounted = source.clone();
        if let Some(selected) = selected_page_id.as_deref() {
            mounted.pages = source
                .pages
                .as_ref()
                .and_then(|pages| pages.iter().find(|page| page.id == selected))
                .cloned()
                .map(|page| vec![page]);
        }
        return PreparedDocument {
            mounted,
            source: Some(source),
            variants,
            path: Some(path),
            selected_page_id,
            warnings,
        };
    }
    if let Some(pages) = schema.pages.as_mut() {
        let had_routes = schema.routes.is_some();
        let mut routes = schema
            .routes
            .take()
            .unwrap_or(jian_ops_schema::routes::RoutesConfig {
                entry: String::new(),
                routes: Default::default(),
                transitions: None,
            });
        let mut id_warnings = Vec::new();
        jian_ops_schema::page_ids::normalize_page_ids(pages, &mut routes, &mut id_warnings);
        warnings.extend(id_warnings.into_iter().map(|warning| warning.to_string()));
        if had_routes {
            schema.routes = Some(routes);
        }
    }
    let selected_page_id = schema
        .pages
        .as_ref()
        .and_then(|pages| pages.first())
        .map(|page| page.id.clone());
    PreparedDocument {
        mounted: schema,
        source: None,
        variants: Default::default(),
        path: None,
        selected_page_id,
        warnings,
    }
}

fn select_variant_page(
    variants: &jian_ops_schema::screen_projection::ScreenVariantTable,
    path: &str,
    width: f32,
) -> Option<String> {
    let set = variants.0.get(path)?;
    Some(
        set.ranged
            .iter()
            .find(|entry| {
                entry.range.min_width.unwrap_or(0.0) as f32 <= width
                    && width <= entry.range.max_width.unwrap_or(f64::INFINITY) as f32
            })
            .map_or_else(
                || set.default_page_id.clone(),
                |entry| entry.page_id.clone(),
            ),
    )
}

pub(super) fn copy_layout_scopes(source: &StateGraph, target: &StateGraph, storage_allowed: bool) {
    target.replace_app(&source.app_snapshot());
    target.replace_vars(&source.vars_snapshot());
    target.replace_route(&source.route_snapshot());
    target.replace_viewport(&source.viewport_snapshot());
    if storage_allowed {
        target.replace_storage(&source.storage_snapshot());
        if let serde_json::Value::Object(values) = source.storage_cache.snapshot() {
            for (key, value) in values {
                target.storage_cache.set_local(&key, value);
            }
        }
    }
    for page_key in source.page_keys() {
        target.replace_page(&page_key, &source.page_snapshot(&page_key));
    }
    for (page_key, node_id) in source.self_keys() {
        target.replace_self(
            &page_key,
            &node_id,
            &source.self_snapshot(&page_key, &node_id),
        );
    }
}

pub(super) fn route_values(route: &RouteState) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([
        ("path".to_owned(), serde_json::json!(route.path)),
        ("params".to_owned(), serde_json::json!(route.params)),
        ("query".to_owned(), serde_json::json!(route.query)),
        ("stack".to_owned(), serde_json::json!(route.stack)),
    ])
}

pub(super) fn normalized_route_values(
    route: &RouteState,
    valid_paths: &[String],
) -> BTreeMap<String, serde_json::Value> {
    let valid: BTreeSet<&str> = valid_paths.iter().map(String::as_str).collect();
    let survives = valid.contains(route.path.as_str());
    let (path, params, query, mut stack) = if survives {
        (
            route.path.clone(),
            route.params.clone(),
            route.query.clone(),
            route
                .stack
                .iter()
                .filter(|path| valid.contains(path.as_str()))
                .cloned()
                .collect(),
        )
    } else {
        (
            valid_paths.first().cloned().unwrap_or_else(|| "/".into()),
            BTreeMap::new(),
            BTreeMap::new(),
            Vec::new(),
        )
    };
    if stack.last() != Some(&path) {
        stack.push(path.clone());
    }
    BTreeMap::from([
        ("path".to_owned(), serde_json::json!(path)),
        ("params".to_owned(), serde_json::json!(params)),
        ("query".to_owned(), serde_json::json!(query)),
        ("stack".to_owned(), serde_json::json!(stack)),
    ])
}
