//! Compositor IPC for the wlroots family members that have one.
//!
//! Ports the C++ Hyprland and niri window-manager providers
//! (`src/server/src/services/window-manager/{hyprland,niri}`). The toplevel
//! protocols every wlroots compositor carries list windows with a title and
//! an app id and nothing else; these two compositors also answer over their
//! own sockets with what the protocols leave out — the owning pid, the
//! workspace, the geometry, the workspace list and which one is active.
//!
//! Each provider is a blocking client: one short connection per request, the
//! way the C++ `Hyprctl::oneshot` and niri's `sendRequest` work, with a read
//! timeout so a wedged compositor cannot hold a caller for ever. Neither keeps
//! an event stream open: everything here is asked for when it is needed.
//!
//! [`Provider::detect`] chooses from the environment the compositor exports
//! to its clients, in the C++ order (Hyprland is a candidate before niri).

use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod hyprland;
pub mod niri;

/// How long one request may take to answer.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// A window, as the compositor reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WmWindow {
    /// The compositor's own id: a Hyprland address (`0x…`), a niri number.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its class (Hyprland) or app id (niri).
    pub wm_class: String,
    /// The process that owns it, when the compositor knows.
    pub pid: Option<u32>,
    /// The id of the workspace it is on, when it is on one.
    pub workspace: Option<String>,
    /// Its geometry in the compositor's layout, when reported.
    pub bounds: Option<WmBounds>,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it is full-screen, when reported.
    pub fullscreen: bool,
}

/// A window's place and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WmBounds {
    /// Left edge.
    pub x: i64,
    /// Top edge.
    pub y: i64,
    /// Width.
    pub width: i64,
    /// Height.
    pub height: i64,
}

/// A workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WmWorkspace {
    /// The compositor's own id.
    pub id: String,
    /// Its label: its name, else its id.
    pub name: String,
    /// The number a person knows it by: Hyprland's id, niri's index on its
    /// monitor.
    pub number: Option<i32>,
    /// The monitor it is on.
    pub monitor: Option<String>,
    /// Whether a window on it is full-screen.
    pub has_fullscreen: bool,
}

/// Why a request got no answer.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// The socket could not be reached or the exchange broke off.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The compositor answered with something this cannot read.
    #[error("unreadable reply: {0}")]
    Parse(String),
    /// The compositor refused the request.
    #[error("refused: {0}")]
    Refused(String),
}

/// One of the compositors with an IPC socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    /// Hyprland, over `.socket.sock`.
    Hyprland(hyprland::Hyprland),
    /// niri, over `$NIRI_SOCKET`.
    Niri(niri::Niri),
}

impl Provider {
    /// The compositor this process runs under, from the environment it
    /// exports: `HYPRLAND_INSTANCE_SIGNATURE` for Hyprland (the C++
    /// `isActivatable`), an existing `$NIRI_SOCKET` for niri.
    #[must_use]
    pub fn detect() -> Option<Self> {
        Self::detect_from(|name| std::env::var_os(name))
    }

    /// [`Self::detect`] over any environment.
    #[must_use]
    pub fn detect_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<Self> {
        if let Some(signature) = var("HYPRLAND_INSTANCE_SIGNATURE") {
            let runtime = var("XDG_RUNTIME_DIR").map(PathBuf::from);
            return Some(Self::Hyprland(hyprland::Hyprland::new(
                hyprland::socket_path(runtime.as_deref(), &signature.to_string_lossy()),
            )));
        }
        let socket = var(niri::SOCKET_ENV).map(PathBuf::from)?;
        socket.exists().then(|| Self::Niri(niri::Niri::new(socket)))
    }

    /// The provider's id, as the C++ names it.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        match self {
            Self::Hyprland(_) => "hyprland",
            Self::Niri(_) => "niri",
        }
    }

    /// Its name for people.
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Hyprland(_) => "Hyprland",
            Self::Niri(_) => "niri",
        }
    }

    /// The socket it talks to.
    #[must_use]
    pub fn socket(&self) -> &Path {
        match self {
            Self::Hyprland(hyprland) => hyprland.socket(),
            Self::Niri(niri) => niri.socket(),
        }
    }

    /// Every window. Hyprland's in its own order, niri's most recently
    /// focused first (the C++ `sortWindowsByFocusTimestamp`).
    ///
    /// # Errors
    ///
    /// When the compositor cannot be reached or answers nonsense.
    pub fn windows(&self) -> Result<Vec<WmWindow>, IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.windows(),
            Self::Niri(niri) => niri.windows(),
        }
    }

    /// The window that has keyboard focus, if one does.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn focused_window(&self) -> Result<Option<WmWindow>, IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.focused_window(),
            Self::Niri(niri) => niri.focused_window(),
        }
    }

    /// The window the person was last in on the active workspace, skipping
    /// the processes and classes in `own`: Hyprland's `getFrontmostWindowSync`
    /// (lowest `focusHistoryID`), niri's focused window else its most recent.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn frontmost_window(&self, own: &OwnWindows) -> Result<Option<WmWindow>, IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.frontmost_window(own),
            Self::Niri(niri) => niri.frontmost_window(own),
        }
    }

    /// Every workspace.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn workspaces(&self) -> Result<Vec<WmWorkspace>, IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.workspaces(),
            Self::Niri(niri) => niri.workspaces(),
        }
    }

    /// The active workspace: Hyprland's `activeworkspace`, niri's focused
    /// one else the first active one.
    ///
    /// # Errors
    ///
    /// As [`Self::windows`].
    pub fn active_workspace(&self) -> Result<Option<WmWorkspace>, IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.active_workspace(),
            Self::Niri(niri) => niri.active_workspace(),
        }
    }

    /// Focuses the window `id`.
    ///
    /// # Errors
    ///
    /// When the compositor refuses or cannot be reached.
    pub fn focus_window(&self, id: &str) -> Result<(), IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.focus_window(id),
            Self::Niri(niri) => niri.focus_window(id),
        }
    }

    /// Closes the window `id`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn close_window(&self, id: &str) -> Result<(), IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.close_window(id),
            Self::Niri(niri) => niri.close_window(id),
        }
    }

    /// Switches to the workspace `id`.
    ///
    /// # Errors
    ///
    /// As [`Self::focus_window`].
    pub fn focus_workspace(&self, id: &str) -> Result<(), IpcError> {
        match self {
            Self::Hyprland(hyprland) => hyprland.focus_workspace(id),
            Self::Niri(niri) => niri.focus_workspace(id),
        }
    }

    /// Whether the compositor answers at all.
    #[must_use]
    pub fn ping(&self) -> bool {
        match self {
            Self::Hyprland(hyprland) => hyprland.ping(),
            Self::Niri(niri) => niri.ping(),
        }
    }
}

