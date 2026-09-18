//! The `WindowManagement` half of the extension API.
//!
//! Ports `ExtWindowManagementService`
//! (`src/server/src/extension/api/wm-service.hpp`): seven methods over the
//! window manager, the application database and the app runtime, which are one
//! [`Windows`] trait here.
//!
//! # Value-initialised fields are still fields
//!
//! The IDL declares `Window::workspaceId` and `Window::bounds` as required, and
//! `serializeWindow` leaves them alone when the window has neither. glaze then
//! writes the value-initialised `""` and `{0,0,0,0}`, so an extension always
//! sees the keys. This writes them too: an absent `bounds` would be a different
//! shape from the one the C++ host sends.

use crate::application_service::Application;
use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "WindowManagement/focusWindow",
    "WindowManagement/getActiveWindow",
    "WindowManagement/getActiveWorkspace",
    "WindowManagement/getWindows",
    "WindowManagement/getScreens",
    "WindowManagement/getWorkspaces",
    "WindowManagement/setWindowBounds",
];

/// `getActiveWindow`'s failure, verbatim from the C++.
pub const NO_ACTIVE_WINDOW: &str = "No active window";
/// `getActiveWorkspace`'s failure, verbatim.
pub const NO_ACTIVE_WORKSPACE: &str = "No active workspace";
/// `setWindowBounds`'s failure when the id does not resolve, verbatim.
pub const NO_SUCH_WINDOW: &str = "No window with the given id";
/// `setWindowBounds`'s failure when the provider refuses, verbatim.
pub const BOUNDS_REFUSED: &str = "Failed to set window bounds";

/// `tsapi::Rect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i64,
    /// Top edge.
    pub y: i64,
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
}

/// `tsapi::Size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
}

/// A window, as the window manager knows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    /// The manager's own id.
    pub id: String,
    /// The window title.
    pub title: String,
    /// The workspace it is on, when the manager reports one.
    pub workspace_id: Option<String>,
    /// Whether it is full-screen.
    pub fullscreen: bool,
    /// Its geometry, when the manager reports any.
    pub bounds: Option<Rect>,
    /// The WM class, used to find the application that owns it.
    pub wm_class: String,
}

/// A monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screen {
    /// The connector name, e.g. `DP-1`.
    pub name: String,
    /// The model string.
    pub model: String,
    /// The manufacturer; `make` on the wire.
    pub make: String,
    /// The serial, when the monitor reports one.
    pub serial: Option<String>,
    /// Its place in the layout.
    pub bounds: Rect,
    /// Its native resolution.
    pub physical_resolution: Size,
    /// Whether it is the active one.
    pub active: bool,
}

/// A workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Workspace {
    /// The manager's own id.
    pub id: String,
    /// Its label.
    pub name: String,
    /// Whether any window on it is full-screen.
    pub fullscreen: bool,
    /// The monitor it lives on, when the manager reports one.
    pub monitor: Option<String>,
}

/// The window manager an extension can reach.
pub trait Windows {
    /// `WindowManager::findWindowById`.
    fn find_window(&self, id: &str) -> Option<Window>;

    /// `focusWindowSync`.
    fn focus(&self, window: &Window);

    /// `getFocusedWindow` / `getFocusedWindowSync`.
    fn focused_window(&self) -> Option<Window>;

    /// `AppRuntime::frontmostApp`, for platforms with no window-level focus.
    fn frontmost_app(&self) -> Option<Application> {
        None
    }

    /// `listWindowsSync`.
    fn list_windows(&self) -> Vec<Window>;

    /// `getActiveWorkspace`.
    fn active_workspace(&self) -> Option<Workspace>;

    /// `listWorkspaces`.
    fn list_workspaces(&self) -> Vec<Workspace>;

    /// `listScreensSync`.
    fn list_screens(&self) -> Vec<Screen>;

    /// `AppService::findByClass`, which puts an `app` on a window.
    fn app_for_class(&self, wm_class: &str) -> Option<Application>;

    /// `setWindowBounds`; `false` is the provider refusing.
    fn set_window_bounds(&self, window: &Window, bounds: Rect) -> bool;
}

/// Serves `WindowManagement` from one manager.
#[derive(Debug)]
pub struct WindowService<W> {
    windows: W,
}

impl<W: Windows> WindowService<W> {
    /// Serves `windows`.
    pub const fn new(windows: W) -> Self {
        Self { windows }
    }

    /// The manager this serves.
    pub const fn windows(&self) -> &W {
        &self.windows
    }

