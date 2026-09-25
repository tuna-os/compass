//! niri over `$NIRI_SOCKET`.
//!
//! Ports `Niri::WindowManager` (`src/server/src/services/window-manager/niri/`)
//! on the `niri-ipc` crate, which is niri's own: its `Request`, `Reply` and
//! types are what niri serialises. The one thing taken around it is the
//! connection: `niri_ipc::socket::Socket` has no timeout, so this opens the
//! stream itself and reads one reply line under [`REQUEST_TIMEOUT`].
//!
//! The C++ keeps the event stream open and mirrors niri's state; this asks
//! `Windows`, `Workspaces` and `FocusedWindow` when it needs them, which gives
//! the same answers without a thread per engine.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use niri_ipc::{Action, Reply, Request, Response, WorkspaceReferenceArg};

use super::{IpcError, OwnWindows, REQUEST_TIMEOUT, WmWindow, WmWorkspace};

/// The variable niri exports its socket in.
pub const SOCKET_ENV: &str = niri_ipc::socket::SOCKET_PATH_ENV;

/// A niri instance's socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Niri {
    socket: PathBuf,
}

fn window(window: niri_ipc::Window) -> WmWindow {
    WmWindow {
        id: window.id.to_string(),
        title: window.title.unwrap_or_default(),
        wm_class: window.app_id.unwrap_or_default(),
        pid: window.pid.and_then(|pid| u32::try_from(pid).ok()),
        workspace: window.workspace_id.map(|id| id.to_string()),
        // niri reports a tile's place in its workspace view, not on the
        // screen; the C++ reports no bounds either.
        bounds: None,
        focused: window.is_focused,
        fullscreen: false,
    }
}

fn workspace(workspace: niri_ipc::Workspace) -> WmWorkspace {
    WmWorkspace {
        id: workspace.id.to_string(),
        name: workspace.name.unwrap_or_default(),
        number: Some(i32::from(workspace.idx)),
        monitor: workspace.output,
        has_fullscreen: false,
    }
}

/// Most recently focused first, windows never focused last, then by id
/// descending: the C++ `sortWindowsByFocusTimestamp`.
fn sort_by_focus(windows: &mut [niri_ipc::Window]) {
    windows.sort_by(|a, b| {
        let key = |w: &niri_ipc::Window| w.focus_timestamp.map(|t| (t.secs, t.nanos));
        match (key(a), key(b)) {
            (Some(a_ts), Some(b_ts)) if a_ts != b_ts => b_ts.cmp(&a_ts),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => b.id.cmp(&a.id),
        }
    });
}

fn parse_id(id: &str) -> Result<u64, IpcError> {
    id.parse()
        .map_err(|_| IpcError::Refused(format!("{id} is not a niri window id")))
}

impl Niri {
    /// A client for the socket at `socket`.
    #[must_use]
    pub const fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    /// The socket this talks to.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Sends `request` and reads niri's one-line reply.
    ///
    /// # Errors
    ///
    /// When the socket cannot be reached, the exchange breaks off, the reply
    /// is not niri's, or niri refuses.
    pub fn request(&self, request: &Request) -> Result<Response, IpcError> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        let mut line =
            serde_json::to_string(request).map_err(|err| IpcError::Parse(err.to_string()))?;
        line.push('\n');
        stream.write_all(line.as_bytes())?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply)?;
        let reply: Reply = serde_json::from_str(&reply).map_err(|err| {
            IpcError::Parse(format!(
                "{err} ({})",
                reply.chars().take(120).collect::<String>()
            ))
        })?;
        reply.map_err(IpcError::Refused)
    }

    fn raw_windows(&self) -> Result<Vec<niri_ipc::Window>, IpcError> {
        match self.request(&Request::Windows)? {
            Response::Windows(mut windows) => {
                sort_by_focus(&mut windows);
                Ok(windows)
            }
            other => Err(IpcError::Parse(format!("Windows answered {other:?}"))),
        }
    }

    fn raw_workspaces(&self) -> Result<Vec<niri_ipc::Workspace>, IpcError> {
        match self.request(&Request::Workspaces)? {
            Response::Workspaces(workspaces) => Ok(workspaces),
            other => Err(IpcError::Parse(format!("Workspaces answered {other:?}"))),
        }
    }

    fn action(&self, action: Action) -> Result<(), IpcError> {
        match self.request(&Request::Action(action))? {
            Response::Handled => Ok(()),
            other => Err(IpcError::Parse(format!("an action answered {other:?}"))),
        }
    }

    /// Every window, most recently focused first.
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn windows(&self) -> Result<Vec<WmWindow>, IpcError> {
        Ok(self.raw_windows()?.into_iter().map(window).collect())
    }

    /// `FocusedWindow`.
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn focused_window(&self) -> Result<Option<WmWindow>, IpcError> {
        match self.request(&Request::FocusedWindow)? {
            Response::FocusedWindow(focused) => Ok(focused.map(window)),
            other => Err(IpcError::Parse(format!("FocusedWindow answered {other:?}"))),
        }
    }

    /// The focused window unless it is the launcher's, else the most recently
    /// focused one that is not. niri reports focus on a layer surface as no
    /// focused window, so while the launcher is up this is the window before.
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn frontmost_window(&self, own: &OwnWindows) -> Result<Option<WmWindow>, IpcError> {
        let windows = self.windows()?;
        if let Some(focused) = windows.iter().find(|w| w.focused && !own.contains(w)) {
            return Ok(Some(focused.clone()));
        }
        Ok(windows.into_iter().find(|w| !own.contains(w)))
    }

    /// Every workspace.
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn workspaces(&self) -> Result<Vec<WmWorkspace>, IpcError> {
        Ok(self.raw_workspaces()?.into_iter().map(workspace).collect())
    }

    /// The focused workspace, else the first active one (the C++
    /// `getActiveWorkspace`).
    ///
    /// # Errors
    ///
    /// As [`Self::request`].
    pub fn active_workspace(&self) -> Result<Option<WmWorkspace>, IpcError> {
        let workspaces = self.raw_workspaces()?;
        let pick = workspaces
            .iter()
            .position(|w| w.is_focused)
            .or_else(|| workspaces.iter().position(|w| w.is_active));
        Ok(pick.map(|index| workspace(workspaces[index].clone())))
    }

    /// `FocusWindow`.
    ///
    /// # Errors
    ///
    /// When `id` is not a niri id, or as [`Self::request`].
    pub fn focus_window(&self, id: &str) -> Result<(), IpcError> {
        self.action(Action::FocusWindow { id: parse_id(id)? })
    }

    /// `CloseWindow`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn close_window(&self, id: &str) -> Result<(), IpcError> {
        self.action(Action::CloseWindow {
            id: Some(parse_id(id)?),
        })
    }

    /// `FocusWorkspace` by id.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn focus_workspace(&self, id: &str) -> Result<(), IpcError> {
        self.action(Action::FocusWorkspace {
            reference: WorkspaceReferenceArg::Id(parse_id(id)?),
        })
    }

    /// `Version`, as the C++ `ping`.
    #[must_use]
    pub fn ping(&self) -> bool {
        matches!(self.request(&Request::Version), Ok(Response::Version(_)))
    }
}
