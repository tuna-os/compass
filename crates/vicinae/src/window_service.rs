//! Window switching: the Shell extension's windows, as switcher rows.
//!
//! On GNOME the extension is the only way to list or raise another
//! application's window (Mutter implements no foreign-toplevel protocol, and
//! `org.gnome.Shell.Introspect` is allowlisted to the portal backends). So
//! the engine holds one `compass_shell` client and answers the window
//! requests through it; without the extension they are refused by name.

use compass_core::{AppIndex, app_service::AppService};
use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, WindowInfo};
use compass_platform_linux::compositor::{Provider, WmWindow, WmWorkspace};
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

impl From<compass_wayland::Toplevel> for ShellWindow {
    fn from(window: compass_wayland::Toplevel) -> Self {
        Self {
            id: window.id,
            title: window.title,
            wm_class: window.app_id,
            // Neither toplevel protocol reports a pid or a workspace.
            pid: None,
            workspace: None,
            focused: window.activated,
            can_close: window.can_act,
        }
    }
}

/// `ListWindows` on a wlroots compositor, or `None` when this session is not
/// one and the Shell extension path should answer instead.
///
/// The launcher's own window is left out by `app_id`: the toplevel protocols
/// carry no pid, and on the `xdg_toplevel` surface the launcher is a window
/// like any other.
pub async fn wlroots_list(index: &AppIndex) -> Option<compass_ipc::Response> {
    let toplevels = wlroots_toplevels().await?;
    let mut windows: Vec<ShellWindow> = match toplevels {
        Ok(toplevels) => toplevels
            .list()
            .into_iter()
            .filter(|window| !window.app_id.eq_ignore_ascii_case(compass_ui::APP_ID))
            .map(Into::into)
            .collect(),
        Err(refusal) => return Some(compass_ipc::Response::Error(refusal)),
    };
    if let Some(provider) = crate::wlroots::compositor() {
        let provider = provider.clone();
        let known = tokio::task::spawn_blocking(move || {
            Some((
                provider.windows().ok()?,
                provider.workspaces().unwrap_or_default(),
            ))
        })
        .await
        .ok()
        .flatten();
        if let Some((known, workspaces)) = known {
            enrich(&mut windows, &known, &workspaces);
        }
    }
    Some(compass_ipc::Response::Windows {
        windows: rows(windows, index),
    })
}

/// Gives each toplevel the pid and workspace number the compositor's own IPC
/// reports for it (Hyprland, niri), matched by class and title: the
/// toplevel protocols carry neither.
pub fn enrich(
    windows: &mut [ShellWindow],
    known: &[compass_platform_linux::compositor::WmWindow],
    workspaces: &[compass_platform_linux::compositor::WmWorkspace],
) {
    let pairs = compass_platform_linux::compositor::match_toplevels(
        windows
            .iter()
            .map(|window| (window.wm_class.as_str(), window.title.as_str())),
        known,
    );
    for (window, pair) in windows.iter_mut().zip(pairs) {
        let Some(found) = pair.map(|index| &known[index]) else {
            continue;
        };
        window.pid = window.pid.or(found.pid);
        window.workspace = window.workspace.or_else(|| {
            let id = found.workspace.as_deref()?;
            workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .and_then(|workspace| workspace.number)
                .or_else(|| id.parse().ok())
        });
    }
}

/// `ListWindows` on KWin, or `None` when this session's compositor is not
/// KWin. The tracker's cache is the window list; the desktops are asked for
/// to number each window's desktop as a person counts them.
pub async fn kwin_list(index: &AppIndex) -> Option<compass_ipc::Response> {
    let Some(Provider::Kwin(kwin)) = crate::wlroots::compositor() else {
        return None;
    };
    let desktops = kwin.desktops().await.unwrap_or_else(|err| {
        tracing::info!(error = %err, "no virtual desktops from KWin");
        Vec::new()
    });
    Some(compass_ipc::Response::Windows {
        windows: rows(kwin_windows(kwin.windows(), &desktops), index),
    })
}

/// KWin's tracked windows as switcher windows: the launcher's own left out
/// (by pid, else by class), each desktop id turned into its number.
#[must_use]
pub fn kwin_windows(windows: Vec<WmWindow>, desktops: &[WmWorkspace]) -> Vec<ShellWindow> {
    let own = compass_platform_linux::compositor::OwnWindows {
        pids: vec![std::process::id()],
        classes: vec![compass_ui::APP_ID.to_owned()],
    };
    windows
        .into_iter()
        .filter(|window| !own.contains(window))
        .filter_map(|window| {
            Some(ShellWindow {
                id: window.id.parse().ok()?,
                workspace: window.workspace.as_deref().and_then(|id| {
                    desktops
                        .iter()
                        .find(|desktop| desktop.id == id)
                        .and_then(|desktop| desktop.number)
                }),
                title: window.title,
                wm_class: window.wm_class,
                pid: window.pid,
                focused: window.focused,
                can_close: true,
            })
        })
        .collect()
}

