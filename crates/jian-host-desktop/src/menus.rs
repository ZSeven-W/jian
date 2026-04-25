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

/// Result of materialising a `MenuSpec`. The `menu` is the muda
/// handle the host attaches to its window; `warnings` lists any
/// authored accelerator strings that failed to parse — these are
/// surfaced rather than silently dropped so a typo doesn't leave
/// the user wondering why their shortcut doesn't work.
#[cfg(feature = "menus")]
pub struct BuiltMenu {
    pub menu: muda::Menu,
    pub warnings: Vec<String>,
}

/// Materialise a `MenuSpec` into a `muda::Menu`. Available only when
/// the `menus` cargo feature is on. Host then `init_for_*` the menu
/// against the active window per the muda docs.
#[cfg(feature = "menus")]
pub fn build_muda_menu(spec: &MenuSpec) -> BuiltMenu {
    use muda::{accelerator::Accelerator, Menu, MenuItem as MudaItem, PredefinedMenuItem, Submenu};
    use std::str::FromStr;

    fn parse_accel(
        raw: Option<&str>,
        ctx: &str,
        warnings: &mut Vec<String>,
    ) -> Option<Accelerator> {
        let s = raw?;
        match Accelerator::from_str(s) {
            Ok(a) => Some(a),
            Err(e) => {
                warnings.push(format!("{}: bad accelerator {:?}: {}", ctx, s, e));
                None
            }
        }
    }

    fn append(parent: &Submenu, items: &[MenuItem], warnings: &mut Vec<String>) {
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
                    let accel = parse_accel(accelerator.as_deref(), id, warnings);
                    let muda_item = MudaItem::with_id(id, label, true, accel);
                    let _ = parent.append(&muda_item);
                }
                MenuItem::Submenu { label, items } => {
                    let sub = Submenu::new(label, true);
                    let _ = parent.append(&sub);
                    append(&sub, items, warnings);
                }
            }
        }
    }

    let menu = Menu::new();
    let mut warnings = Vec::new();
    for item in &spec.items {
        match item {
            MenuItem::Submenu { label, items } => {
                let sub = Submenu::new(label, true);
                let _ = menu.append(&sub);
                append(&sub, items, &mut warnings);
            }
            MenuItem::Separator => {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
            MenuItem::Action {
                id,
                label,
                accelerator,
            } => {
                let accel = parse_accel(accelerator.as_deref(), id, &mut warnings);
                let muda_item = MudaItem::with_id(id, label, true, accel);
                let _ = menu.append(&muda_item);
            }
        }
    }
    BuiltMenu { menu, warnings }
}

/// Attach a built `muda::Menu` to the host's active window. Each
/// platform has its own muda entry point — macOS hangs the menu off
/// the running NSApp (no window needed), Windows wants the HWND,
/// Linux/GTK needs the underlying GTK window which winit doesn't
/// expose at this layer.
///
/// Returns `Ok(())` on platforms where attachment succeeded (or was
/// a no-op by design). Linux currently logs to stderr and returns
/// `Ok(())` rather than failing — the menu spec is still
/// serialisable for future GTK or Wayland integration.
#[cfg(feature = "menus")]
pub fn init_menu_for_window(
    menu: &muda::Menu,
    #[allow(unused_variables)] window: &winit::window::Window,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // macOS menus are app-global, not per-window — initialising
        // once on the running NSApp covers every future window. Calling
        // it again on a second window is harmless (muda just overwrites).
        menu.init_for_nsapp();
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = window
            .window_handle()
            .map_err(|e| format!("window_handle: {e}"))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => menu
                .init_for_hwnd(h.hwnd.get() as isize)
                .map_err(|e| format!("init_for_hwnd: {e}")),
            other => Err(format!("unexpected window handle on Windows: {:?}", other)),
        }
    }
    #[cfg(target_os = "linux")]
    {
        // muda's Linux backend wants a `gtk::Window`, but winit on
        // Linux can run on either X11 or Wayland and doesn't surface
        // a GTK widget for us to hand over. Hosts that need a Linux
        // menu bar must build their own gtk::Application + gtk::Window
        // and call `menu.init_for_gtk_window` themselves; the run
        // loop here just leaves the menu unattached.
        eprintln!(
            "jian-host-desktop: native menu unattached on Linux \
             (muda needs a gtk::Window the winit backend doesn't expose)"
        );
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = menu;
        Err("init_menu_for_window: unsupported platform".into())
    }
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
