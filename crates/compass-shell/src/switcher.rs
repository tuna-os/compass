//! The shell side of the window switcher: turning listed windows into rows.
//!
//! The display model — [`compass_core::window_switcher::WindowEntry`] and
//! everything that describes it — lives in the shared crate, which must never
//! name this crate's D-Bus types. This module is the boundary: it converts a
//! shell [`crate::model::Window`] (typed `a{sv}`) into that model, and mints
//! the `window:{n}` root-item ids the shared parsers accept.

use crate::model::Window;
use compass_core::root_items::{RootItem, RootItemMeta};
use compass_core::window_switcher::{
    WindowEntry, window_search_fields, window_subtitle, window_title,
};

/// Convert a shell `Window` into a switcher `WindowEntry`.
///
/// The shell gives `title`/`wm_class`/`workspace`; the app registry (when
/// wired) enriches with `app_name`/`app_icon`. Keeping the conversion here
/// keeps the `a{sv}` decoding in [`crate::model`] and the display logic in
/// the shared module distinct.
#[must_use]
pub fn entry_from_shell(window: &Window) -> WindowEntry {
    WindowEntry {
        title: window.title.clone(),
        wm_class: window.wm_class.clone(),
        app_name: None,
        app_icon: None,
        workspace_name: String::new(),
        workspace_id: window.workspace.map(|id| id.to_string()),
    }
}

/// Convert a shell `Window` into a searchable `RootItem` for the
/// “switch-windows” provider.
///
/// This is the thin wiring that makes `ListWindows` appear in the root list
/// ranking — title weight 1.0, subtitle weight 0.5, `wm_class` at 0.3, so
/// typing either the window title or its `WM_CLASS` finds it. The minted id
/// is `window:{n}`, the only shape
/// [`compass_core::window_switcher::window_launch_target`] accepts.
#[must_use]
pub fn window_to_root_item(window: &Window) -> RootItem {
    let entry = entry_from_shell(window);
    let title = window_title(&entry).to_owned();
    let subtitle = window_subtitle(&entry).to_owned();
    let keywords = window_search_fields(&entry)
        .into_iter()
        .skip(1)
        .map(|(t, _)| t.to_owned())
        .collect::<Vec<_>>();
    RootItem {
        id: format!("window:{}", window.id),
        title,
        unlocalized_title: None,
        subtitle,
        keywords,
        meta: RootItemMeta {
            provider_id: "switch-windows".to_owned(),
            enabled: true,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::window_key;
    use compass_core::window_switcher::{
        Capabilities, window_accessory, window_action_panel, window_launch_target,
    };
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};

    fn window_from(title: &str, wm_class: &str, workspace: Option<i32>) -> Window {
        let mut dict = HashMap::from([
            (window_key::ID.to_owned(), OwnedValue::from(42u32)),
            (
                window_key::TITLE.to_owned(),
                OwnedValue::try_from(Value::from(title.to_owned())).unwrap(),
            ),
            (
                window_key::WM_CLASS.to_owned(),
                OwnedValue::try_from(Value::from(wm_class.to_owned())).unwrap(),
            ),
        ]);
        if let Some(ws) = workspace {
            dict.insert(window_key::WORKSPACE.to_owned(), OwnedValue::from(ws));
        }
        Window::from_dict(&dict).unwrap()
    }

    #[test]
    fn entry_from_shell_preserves_title_and_workspace() {
        let window = window_from("Inbox", "org.gnome.Geary", Some(2));
        let entry = entry_from_shell(&window);
        assert_eq!(entry.title, "Inbox");
        assert_eq!(entry.wm_class, "org.gnome.Geary");
        assert_eq!(entry.workspace_id.as_deref(), Some("2"));
        assert_eq!(window_subtitle(&entry), "org.gnome.Geary");
        assert_eq!(window_accessory(&entry).as_deref(), Some("WS 2"));
    }

    #[test]
    fn entry_from_shell_with_no_workspace_shows_no_accessory() {
        let window = window_from("Terminal", "org.gnome.Terminal", None);
        let entry = entry_from_shell(&window);
        assert_eq!(entry.workspace_id, None);
        assert_eq!(window_accessory(&entry), None);
    }

    #[test]
    fn window_to_root_item_is_searchable_by_title_and_wm_class() {
        let window = window_from("Inbox", "org.gnome.Geary", Some(1));
        let item = window_to_root_item(&window);
        assert_eq!(item.id, "window:42");
        assert_eq!(item.title, "Inbox");
        assert_eq!(item.subtitle, "org.gnome.Geary");
        assert_eq!(item.meta.provider_id, "switch-windows");
        // keywords keep wm_class at low weight, searchable via root search
        assert!(item.keywords.contains(&"org.gnome.Geary".to_owned()));
        assert_eq!(window_launch_target(&item), Some(42));
    }

    #[test]
    fn window_launch_target_rejects_non_window_items() {
        let window = window_from("Inbox", "org.gnome.Geary", None);
        let mut root = window_to_root_item(&window);
        // Wrong provider
        root.meta.provider_id = "apps".to_owned();
        assert_eq!(window_launch_target(&root), None);
        // Wrong id shape
        root.meta.provider_id = "switch-windows".to_owned();
        root.id = "window:foo".to_owned();
        assert_eq!(window_launch_target(&root), None);
        root.id = "not-a-window".to_owned();
        assert_eq!(window_launch_target(&root), None);
        // Correct again
        root.id = "window:42".to_owned();
        assert!(window_launch_target(&root).is_some());
    }

    #[test]
    fn window_root_item_action_panel_exposes_focus_and_close() {
        let window = window_from("Terminal", "org.gnome.Terminal", None);
        let entry = entry_from_shell(&window);
        let panel = window_action_panel(&entry, Capabilities::default());
        assert_eq!(panel[0], ["focus", "close"]);
        assert_eq!(panel.len(), 1); // no app_name → no quit-app section
        // With app_name and full caps, panel gains pin/bring and quit
        let mut entry_with_app = entry;
        entry_with_app.app_name = Some("Terminal".to_owned());
        let caps = Capabilities {
            set_sticky: true,
            move_to_workspace: true,
            ..Capabilities::default()
        };
        let panel2 = window_action_panel(&entry_with_app, caps);
        assert_eq!(panel2[0], ["focus", "pin", "bring-to-workspace", "close"]);
        assert_eq!(panel2[1], ["quit-app", "force-quit-app"]);
    }
}
