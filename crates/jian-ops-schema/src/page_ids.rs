use crate::page::PenPage;
use crate::routes::RoutesConfig;
use crate::screen_projection::ProjectionWarning;
use std::collections::{BTreeMap, BTreeSet};

/// Make responsive page ids globally unique without consuming any authored id
/// that appears later in author order.
pub fn normalize_page_ids(
    pages: &mut [PenPage],
    routes: &mut RoutesConfig,
    warnings: &mut Vec<ProjectionWarning>,
) {
    let mut taken: BTreeSet<String> = pages.iter().map(|page| page.id.clone()).collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut first_empty_replacement = None;
    for page in pages {
        let raw = page.id.clone();
        let occurrence = seen.entry(raw.clone()).or_default();
        let keep = !raw.is_empty() && *occurrence == 0;
        *occurrence += 1;
        if keep {
            continue;
        }
        let replacement = first_free(&raw, &taken);
        taken.insert(replacement.clone());
        if raw.is_empty() && first_empty_replacement.is_none() {
            first_empty_replacement = Some(replacement.clone());
        }
        warnings.push(ProjectionWarning::PageIdRekeyed {
            from: raw,
            to: replacement.clone(),
        });
        page.id = replacement;
    }
    if let Some(replacement) = first_empty_replacement {
        for route in routes.routes.values_mut() {
            if route.page_id.is_empty() {
                route.page_id = replacement.clone();
            }
        }
    }
}

fn first_free(base: &str, taken: &BTreeSet<String>) -> String {
    if base.is_empty() {
        if !taken.contains("~root") {
            return "~root".to_owned();
        }
        return (2_u64..)
            .map(|number| format!("~root~{number}"))
            .find(|candidate| !taken.contains(candidate))
            .expect("u64 page id probe space exhausted");
    }
    (2_u64..)
        .map(|number| format!("{base}~{number}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("u64 page id probe space exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::PenPage;
    use crate::routes::{RouteSpec, RoutesConfig};
    use std::collections::BTreeMap;

    fn pages_with_ids(ids: &[&str]) -> Vec<PenPage> {
        ids.iter()
            .map(|id| PenPage {
                id: (*id).to_owned(),
                name: (*id).to_owned(),
                children: Vec::new(),
                background_color: None,
                state: None,
                lifecycle: None,
            })
            .collect()
    }

    fn routes_to(page_id: &str) -> RoutesConfig {
        RoutesConfig {
            entry: "/".to_owned(),
            routes: BTreeMap::from([(
                "/".to_owned(),
                RouteSpec {
                    page_id: page_id.to_owned(),
                    preload: None,
                    guards: None,
                    params: None,
                },
            )]),
            transitions: None,
        }
    }

    #[test]
    fn probe_never_steals_authored_names() {
        let mut pages = pages_with_ids(&["a", "a", "a~2"]);
        let mut routes = routes_to("a");
        let mut warnings = Vec::new();
        normalize_page_ids(&mut pages, &mut routes, &mut warnings);
        assert_eq!(
            pages
                .iter()
                .map(|page| page.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "a~3", "a~2"]
        );
        assert_eq!(routes.routes["/"].page_id, "a");
    }

    #[test]
    fn empty_ids_probe_from_root_and_rewrite_empty_references() {
        let mut pages = pages_with_ids(&["", "", "~root"]);
        let mut routes = routes_to("");
        let mut warnings = Vec::new();
        normalize_page_ids(&mut pages, &mut routes, &mut warnings);
        assert_eq!(
            pages
                .iter()
                .map(|page| page.id.as_str())
                .collect::<Vec<_>>(),
            ["~root~2", "~root~3", "~root"]
        );
        assert_eq!(routes.routes["/"].page_id, "~root~2");
        assert!(warnings.len() >= 2);
    }
}