    /// Answers `call`, or `None` if it is not a `WindowManagement` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let string = |name: &str| {
            call.params
                .get(name)
                .filter(|value| !value.is_null())
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };

        Some(match call.method.as_str() {
            "WindowManagement/focusWindow" => {
                // An id that does not resolve is `ok(false)`, not a failure:
                // windows close between listing one and focusing it, and an
                // extension checking the bool handles that.
                let focused = match self
                    .windows
                    .find_window(&string("winId").unwrap_or_default())
                {
                    Some(window) => {
                        self.windows.focus(&window);
                        true
                    }
                    None => false,
                };
                tsapi::reply(id, serde_json::Value::Bool(focused))
            }
            "WindowManagement/getActiveWindow" => match self.active_window() {
                Some(window) => tsapi::reply(id, window),
                None => tsapi::reply_error(id, NO_ACTIVE_WINDOW),
            },
            "WindowManagement/getActiveWorkspace" => match self.windows.active_workspace() {
                // Always `active = true`: it is the active one by construction.
                Some(workspace) => tsapi::reply(id, self::workspace(&workspace, true)),
                None => tsapi::reply_error(id, NO_ACTIVE_WORKSPACE),
            },
            "WindowManagement/getWindows" => {
                let wanted = string("workspaceId");
                let focused = self.windows.focused_window();
                let windows: Vec<serde_json::Value> = self
                    .windows
                    .list_windows()
                    .iter()
                    // `win->workspace().value_or("") != *workspaceId` -- a window
                    // the manager gives no workspace matches only a request for
                    // the empty one.
                    .filter(|window| {
                        wanted.as_ref().is_none_or(|wanted| {
                            window.workspace_id.as_deref().unwrap_or("") == wanted
                        })
                    })
                    .map(|window| {
                        let active = focused.as_ref().is_some_and(|f| f.id == window.id);
                        self.serialize(window, active)
                    })
                    .collect();
                tsapi::reply(id, serde_json::Value::Array(windows))
            }
            "WindowManagement/getScreens" => {
                let screens: Vec<serde_json::Value> =
                    self.windows.list_screens().iter().map(screen).collect();
                tsapi::reply(id, serde_json::Value::Array(screens))
            }
            "WindowManagement/getWorkspaces" => {
                let active = self.windows.active_workspace();
                let workspaces: Vec<serde_json::Value> = self
                    .windows
                    .list_workspaces()
                    .iter()
                    .map(|ws| {
                        let is_active = active.as_ref().is_some_and(|a| a.id == ws.id);
                        workspace(ws, is_active)
                    })
                    .collect();
                tsapi::reply(id, serde_json::Value::Array(workspaces))
            }
            _ => {
                let Some(window) = self
                    .windows
                    .find_window(&string("winId").unwrap_or_default())
                else {
                    return Some(tsapi::reply_error(id, NO_SUCH_WINDOW));
                };
                let bounds = rect_from(call.params.get("bounds"));
                if self.windows.set_window_bounds(&window, bounds) {
                    tsapi::reply(id, serde_json::Value::Null)
                } else {
                    tsapi::reply_error(id, BOUNDS_REFUSED)
                }
            }
        })
    }

    /// `getActiveWindow`'s two sources.
    ///
    /// A focused window is the answer where the compositor reports one. Where
    /// it does not — macOS, per the C++ comment — the frontmost *application*
    /// stands in as a window carrying its id and display name, which is why an
    /// extension can get a `Window` with no bounds and no workspace.
    fn active_window(&self) -> Option<serde_json::Value> {
        if let Some(window) = self.windows.focused_window() {
            return Some(self.serialize(&window, true));
        }

        let front = self.windows.frontmost_app()?;
        let mut out = window_object(&Window {
            id: front.id.clone(),
            title: front.name.clone(),
            ..Window::default()
        });
        out["active"] = serde_json::Value::Bool(true);
        out["app"] = application(&front);
        Some(out)
    }

    /// One window, with its owning application attached when the class resolves.
    fn serialize(&self, window: &Window, active: bool) -> serde_json::Value {
        let mut out = window_object(window);
        out["active"] = serde_json::Value::Bool(active);
        if let Some(app) = self.windows.app_for_class(&window.wm_class) {
            out["app"] = application(&app);
        }
        out
    }
}

impl<W: Windows> tsapi::Service for WindowService<W> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The required half of a `Window`: everything but `active` and `app`.
fn window_object(window: &Window) -> serde_json::Value {
    serde_json::json!({
        "id": window.id,
        "title": window.title,
        "workspaceId": window.workspace_id.clone().unwrap_or_default(),
        "active": false,
        "fullscreen": window.fullscreen,
        "bounds": rect(window.bounds.unwrap_or_default()),
    })
}

fn rect(bounds: Rect) -> serde_json::Value {
    serde_json::json!({
        "x": bounds.x,
        "y": bounds.y,
        "width": bounds.width,
        "height": bounds.height,
    })
}

/// A `Rect` off the wire; a missing field is zero, as the C++ struct's is.
fn rect_from(bounds: Option<&serde_json::Value>) -> Rect {
    let field = |name: &str| {
        bounds
            .and_then(|bounds| bounds.get(name))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    };
    Rect {
        x: field("x"),
        y: field("y"),
        width: field("width"),
        height: field("height"),
    }
}

fn screen(screen: &Screen) -> serde_json::Value {
    let mut out = serde_json::json!({
        "name": screen.name,
        "model": screen.model,
        "make": screen.make,
        "bounds": rect(screen.bounds),
        "physicalResolution": {
            "width": screen.physical_resolution.width,
            "height": screen.physical_resolution.height,
        },
        "active": screen.active,
    });
    // `if (screen.serial) sc.serial = ...` -- optional, and omitted when the
    // monitor does not report one.
    if let Some(serial) = &screen.serial {
        out["serial"] = serde_json::Value::String(serial.clone());
    }
    out
}

fn workspace(workspace: &Workspace, active: bool) -> serde_json::Value {
    let mut out = serde_json::json!({
        "id": workspace.id,
        "name": workspace.name,
        "active": active,
        "fullscreen": workspace.fullscreen,
    });
    if let Some(monitor) = &workspace.monitor {
        out["monitor"] = serde_json::Value::String(monitor.clone());
    }
    out
}

fn application(app: &Application) -> serde_json::Value {
    serde_json::json!({
        "id": app.id,
        "name": app.name,
        "icon": app.icon,
        "path": app.path,
    })
}
