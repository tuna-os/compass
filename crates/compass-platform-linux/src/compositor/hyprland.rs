//! Hyprland over its request socket.
//!
//! Ports `Hyprland::Controller` and `HyprlandWindowManager`
//! (`src/server/src/services/window-manager/hyprland/`). The protocol is the
//! one `hyprctl` speaks: connect to
//! `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`, write
//! one command, read until the compositor closes the connection. `-j/` asks
//! for JSON.
//!
//! Hand-rolled rather than the `hyprland` crate: see CRATE-AUDIT, "Hyprland
//! IPC". The replies are read the way the C++ reads them — the handful of
//! fields it uses, every one optional, unknown ones ignored — because
//! Hyprland adds and retypes fields between releases.
//!
//! # Dispatching
//!
//! The C++ dispatches with Hyprland's Lua expressions
//! (`dispatch hl.dsp.focus({ window = "address:0x…" })`), which is what
//! current Hyprland takes, and treats any reply but `ok` as failure. A
//! Hyprland from before the Lua dispatchers refuses those, so a refusal is
//! retried once with the classic dispatcher (`dispatch focuswindow
//! address:0x…`) before it counts. PARITY, "wlroots".

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{IpcError, OwnWindows, REQUEST_TIMEOUT, WmBounds, WmWindow, WmWorkspace};

/// The request socket for `signature` under `runtime_dir` (else `/tmp`, as
/// the C++ falls back).
#[must_use]
pub fn socket_path(runtime_dir: Option<&Path>, signature: &str) -> PathBuf {
    runtime_dir
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("hypr")
        .join(signature)
        .join(".socket.sock")
}

/// A Hyprland instance's request socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyprland {
    socket: PathBuf,
}

/// `Hyprland::ipc::Window`: what `clients` and `activewindow` carry.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Client {
    address: String,
    title: String,
    class: String,
    workspace: WorkspaceRef,
    pid: i64,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i64,
    at: Option<[i64; 2]>,
    size: Option<[i64; 2]>,
    fullscreen: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct WorkspaceRef {
    id: i64,
}

impl Default for WorkspaceRef {
    fn default() -> Self {
        Self { id: -1 }
    }
}

/// `Hyprland::ipc::Workspace`.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct Workspace {
    id: i64,
    name: String,
    #[serde(rename = "hasfullscreen")]
    has_fullscreen: bool,
    monitor: String,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            id: -1,
            name: String::new(),
            has_fullscreen: false,
            monitor: String::new(),
        }
    }
}

impl Client {
    fn into_window(self, focused_address: Option<&str>) -> WmWindow {
        let bounds = match (self.at, self.size) {
            (Some([x, y]), Some([width, height])) => Some(WmBounds {
                x,
                y,
                width,
                height,
            }),
            _ => None,
        };
        WmWindow {
            focused: focused_address.is_some_and(|focused| focused == self.address),
            id: self.address,
            title: self.title,
            wm_class: self.class,
            pid: u32::try_from(self.pid).ok().filter(|pid| *pid > 0),
            workspace: Some(self.workspace.id.to_string()),
            bounds,
            // `0`/`false` is windowed; anything else is one of Hyprland's
            // full-screen modes.
            fullscreen: match &self.fullscreen {
                serde_json::Value::Bool(on) => *on,
                serde_json::Value::Number(mode) => mode.as_i64().is_some_and(|mode| mode != 0),
                _ => false,
            },
        }
    }
}

impl From<Workspace> for WmWorkspace {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id.to_string(),
            name: workspace.name,
            number: i32::try_from(workspace.id).ok(),
            monitor: (!workspace.monitor.is_empty()).then_some(workspace.monitor),
            has_fullscreen: workspace.has_fullscreen,
        }
    }
}

impl Hyprland {
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