/// `ActivateWindow` / `CloseWindow` on KWin, or `None` when this session's
/// compositor is not KWin.
pub async fn kwin_act(id: u32, close: bool, what: &str) -> Option<compass_ipc::Response> {
    use compass_platform_linux::compositor::{IpcError, kwin::Action};
    let Some(Provider::Kwin(kwin)) = crate::wlroots::compositor() else {
        return None;
    };
    let action = if close { Action::Close } else { Action::Focus };
    Some(match kwin.act(&id.to_string(), action).await {
        Ok(()) => compass_ipc::Response::Ack,
        Err(err) => {
            let kind = if matches!(err, IpcError::Refused(_)) {
                ErrorKind::BadRequest
            } else {
                ErrorKind::Internal
            };
            compass_ipc::Response::Error(ProtocolError::new(kind, format!("{what} failed: {err}")))
        }
    })
}

/// `ActivateWindow` / `CloseWindow` on a wlroots compositor, or `None` when
/// this session is not one.
pub async fn wlroots_act(id: u32, close: bool, what: &str) -> Option<compass_ipc::Response> {
    let toplevels = match wlroots_toplevels().await? {
        Ok(toplevels) => toplevels,
        Err(refusal) => return Some(compass_ipc::Response::Error(refusal)),
    };
    let done = if close {
        toplevels.close(id)
    } else {
        toplevels.activate(id)
    };
    Some(match done {
        Ok(()) => compass_ipc::Response::Ack,
        Err(err) => compass_ipc::Response::Error(wlroots_refusal(&err, what)),
    })
}

/// `None` off wlroots; on it, the window list or the reason there is none.
async fn wlroots_toplevels() -> Option<Result<Arc<compass_wayland::Toplevels>, ProtocolError>> {
    let session = crate::wlroots::detect().await?;
    Some(session.toplevels.clone().ok_or_else(|| {
        ProtocolError::new(
            ErrorKind::Unsupported,
            "window switching needs zwlr_foreign_toplevel_manager_v1 or \
             ext_foreign_toplevel_list_v1, and this compositor advertises neither \
             (a sandboxed client may be denied them; `vicinae doctor` says which)",
        )
    }))
}

/// The refusal for a failed toplevel request.
#[must_use]
pub fn wlroots_refusal(err: &compass_wayland::ToplevelError, what: &str) -> ProtocolError {
    use compass_wayland::ToplevelError;
    let kind = match err {
        ToplevelError::NoSuchWindow(_) => ErrorKind::BadRequest,
        ToplevelError::ListOnly | ToplevelError::Unsupported | ToplevelError::NoSeat => {
            ErrorKind::Unsupported
        }
        _ => ErrorKind::Internal,
    };
    ProtocolError::new(kind, format!("{what} failed: {err}"))
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
    fn toplevels_learn_their_pid_and_workspace_number_from_the_compositor() {
        use compass_platform_linux::compositor::{WmWindow, WmWorkspace};
        let mut windows = vec![
            window(1, "~", "foot", false),
            window(2, "Spotify Premium", "spotify", false),
            window(3, "film", "mpv", false),
        ];
        let known = [
            WmWindow {
                id: "4".into(),
                title: "Spotify Premium".into(),
                wm_class: "spotify".into(),
                pid: Some(3120),
                workspace: Some("2".into()),
                ..WmWindow::default()
            },
            WmWindow {
                id: "7".into(),
                title: "~".into(),
                wm_class: "foot".into(),
                pid: Some(2398),
                workspace: Some("9".into()),
                ..WmWindow::default()
            },
        ];
        // niri: the workspace id is not its number; its index is.
        let workspaces = [WmWorkspace {
            id: "2".into(),
            number: Some(1),
            ..WmWorkspace::default()
        }];
        enrich(&mut windows, &known, &workspaces);
        assert_eq!(
            windows
                .iter()
                .map(|w| (w.id, w.pid, w.workspace))
                .collect::<Vec<_>>(),
            [
                (1, Some(2398), Some(9)),
                (2, Some(3120), Some(1)),
                (3, None, None)
            ]
        );
    }

    #[test]
    fn kwin_windows_leave_out_the_launcher_and_number_their_desktop() {
        let window = |id: &str, class: &str, pid: u32, desktop: Option<&str>| WmWindow {
            id: id.into(),
            title: format!("window {id}"),
            wm_class: class.into(),
            pid: Some(pid),
            workspace: desktop.map(str::to_owned),
            ..WmWindow::default()
        };
        let desktops = [
            WmWorkspace {
                id: "0b1c".into(),
                number: Some(1),
                ..WmWorkspace::default()
            },
            WmWorkspace {
                id: "9f2e".into(),
                number: Some(2),
                ..WmWorkspace::default()
            },
        ];
        let windows = kwin_windows(
            vec![
                window("1", "firefox", 100, Some("9f2e")),
                window("2", "konsole", std::process::id(), None),
                window("3", compass_ui::APP_ID, 7, None),
                window("4", "dolphin", 300, None),
                window("5", "kate", 400, Some("gone")),
            ],
            &desktops,
        );
        assert_eq!(
            windows
                .iter()
                .map(|w| (w.id, w.workspace, w.can_close))
                .collect::<Vec<_>>(),
            [(1, Some(2), true), (4, None, true), (5, None, true)]
        );
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
