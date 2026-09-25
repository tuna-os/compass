//! The window manager an extension reaches through `WindowManagement/*`.
//!
//! The same two backends the window switcher uses: the GNOME Shell
//! extension's window list on GNOME, the foreign-toplevel protocols on a
//! wlroots compositor. Monitors come from `wl_output`, which every compositor
//! carries. Where neither backend is there (a headless run, a TTY, a
//! compositor with no toplevel protocol) there are no windows, and
//! `getActiveWindow` fails with the C++'s own "No active window".
//!
//! # Which window is "active"
//!
//! An extension asks from inside the launcher, so the focused window is
//! usually the launcher itself. The C++ answers with the window focused
//! *before* it (`WindowManager::getFocusedWindow` remembers the last foreign
//! one); this takes the first window in the backend's most-recently-used
//! order that is not the launcher, which is the same window without a
//! focus-tracking loop to keep it.
//!
//! # What neither backend reports
//!
//! Workspaces as objects (the Shell contract gives a window's workspace
//! index, not a workspace list; the toplevel protocols give nothing), and a
//! way to move or resize a window: `getActiveWorkspace` fails with "No active
//! workspace", `getWorkspaces` is empty and `setWindowBounds` is refused.
//! PARITY, "The extension host API".

use std::sync::Arc;

use compass_core::app_windows::AppIdentity;
use compass_worker_host::application_service::Application;
use compass_worker_host::window_service::{Rect, Screen, Size, Window, Windows, Workspace};

/// Where the windows come from.
enum Backend {
    /// The GNOME Shell extension, reached on a runtime.
    Shell {
        client: Arc<compass_shell::ShellClient>,
        handle: tokio::runtime::Handle,
    },
    /// A wlroots compositor's toplevel list.
    Toplevels(Arc<compass_wayland::Toplevels>),
    /// Nothing to ask.
    None,
}

/// [`Windows`] over this session's window manager.
pub struct EngineWindows {
    backend: Backend,
    /// Every installed application, as `findByClass` matches them.
    classes: Vec<(AppIdentity, Application)>,
    /// Whether to ask the compositor for its outputs: off in a session with
    /// no Wayland display.
    outputs: bool,
}

impl std::fmt::Debug for EngineWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            Backend::Shell { .. } => "shell",
            Backend::Toplevels(_) => "toplevels",
            Backend::None => "none",
        };
        f.debug_struct("EngineWindows")
            .field("backend", &backend)
            .field("classes", &self.classes.len())
            .finish_non_exhaustive()
    }
}

impl EngineWindows {
    /// This session's window manager: the wlroots toplevel list when the
    /// compositor is one, else the Shell extension when there is a client.
    ///
    /// Blocking (wlroots detection is a registry round trip); called from
    /// the thread that serves the command.
    #[must_use]
    pub fn detect(
        shell: Option<Arc<compass_shell::ShellClient>>,
        handle: Option<tokio::runtime::Handle>,
        classes: Vec<(AppIdentity, Application)>,
    ) -> Self {
        let backend = if let Some(wlroots) = crate::wlroots::session() {
            wlroots
                .toplevels
                .clone()
                .map_or(Backend::None, Backend::Toplevels)
        } else if let (Some(client), Some(handle)) = (shell, handle) {
            Backend::Shell { client, handle }
        } else {
            Backend::None
        };
        Self {
            backend,
            classes,
            outputs: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        }
    }

    /// No window manager at all, for a session that has none.
    #[must_use]
    pub const fn none(classes: Vec<(AppIdentity, Application)>) -> Self {
        Self {
            backend: Backend::None,
            classes,
            outputs: false,
        }
    }

