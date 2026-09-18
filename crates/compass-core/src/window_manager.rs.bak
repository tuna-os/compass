//! Choosing a window-manager backend, and remembering what had focus.
//!
//! Ports `src/server/src/services/window-manager/window-manager.cpp` — the
//! layer above the per-compositor providers. None of the providers themselves
//! are here; what is here is which one gets picked, and the focus bookkeeping
//! that lets the launcher act on the window the user was in *before* they
//! opened the launcher.

/// The provider ids, in the order they are offered a chance to activate.
///
/// Order is the whole mechanism: the first that says it can run wins. The
/// specific compositors come first and the generic Wayland implementation
/// **last**, because it is good enough for most standalone compositors and
/// would therefore claim Hyprland and Niri too if it were asked first — and
/// then neither would get its own workspace support.
pub const LINUX_PROVIDER_ORDER: &[&str] = &["hyprland", "gnome", "kde", "x11", "niri", "wayland"];

/// The id of the provider used when nothing else will run.
pub const DUMMY_PROVIDER_ID: &str = "dummy";

/// Pick a provider.
///
/// The first activatable candidate in order, or the dummy. The dummy is a real
/// provider rather than an absence, so every caller has something to call and
/// none of them has to check for null.
#[must_use]
pub fn choose_provider<'a>(candidates: &[(&'a str, bool)]) -> &'a str {
    candidates
        .iter()
        .find(|(_, activatable)| *activatable)
        .map_or(DUMMY_PROVIDER_ID, |(id, _)| *id)
}

/// Whether window management does anything at all.
#[must_use]
pub fn is_capable(provider_id: &str) -> bool {
    provider_id != DUMMY_PROVIDER_ID
}

/// A window, as this layer sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    /// The compositor's own handle.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its `WM_CLASS`.
    pub wm_class: String,
    /// The process that owns it, where the compositor says.
    pub pid: Option<u32>,
    /// Which workspace it is on, where there are workspaces.
    pub workspace: Option<String>,
}

/// Whether a window is one of the launcher's own.
///
/// The pid is checked first and is conclusive; the `WM_CLASS` is only a
/// fallback for compositors that do not report one, and it is compared
/// **case-insensitively** because the class a toolkit sets does not always
/// match the application id in case.
#[must_use]
pub fn is_own_window(window: &Window, own_pid: u32, app_id: &str) -> bool {
    if let Some(pid) = window.pid {
        return pid == own_pid;
    }
    window.wm_class.eq_ignore_ascii_case(app_id)
}

/// Whether a window is on the workspace currently being shown.
///
/// Every unknown answers **yes**, and that is deliberate rather than lazy. The
/// callers use this to decide whether to act on a window, and a compositor
/// that reports partial data — no workspaces, no workspace for this window, no
/// active workspace — would otherwise have every action refused. Saying yes
/// risks acting on a window the user cannot see; saying no guarantees doing
/// nothing at all.
#[must_use]
pub fn is_on_active_workspace(
    window: &Window,
    has_workspaces: bool,
    active_workspace: Option<&str>,
) -> bool {
    if !has_workspaces {
        return true;
    }
    let Some(workspace) = window.workspace.as_deref() else {
        return true;
    };
    if workspace.is_empty() {
        return true;
    }
    let Some(active) = active_workspace else {
        return true;
    };
    active == workspace
}

/// What the launcher considers "the focused window".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusedWindow {
    /// The provider's answer, used as given.
    FromProvider,
    /// The window remembered from before the launcher took focus.
    Remembered,
    /// Nothing.
    None,
}

/// Decide which window an action should act on.
///
/// A compositor that can report the *frontmost* window is trusted outright:
/// it knows what is in front even while the launcher has keyboard focus, so
/// none of the remembering below is needed.
///
/// Otherwise the provider's focused window is used unless it is the launcher's
/// own — which it will be, because the launcher is open. That is the case the
/// memory exists for. And when the provider reports nothing focused, the
/// memory is used only if the launcher itself has focus; if nothing anywhere
/// has focus the answer is nothing, because the memory may be stale.
#[must_use]
pub fn focused_window(
    supports_frontmost: bool,
    provider_focused: Option<&Window>,
    own_pid: u32,
    app_id: &str,
    launcher_has_focus: bool,
) -> FocusedWindow {
    if supports_frontmost {
        return FocusedWindow::FromProvider;
    }
    match provider_focused {
        Some(window) => {
            if is_own_window(window, own_pid, app_id) {
                FocusedWindow::Remembered
            } else {
                FocusedWindow::FromProvider
            }
        }
        None => {
            if launcher_has_focus {
                FocusedWindow::Remembered
            } else {
                FocusedWindow::None
            }
        }
    }
}