/// What counts as the launcher's own windows, which "the window the person
/// was in" must skip: the C++ `isOwnWindow` compares the pid, else the class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnWindows {
    /// Processes whose windows are the launcher's.
    pub pids: Vec<u32>,
    /// Classes (app ids) that are the launcher's, compared without case.
    pub classes: Vec<String>,
}

impl OwnWindows {
    /// Whether `window` is one of the launcher's.
    #[must_use]
    pub fn contains(&self, window: &WmWindow) -> bool {
        window.pid.is_some_and(|pid| self.pids.contains(&pid))
            || self
                .classes
                .iter()
                .any(|class| class.eq_ignore_ascii_case(&window.wm_class))
    }
}

/// Pairs each of `toplevels` (`(app_id, title)`, in their order) with the
/// compositor window it is, when one matches: same class and title, each
/// compositor window used once, first come first served.
///
/// The toplevel protocols and the compositor sockets number windows
/// differently and neither carries the other's id, so the window switcher
/// learns a toplevel's pid and workspace this way. Two windows of one
/// application with one title are told apart only by order, which can pair
/// them the wrong way round; both then show the same application anyway.
#[must_use]
pub fn match_toplevels<'a>(
    toplevels: impl IntoIterator<Item = (&'a str, &'a str)>,
    windows: &[WmWindow],
) -> Vec<Option<usize>> {
    let mut used = vec![false; windows.len()];
    toplevels
        .into_iter()
        .map(|(app_id, title)| {
            let found = windows.iter().enumerate().position(|(index, window)| {
                !used[index] && window.title == title && window.wm_class == app_id
            })?;
            used[found] = true;
            Some(found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, class: &str, title: &str) -> WmWindow {
        WmWindow {
            id: id.into(),
            wm_class: class.into(),
            title: title.into(),
            ..WmWindow::default()
        }
    }

    #[test]
    fn toplevels_pair_with_the_compositor_window_of_the_same_class_and_title() {
        let windows = [
            window("0xa", "firefox", "Mozilla Firefox"),
            window("0xb", "foot", "~"),
            window("0xc", "foot", "~"),
        ];
        assert_eq!(
            match_toplevels(
                [
                    ("foot", "~"),
                    ("firefox", "Mozilla Firefox"),
                    ("foot", "~"),
                    ("foot", "~"),
                    ("mpv", "film"),
                ],
                &windows
            ),
            [Some(1), Some(0), Some(2), None, None]
        );
    }

    #[test]
    fn hyprland_is_detected_before_niri_and_niri_needs_its_socket() {
        let dir = tempfile::tempdir().unwrap();
        let niri_socket = dir.path().join("niri.sock");
        let env = |pairs: Vec<(&'static str, std::ffi::OsString)>| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.clone())
            }
        };

        let both = Provider::detect_from(env(vec![
            ("HYPRLAND_INSTANCE_SIGNATURE", "abc".into()),
            ("XDG_RUNTIME_DIR", dir.path().into()),
            ("NIRI_SOCKET", niri_socket.clone().into()),
        ]));
        let Some(Provider::Hyprland(hyprland)) = both else {
            panic!("not Hyprland: {both:?}");
        };
        assert_eq!(
            hyprland.socket(),
            dir.path().join("hypr/abc/.socket.sock").as_path()
        );

        assert_eq!(
            Provider::detect_from(env(vec![("NIRI_SOCKET", niri_socket.clone().into())])),
            None,
            "a socket path that does not exist is not niri"
        );
        std::fs::write(&niri_socket, "").unwrap();
        assert!(matches!(
            Provider::detect_from(env(vec![("NIRI_SOCKET", niri_socket.into())])),
            Some(Provider::Niri(_))
        ));
        assert_eq!(Provider::detect_from(env(Vec::new())), None);
    }

    #[test]
    fn the_launcher_is_its_pid_or_its_class() {
        let own = OwnWindows {
            pids: vec![42],
            classes: vec!["com.vicinae.Vicinae".into()],
        };
        let mut launcher = window("1", "COM.VICINAE.VICINAE", "Compass");
        assert!(own.contains(&launcher));
        launcher.wm_class = "other".into();
        launcher.pid = Some(42);
        assert!(own.contains(&launcher));
        launcher.pid = Some(7);
        assert!(!own.contains(&launcher));
    }
}
