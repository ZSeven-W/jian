//! Native menu bar — Plan 8 §B.5.
//!
//! Phase 1 ships:
//! - **`MenuSpec`** — pure-data declaration of File / Edit / View
//!   entries authors can populate from a `.op` file's `app.menus`
//!   section (or a host-supplied default). Always built; doesn't
//!   depend on the `menus` cargo feature.
//! - **`muda` integration** under `cfg(feature = "menus")` — the
//!   spec is materialised into a `muda::Menu` attached to the
//!   active window. Without the feature the spec is still
//!   serialisable for IPC tools but no native menu shows.
//!
//! Real cross-platform polish (accelerators, system menus on
//! macOS, Windows menu styling) lands once the host has actual
//! shipping apps to validate against. Phase 1 is the contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuSpec {
    pub items: Vec<MenuItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuItem {
    /// Top-level submenu (File / Edit / View / …).
    Submenu { label: String, items: Vec<MenuItem> },
    /// Clickable action. `id` is what the host receives back when
    /// the menu fires.
    Action {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accelerator: Option<String>,
    },
    /// Visual separator.
    Separator,
}

impl MenuSpec {
    /// Standard File / Edit / View / Help skeleton. Hosts call
    /// `default_app_spec()` to get something reasonable when an
    /// authored `app.menus` is absent.
    pub fn default_app_spec(app_name: &str) -> Self {
        Self {
            items: vec![
                MenuItem::Submenu {
                    label: app_name.to_owned(),
                    items: vec![
                        MenuItem::Action {
                            id: "app.about".into(),
                            label: format!("About {}", app_name),
                            accelerator: None,
                        },
                        MenuItem::Separator,
                        MenuItem::Action {
                            id: "app.quit".into(),
                            label: format!("Quit {}", app_name),
                            accelerator: Some("CmdOrCtrl+Q".into()),
                        },
                    ],
                },
                MenuItem::Submenu {
                    label: "File".into(),
                    items: vec![
                        MenuItem::Action {
                            id: "file.open".into(),
                            label: "Open…".into(),
                            accelerator: Some("CmdOrCtrl+O".into()),
                        },
                        MenuItem::Action {
                            id: "file.save".into(),
                            label: "Save".into(),
                            accelerator: Some("CmdOrCtrl+S".into()),
                        },
                    ],
                },
                MenuItem::Submenu {
                    label: "Edit".into(),
                    items: vec![
                        MenuItem::Action {
                            id: "edit.undo".into(),
                            label: "Undo".into(),
                            accelerator: Some("CmdOrCtrl+Z".into()),
                        },
                        MenuItem::Action {
                            id: "edit.redo".into(),
                            label: "Redo".into(),
                            accelerator: Some("CmdOrCtrl+Shift+Z".into()),
                        },
                    ],
                },
            ],
        }
    }
}

/// Materialise a `MenuSpec` into a `muda::Menu`. Available only when
/// the `menus` cargo feature is on. Host then `init_for_*` the menu
/// against the active window per the muda docs.
#[cfg(feature = "menus")]
pub fn build_muda_menu(spec: &MenuSpec) -> muda::Menu {
    use muda::{accelerator::Accelerator, Menu, MenuItem as MudaItem, PredefinedMenuItem, Submenu};
    use std::str::FromStr;

    fn append(parent: &Submenu, items: &[MenuItem]) {
        for item in items {
            match item {
                MenuItem::Separator => {
                    let _ = parent.append(&PredefinedMenuItem::separator());
                }
                MenuItem::Action {
                    id,
                    label,
                    accelerator,
                } => {
                    let accel: Option<Accelerator> = accelerator
                        .as_deref()
                        .and_then(|s| Accelerator::from_str(s).ok());
                    let muda_item = MudaItem::with_id(id, label, true, accel);
                    let _ = parent.append(&muda_item);
                }
                MenuItem::Submenu { label, items } => {
                    let sub = Submenu::new(label, true);
                    let _ = parent.append(&sub);
                    append(&sub, items);
                }
            }
        }
    }

    let menu = Menu::new();
    for item in &spec.items {
        match item {
            MenuItem::Submenu { label, items } => {
                let sub = Submenu::new(label, true);
                let _ = menu.append(&sub);
                append(&sub, items);
            }
            MenuItem::Separator => {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
            MenuItem::Action {
                id,
                label,
                accelerator,
            } => {
                let accel: Option<Accelerator> = accelerator
                    .as_deref()
                    .and_then(|s| Accelerator::from_str(s).ok());
                let muda_item = MudaItem::with_id(id, label, true, accel);
                let _ = menu.append(&muda_item);
            }
        }
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_has_app_file_edit() {
        let spec = MenuSpec::default_app_spec("Jian");
        let labels: Vec<&str> = spec
            .items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Submenu { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["Jian", "File", "Edit"]);
    }

    #[test]
    fn menuspec_round_trips_through_serde() {
        let spec = MenuSpec::default_app_spec("Jian");
        let s = serde_json::to_string(&spec).unwrap();
        let back: MenuSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn action_with_accelerator_serialises() {
        let item = MenuItem::Action {
            id: "file.save".into(),
            label: "Save".into(),
            accelerator: Some("CmdOrCtrl+S".into()),
        };
        let s = serde_json::to_string(&item).unwrap();
        assert!(s.contains("\"accelerator\":\"CmdOrCtrl+S\""));
    }
}