    /// Sends `command` and returns the whole reply.
    ///
    /// # Errors
    ///
    /// When the socket cannot be reached or the exchange breaks off.
    pub fn request(&self, command: &str) -> Result<Vec<u8>, IpcError> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        stream.write_all(command.as_bytes())?;
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply)?;
        Ok(reply)
    }

    fn json<T: serde::de::DeserializeOwned>(&self, command: &str) -> Result<T, IpcError> {
        let reply = self.request(command)?;
        serde_json::from_slice(&reply).map_err(|err| {
            IpcError::Parse(format!(
                "{command}: {err} ({})",
                String::from_utf8_lossy(&reply)
                    .chars()
                    .take(120)
                    .collect::<String>()
            ))
        })
    }

    fn active_client(&self) -> Result<Option<Client>, IpcError> {
        // No focused window is `{}`, which reads as a client with no address.
        let active: Client = self.json("-j/activewindow")?;
        Ok((!active.address.is_empty()).then_some(active))
    }

    /// `listWindowsSync`: `clients`, each marked focused if it is the
    /// `activewindow`.
    ///
    /// # Errors
    ///
    /// When Hyprland cannot be reached or answers nonsense.
    pub fn windows(&self) -> Result<Vec<WmWindow>, IpcError> {
        let clients: Vec<Client> = self.json("-j/clients")?;
        let focused = self.active_client().ok().flatten().map(|c| c.address);
        Ok(clients
            .into_iter()
            .map(|client| client.into_window(focused.as_deref()))
            .collect())
    }

    /// `getFocusedWindowSync`: `activewindow`.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn focused_window(&self) -> Result<Option<WmWindow>, IpcError> {
        Ok(self.active_client()?.map(|client| {
            let address = client.address.clone();
            client.into_window(Some(&address))
        }))
    }

    /// `getFrontmostWindowSync`: on the active workspace, not the launcher's,
    /// the lowest `focusHistoryID` that is not negative.
    ///
    /// Hyprland keeps reporting the window before a layer surface took focus,
    /// so this is the window the person was in even while the launcher is up.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn frontmost_window(&self, own: &OwnWindows) -> Result<Option<WmWindow>, IpcError> {
        let clients: Vec<Client> = self.json("-j/clients")?;
        let active: Workspace = self.json("-j/activeworkspace")?;
        let focused = self.active_client().ok().flatten().map(|c| c.address);
        Ok(clients
            .into_iter()
            .filter(|client| client.workspace.id == active.id && client.focus_history_id >= 0)
            .map(|client| {
                let history = client.focus_history_id;
                (history, client.into_window(focused.as_deref()))
            })
            .filter(|(_, window)| !own.contains(window))
            .min_by_key(|(history, _)| *history)
            .map(|(_, window)| window))
    }

    /// `listWorkspaces`.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn workspaces(&self) -> Result<Vec<WmWorkspace>, IpcError> {
        let workspaces: Vec<Workspace> = self.json("-j/workspaces")?;
        Ok(workspaces.into_iter().map(Into::into).collect())
    }

    /// `getActiveWorkspace`.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn active_workspace(&self) -> Result<Option<WmWorkspace>, IpcError> {
        let active: Workspace = self.json("-j/activeworkspace")?;
        Ok((active.id != -1 || !active.name.is_empty()).then(|| active.into()))
    }

    /// Runs `lua`, else `classic` when Hyprland refuses the Lua form.
    fn dispatch(&self, lua: &str, classic: &str) -> Result<(), IpcError> {
        let first = self.request(&format!("dispatch {lua}"))?;
        if first.trim_ascii() == b"ok" {
            return Ok(());
        }
        let second = self.request(&format!("dispatch {classic}"))?;
        if second.trim_ascii() == b"ok" {
            return Ok(());
        }
        Err(IpcError::Refused(
            String::from_utf8_lossy(&first).trim().to_owned(),
        ))
    }

    /// `focusWindowSync`.
    ///
    /// # Errors
    ///
    /// When Hyprland refuses both dispatchers or cannot be reached.
    pub fn focus_window(&self, address: &str) -> Result<(), IpcError> {
        self.dispatch(
            &format!(r#"hl.dsp.focus({{ window = "address:{address}" }})"#),
            &format!("focuswindow address:{address}"),
        )
    }

    /// `closeWindow`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn close_window(&self, address: &str) -> Result<(), IpcError> {
        self.dispatch(
            &format!(r#"hl.dsp.window.close({{ window = "address:{address}" }})"#),
            &format!("closewindow address:{address}"),
        )
    }

    /// `focusWorkspaceSync`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn focus_workspace(&self, id: &str) -> Result<(), IpcError> {
        self.dispatch(
            &format!(r#"hl.dsp.focus({{ workspace = "{id}" }})"#),
            &format!("workspace {id}"),
        )
    }

    /// `toggleFullscreen`: the window `address`, else (the classic form) the
    /// active window, which is the one the commands pass.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn toggle_fullscreen(&self, address: &str) -> Result<(), IpcError> {
        self.dispatch(
            &format!(
                r#"hl.dsp.window.fullscreen({{ action = "toggle", window = "address:{address}" }})"#
            ),
            "fullscreen 0",
        )
    }

    /// `toggleFloating`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn toggle_floating(&self, address: &str) -> Result<(), IpcError> {
        self.dispatch(
            &format!(
                r#"hl.dsp.window.float({{ action = "toggle", window = "address:{address}" }})"#
            ),
            &format!("togglefloating address:{address}"),
        )
    }

    /// Whether Hyprland answers `version`. (The C++ `ping` always says yes.)
    #[must_use]
    pub fn ping(&self) -> bool {
        self.request("-j/version")
            .is_ok_and(|reply| serde_json::from_slice::<serde_json::Value>(&reply).is_ok())
    }
}