    /// The windows, most recently used first, with whether each is focused.
    fn windows(&self) -> Vec<(Window, bool)> {
        match &self.backend {
            Backend::Shell { client, handle } => {
                let client = Arc::clone(client);
                match handle.block_on(async move { client.list_windows().await }) {
                    Ok(windows) => windows.into_iter().map(from_shell).collect(),
                    Err(err) => {
                        tracing::info!(error = %err, "no window list for the extension");
                        Vec::new()
                    }
                }
            }
            Backend::Toplevels(toplevels) => toplevels
                .list()
                .into_iter()
                .map(|toplevel| {
                    let focused = toplevel.activated;
                    (
                        Window {
                            id: toplevel.id.to_string(),
                            title: toplevel.title,
                            wm_class: toplevel.app_id,
                            ..Window::default()
                        },
                        focused,
                    )
                })
                .collect(),
            Backend::None => Vec::new(),
        }
    }
}

fn from_shell(window: compass_shell::Window) -> (Window, bool) {
    let focused = window.focused;
    (
        Window {
            id: window.id.0.to_string(),
            title: window.title,
            workspace_id: window.workspace.map(|index| index.to_string()),
            fullscreen: window.fullscreen,
            bounds: window.frame.map(|frame| Rect {
                x: frame.x.into(),
                y: frame.y.into(),
                width: frame.width.into(),
                height: frame.height.into(),
            }),
            wm_class: window.wm_class,
        },
        focused,
    )
}

/// Whether `window` is the launcher's own.
fn own(window: &Window) -> bool {
    window.wm_class.eq_ignore_ascii_case(compass_ui::APP_ID)
}

/// The active window among `windows` (most recently used first): the
/// focused one, or the one focused before the launcher took focus.
fn active(windows: Vec<(Window, bool)>) -> Option<Window> {
    let launcher_focused = windows
        .iter()
        .any(|(window, focused)| *focused && own(window));
    windows
        .into_iter()
        .find(|(window, focused)| !own(window) && (*focused || launcher_focused))
        .map(|(window, _)| window)
}

/// `outputs` as screens, the one holding `focus`'s centre active; with one
/// monitor, that one.
fn screens(outputs: Vec<compass_wayland::output::Output>, focus: Option<Rect>) -> Vec<Screen> {
    let only = outputs.len() == 1;
    outputs
        .into_iter()
        .map(|output| {
            let bounds = Rect {
                x: output.x.into(),
                y: output.y.into(),
                width: output.width.into(),
                height: output.height.into(),
            };
            let active = only
                || focus.is_some_and(|focus| {
                    let (cx, cy) = (focus.x + focus.width / 2, focus.y + focus.height / 2);
                    (bounds.x..bounds.x + bounds.width).contains(&cx)
                        && (bounds.y..bounds.y + bounds.height).contains(&cy)
                });
            Screen {
                name: output.name,
                model: output.model,
                make: output.make,
                serial: None,
                bounds,
                physical_resolution: Size {
                    width: output.pixel_width.into(),
                    height: output.pixel_height.into(),
                },
                active,
            }
        })
        .collect()
}

impl Windows for EngineWindows {
    fn find_window(&self, id: &str) -> Option<Window> {
        self.windows()
            .into_iter()
            .map(|(window, _)| window)
            .find(|window| window.id == id)
    }

    fn focus(&self, window: &Window) {
        let Ok(id) = window.id.parse::<u32>() else {
            return;
        };
        match &self.backend {
            Backend::Shell { client, handle } => {
                let client = Arc::clone(client);
                let done = handle.block_on(async move {
                    client.activate_window(compass_shell::WindowId(id)).await
                });
                if let Err(err) = done {
                    tracing::info!(error = %err, "could not focus the extension's window");
                }
            }
            Backend::Toplevels(toplevels) => {
                if let Err(err) = toplevels.activate(id) {
                    tracing::info!(error = %err, "could not focus the extension's window");
                }
            }
            Backend::None => {}
        }
    }

    fn focused_window(&self) -> Option<Window> {
        active(self.windows())
    }

    fn list_windows(&self) -> Vec<Window> {
        self.windows()
            .into_iter()
            .map(|(window, _)| window)
            .filter(|window| !own(window))
            .collect()
    }

