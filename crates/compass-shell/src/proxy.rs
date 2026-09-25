//! `zbus` proxies for the two contract interfaces.
//!
//! These mirror the checked-in introspection XML exactly, and that is checked
//! rather than asserted in prose: `tests/contract_introspection.rs`
//! introspects a live object server built from these same interfaces and
//! compares every method, signal, argument and property against
//! `dbus/*.xml`. They intentionally
//! carry no default service/path: [`ShellClient`](crate::ShellClient) builds
//! them from [`ShellConfig`](crate::ShellConfig) so that tests can point the
//! same code at a private bus.
//!
//! The `#[zbus::proxy]` macro generates undocumented helper items, so
//! `missing_docs` is relaxed for this module.

#![allow(missing_docs)]

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

/// Proxy for `org.gnome.Shell.Extensions.Vicinae.Windows`.
#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions.Vicinae.Windows",
    default_service = "org.gnome.Shell",
    default_path = "/org/gnome/Shell/Extensions/Vicinae/Windows"
)]
pub trait Windows {
    /// Contract version implemented by the extension.
    #[zbus(property(emits_changed_signal = "false"))]
    fn version(&self) -> zbus::Result<u32>;

    /// All windows the extension exposes, most-recently-used first.
    fn list_windows(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// Focus and raise a window.
    fn activate_window(&self, id: u32) -> zbus::Result<()>;

    /// Ask a window to close.
    fn close_window(&self, id: u32) -> zbus::Result<()>;

    /// Something in the window set changed; re-read `ListWindows`.
    #[zbus(signal)]
    fn windows_changed(&self) -> zbus::Result<()>;

    /// The workspaces, in order (contract 4).
    fn list_workspaces(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// Switch to the workspace at `index` (contract 4).
    fn activate_workspace(&self, index: i32) -> zbus::Result<()>;
}

/// Proxy for `org.gnome.Shell.Extensions.Vicinae.Clipboard`.
#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions.Vicinae.Clipboard",
    default_service = "org.gnome.Shell",
    default_path = "/org/gnome/Shell/Extensions/Vicinae/Clipboard"
)]
pub trait Clipboard {
    /// Contract version implemented by the extension.
    #[zbus(property(emits_changed_signal = "false"))]
    fn version(&self) -> zbus::Result<u32>;

    /// Read the current selection as `(content, mime_type)`.
    fn get_clipboard(&self) -> zbus::Result<(Vec<u8>, String)>;

    /// Replace the current selection.
    fn set_clipboard(&self, content: &[u8], mime_type: &str) -> zbus::Result<()>;

    /// Paste the selection into the window focus moves to next, with
    /// Ctrl+Shift+V for the listed `WM_CLASS`es.
    fn paste(&self, shift_wm_classes: &[&str]) -> zbus::Result<()>;

    /// The primary selection as text; `""` when there is none.
    fn get_primary_selection(&self) -> zbus::Result<String>;

    /// The selection changed.
    #[zbus(signal)]
    fn clipboard_changed(
        &self,
        content: Vec<u8>,
        mime_type: String,
        source_app: String,
    ) -> zbus::Result<()>;
}
