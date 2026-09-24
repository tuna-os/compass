//! Window switching: the Shell extension's windows, as switcher rows.
//!
//! On GNOME the extension is the only way to list or raise another
//! application's window (Mutter implements no foreign-toplevel protocol, and
//! `org.gnome.Shell.Introspect` is allowlisted to the portal backends). So
//! the engine holds one `compass_shell` client and answers the window
//! requests through it; without the extension they are refused by name.

use compass_core::{AppIndex, app_service::AppService};
use compass_ipc::{ErrorKind, ProtocolError, WindowInfo};
use compass_shell::{ShellError, Window};

/// A window as the extension reported it: the fields rows are built from.
///
/// Its own type rather than `compass_shell::Window`, which is
/// `#[non_exhaustive]` and so cannot be built by the tests that pin [`rows`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellWindow {
    /// Handle for activate and close.
    pub id: u32,
    /// Title.
    pub title: String,
    /// `WM_CLASS`.
    pub wm_class: String,
    /// Owning process.
    pub pid: Option<u32>,
    /// Workspace index.
    pub workspace: Option<i32>,
    /// Has focus.
    pub focused: bool,
    /// Can be closed.
    pub can_close: bool,
}

impl From<Window> for ShellWindow {
    fn from(window: Window) -> Self {
        Self {
            id: window.id.0,
            title: window.title,
            wm_class: window.wm_class,
            pid: window.pid,
            workspace: window.workspace,
            focused: window.focused,
            can_close: window.can_close,
        }
    }
}

/// Switcher rows for `windows`, recognising each window's application in
/// `index`.
///
/// Order is the extension's, except that the focused window goes last: the
/// focused window is the one the user is already looking at, so the row worth
/// selecting first is the one before it.
#[must_use]
pub fn rows(windows: Vec<ShellWindow>, index: &AppIndex) -> Vec<WindowInfo> {
    let apps = AppService::new(index);
    let mut rows: Vec<WindowInfo> = windows
        .into_iter()
        .map(|window| {
            let app = (!window.wm_class.is_empty())
                .then(|| apps.find_by_class(&window.wm_class))
                .flatten();
            WindowInfo {
                id: window.id,
                title: window.title,
                wm_class: window.wm_class,
                app_name: app.map(compass_core::AppItem::display_name),
                app_icon: app.and_then(|a| a.icon().map(str::to_owned)),
                pid: window.pid,
                workspace: window.workspace,
                focused: window.focused,
                can_close: window.can_close,
            }
        })
        .collect();
    rows.sort_by_key(|row| row.focused);
    rows
}

/// The refusal for a failed shell call, in words a user can act on.
#[must_use]
pub fn refusal(err: &ShellError, what: &str) -> ProtocolError {
    match err {
        ShellError::Unavailable(availability) => ProtocolError::new(
            ErrorKind::Unsupported,
            format!(
                "{what} needs the Compass GNOME Shell extension ({availability}). \
                 `vicinae doctor` shows how to install it"
            ),
        ),
        other => ProtocolError::new(ErrorKind::Internal, format!("{what} failed: {other}")),
    }
}

/// The refusal when the engine has no session bus at all.
#[must_use]
pub fn no_bus(what: &str) -> ProtocolError {
    ProtocolError::new(
        ErrorKind::Unsupported,
        format!("{what} needs a session bus, and the engine has none"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(id: u32, title: &str, wm_class: &str, focused: bool) -> ShellWindow {
        ShellWindow {
            id,
            title: title.into(),
            wm_class: wm_class.into(),
            focused,
            can_close: true,
            ..ShellWindow::default()
        }
    }

    fn index() -> (tempfile::TempDir, AppIndex) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("org.gnome.Nautilus.desktop"),
            "[Desktop Entry]\nType=Application\nName=Files\nExec=nautilus\nIcon=org.gnome.Nautilus\nStartupWMClass=org.gnome.Nautilus\n",
        )
        .unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        (dir, index)
    }

    #[test]
    fn a_window_is_named_by_its_application_when_one_matches() {
        let (_dir, index) = index();
        let rows = rows(
            vec![
                window(1, "Downloads", "org.gnome.Nautilus", false),
                window(2, "mystery", "some.unknown.App", false),
            ],
            &index,
        );
        assert_eq!(rows[0].app_name.as_deref(), Some("Files"));
        assert_eq!(rows[0].app_icon.as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(rows[1].app_name, None, "unknown classes are not guessed");
        assert_eq!(rows[1].wm_class, "some.unknown.App");
    }

    #[test]
    fn the_focused_window_goes_last_and_the_rest_keep_their_order() {
        let (_dir, index) = index();
        let ids: Vec<u32> = rows(
            vec![
                window(1, "a", "x", true),
                window(2, "b", "x", false),
                window(3, "c", "x", false),
            ],
            &index,
        )
        .iter()
        .map(|row| row.id)
        .collect();
        assert_eq!(ids, [2, 3, 1]);
    }

    #[test]
    fn a_missing_extension_is_refused_by_name_not_as_a_crash() {
        let refusal = refusal(
            &ShellError::Unavailable(compass_shell::Availability::Absent),
            "Window switching",
        );
        assert_eq!(refusal.kind, ErrorKind::Unsupported);
        assert!(
            refusal.message.contains("GNOME Shell extension"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("vicinae doctor"));
    }
}