    fn active_workspace(&self) -> Option<Workspace> {
        None
    }

    fn list_workspaces(&self) -> Vec<Workspace> {
        Vec::new()
    }

    fn list_screens(&self) -> Vec<Screen> {
        if !self.outputs {
            return Vec::new();
        }
        match compass_wayland::output::list() {
            Ok(outputs) => screens(
                outputs,
                self.focused_window().and_then(|window| window.bounds),
            ),
            Err(err) => {
                tracing::info!(error = %err, "no monitors for the extension");
                Vec::new()
            }
        }
    }

    fn app_for_class(&self, wm_class: &str) -> Option<Application> {
        if wm_class.is_empty() {
            return None;
        }
        self.classes
            .iter()
            .find(|(identity, _)| identity.matches_window_class(wm_class))
            .map(|(_, app)| app.clone())
    }

    fn set_window_bounds(&self, _window: &Window, _bounds: Rect) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, wm_class: &str) -> Window {
        Window {
            id: id.into(),
            title: id.into(),
            wm_class: wm_class.into(),
            ..Window::default()
        }
    }

    #[test]
    fn the_focused_window_is_active_unless_it_is_the_launcher() {
        let editor = window("1", "org.gnome.TextEditor");
        assert_eq!(
            active(vec![
                (editor.clone(), true),
                (window("2", "firefox"), false)
            ]),
            Some(editor)
        );
    }

    #[test]
    fn with_the_launcher_focused_the_window_before_it_is_active() {
        let launcher = window("9", compass_ui::APP_ID);
        let before = window("1", "firefox");
        assert_eq!(
            active(vec![
                (launcher, true),
                (before.clone(), false),
                (window("2", "org.gnome.Nautilus"), false),
            ]),
            Some(before)
        );
    }

    #[test]
    fn nothing_focused_is_no_active_window() {
        assert_eq!(active(vec![(window("1", "firefox"), false)]), None);
        assert_eq!(active(Vec::new()), None);
    }

    fn output(name: &str, x: i32) -> compass_wayland::output::Output {
        compass_wayland::output::Output {
            name: name.into(),
            x,
            width: 1920,
            height: 1080,
            pixel_width: 3840,
            pixel_height: 2160,
            ..compass_wayland::output::Output::default()
        }
    }

    #[test]
    fn the_screen_under_the_focused_window_is_the_active_one() {
        let focus = Rect {
            x: 2000,
            y: 100,
            width: 800,
            height: 600,
        };
        let screens = screens(vec![output("DP-1", 0), output("DP-2", 1920)], Some(focus));
        assert_eq!(
            screens
                .iter()
                .map(|s| (s.name.as_str(), s.active))
                .collect::<Vec<_>>(),
            [("DP-1", false), ("DP-2", true)]
        );
        assert_eq!(screens[1].physical_resolution.width, 3840);
        assert_eq!(screens[1].bounds.width, 1920);
    }

    #[test]
    fn a_lone_screen_is_active_even_with_no_window_to_place() {
        assert!(screens(vec![output("eDP-1", 0)], None)[0].active);
    }

    #[test]
    fn a_window_is_matched_to_its_application_by_class() {
        let windows = EngineWindows::none(vec![(
            AppIdentity {
                desktop_id: "org.gnome.Nautilus.desktop".into(),
                startup_wm_class: None,
                display_name: "Files".into(),
            },
            Application {
                id: "org.gnome.Nautilus.desktop".into(),
                name: "Files".into(),
                ..Application::default()
            },
        )]);
        assert_eq!(
            windows
                .app_for_class("org.gnome.Nautilus")
                .map(|app| app.name),
            Some("Files".to_owned())
        );
        assert_eq!(windows.app_for_class(""), None);
        assert_eq!(windows.list_windows(), Vec::new());
        assert_eq!(windows.focused_window(), None);
    }
}