/// The focused window, ignoring the launcher's own.
///
/// Unlike [`focused_window`] this never falls back to the memory: it answers
/// what is focused *now*, and the launcher's own window is not an answer.
#[must_use]
pub fn focused_foreign_window(
    provider_focused: Option<&Window>,
    own_pid: u32,
    app_id: &str,
) -> Option<Window> {
    let window = provider_focused?;
    if is_own_window(window, own_pid, app_id) {
        return None;
    }
    Some(window.clone())
}

/// What to do with the remembered window when focus changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMemoryUpdate {
    /// Leave it as it is.
    Keep,
    /// Remember the window the provider reported.
    Remember,
    /// Forget what was remembered.
    Forget,
}

/// Update the focus memory.
///
/// Three things are worth noting. A provider that reports the frontmost window
/// needs no memory, so nothing is recorded at all. The launcher's own window
/// is never remembered but does **not** clear the memory either — the whole
/// point is that opening the launcher should not lose what was underneath it.
/// And nothing focused only clears the memory when the launcher does not have
/// focus, for the same reason.
#[must_use]
pub fn focus_memory_update(
    supports_frontmost: bool,
    provider_focused: Option<&Window>,
    own_pid: u32,
    app_id: &str,
    launcher_has_focus: bool,
) -> FocusMemoryUpdate {
    if supports_frontmost {
        return FocusMemoryUpdate::Keep;
    }
    match provider_focused {
        Some(window) => {
            if is_own_window(window, own_pid, app_id) {
                FocusMemoryUpdate::Keep
            } else {
                FocusMemoryUpdate::Remember
            }
        }
        None => {
            if launcher_has_focus {
                FocusMemoryUpdate::Keep
            } else {
                FocusMemoryUpdate::Forget
            }
        }
    }
}

/// Whether the remembered window is still worth having.
///
/// It is dropped once it is no longer on the active workspace: focusing a
/// window on another workspace would take the user somewhere they did not ask
/// to go, which is worse than doing nothing.
#[must_use]
pub fn remembered_window_survives(
    remembered: &Window,
    has_workspaces: bool,
    active_workspace: Option<&str>,
) -> bool {
    is_on_active_workspace(remembered, has_workspaces, active_workspace)
}

/// Whether the remembered window survives a refresh of the window list.
///
/// A window that has closed is dropped. Checking by **id** rather than by
/// identity is what catches a window the compositor destroyed and replaced.
#[must_use]
pub fn remembered_window_still_open(remembered_id: &str, window_ids: &[String]) -> bool {
    window_ids.iter().any(|id| id == remembered_id)
}

/// The windows belonging to an application.
///
/// Two ways to match, and the second is a fallback for applications whose
/// windows do not carry a usable class: the window's *title* compared
/// case-insensitively against the application's display name. It is loose —
/// a document window titled after its file will not match — but it is what
/// finds a window that would otherwise be unreachable.
#[must_use]
pub fn find_app_windows<'a>(
    windows: &'a [Window],
    display_name: &str,
    matches_class: impl Fn(&str) -> bool,
) -> Vec<&'a Window> {
    windows
        .iter()
        .filter(|window| {
            matches_class(&window.wm_class) || window.title.eq_ignore_ascii_case(display_name)
        })
        .collect()
}

/// Find a window by the compositor's handle.
#[must_use]
pub fn find_window_by_id<'a>(windows: &'a [Window], id: &str) -> Option<&'a Window> {
    windows.iter().find(|window| window.id == id)
}

/// Whether the workspace list needs fetching.
///
/// It is cached until the window list changes, and a provider with no
/// workspaces caches an **empty list** rather than nothing — so the absence is
/// remembered too and the provider is not asked again on every lookup.
#[must_use]
pub const fn workspaces_need_fetch(cached: bool) -> bool {
    !cached
}
