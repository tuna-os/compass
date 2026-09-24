//! Messages for the launcher application.

use iced::Event;

use crate::resident::UiCommand;

/// Which way the selection moves.
///
/// An enum rather than a signed delta: the only two motions a launcher list has
/// are next and previous, and a number invites callers to invent a third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the top of the list.
    Up,
    /// Towards the bottom of the list.
    Down,
}

/// Messages that the launcher application can handle.
#[derive(Debug, Clone)]
pub enum Message {
    /// Initialize the application (connect portals, bind shortcuts).
    Initialize,
    /// The window is about to draw a frame.
    ///
    /// Only the first one is acted on, to time cold start (#13 §8.5). Iced
    /// yields this on `RedrawRequested`, which is the moment the compositor
    /// asks for a frame rather than the moment one reaches the screen -- see
    /// [`crate::app::LauncherApp`]'s handling for why that distinction is
    /// recorded rather than glossed.
    FrameDrawn,
    /// The search query changed.
    QueryChanged(String),
    /// An asynchronous search completed. Only the current generation may apply.
    SearchCompleted {
        /// Generation captured when the request started.
        generation: u64,
        /// Stable application keys, in backend ranking order, or a failure.
        result: Result<Vec<String>, String>,
    },
    /// A result was selected (by keyboard navigation).
    ResultSelected(usize),
    /// Move the selection one row, wrapping at both ends.
    MoveSelection(Direction),
    /// Launch the selected result.
    LaunchSelected,
    /// The clipboard history filter changed.
    ClipboardQueryChanged(String),
    /// Clipboard history rows arrived for request `generation`.
    ClipboardLoaded {
        /// The request they answer; a stale one is dropped.
        generation: u64,
        /// The rows, or why there are none.
        result: Result<Vec<crate::backend::ClipboardRow>, String>,
    },
    /// The selected clipboard entry's content arrived, to be copied.
    ClipboardContentLoaded(Result<crate::backend::ClipboardContent, String>),
    /// The engine armed a paste of the selected entry, or could not; on a
    /// refusal the entry is copied instead.
    ClipboardPasted(Result<(), String>),
    /// The engine started an extension command, or said why it could not.
    ExtensionCommandStarted(Result<(), String>),
    /// An entry was pinned, unpinned or removed, or could not be; the list
    /// reloads on success and says why on failure.
    ClipboardEntryChanged(Result<(), String>),
    /// A clipboard row was clicked.
    ClipboardSelected(usize),
    /// Leave a command's view for the root list.
    Back,
    /// The window switcher's filter changed.
    WindowsQueryChanged(String),
    /// The open windows arrived, or why they could not be listed.
    WindowsLoaded(Result<Vec<crate::backend::WindowRow>, String>),
    /// A window row was clicked.
    WindowSelected(usize),
    /// Switching to a window finished.
    WindowActivated(Result<(), String>),
    /// Closing a window finished; the list is reloaded either way.
    ShellWindowClosed(Result<(), String>),
    /// A launch finished, successfully or not.
    ///
    /// Carried as a string rather than the error type because a `Message` must
    /// be `Clone` and `LaunchError` is not — and because the only thing the UI
    /// does with a failure is show it.
    Launched(Result<(), String>),
    /// Dismiss the launcher without launching anything.
    Dismiss,
    /// Open or close the action panel over the results.
    TogglePanel,
    /// The action panel's filter changed.
    PanelFilterChanged(String),
    /// Move the action panel's selection one row.
    PanelMove(Direction),
    /// Run the action panel's selected action.
    PanelActivate,
    /// Activate the clicked action row; headings and dividers are ignored.
    PanelClicked(usize),
    /// A global shortcut was activated.
    ShortcutActivated(String),
    /// Window focus changed.
    FocusChanged(bool),
    /// Window was closed.
    WindowClosed,
    /// Poll for shortcut events.
    PollShortcuts,
    /// Raw Iced event (for advanced handling).
    EventOccurred(Event),
    /// The engine asked the window to show, hide or toggle.
    Command(UiCommand),
    /// The engine closed its command channel.
    EngineDisconnected,
    /// The desktop's light/dark preference changed.
    AppearanceChanged(crate::design::Appearance),
    /// The desktop's interface font family changed.
    TypographyChanged(String),
    /// Preview a theme without persisting it (#153 live preview).
    ThemePreview(crate::theme::Theme),
    /// Commit the previewed theme to config.
    ThemeCommit,
    /// Cancel preview and restore the theme from config.
    ThemeCancel,
    /// Close a window from the switcher (`ctrl+q`).
    ///
    /// The id is the plain `u32` the shell minted (see
    /// `compass_core::window_switcher::window_launch_target_for_app`), not the
    /// shell's `WindowId` type: this shared crate must never name it.
    CloseWindow(u32),
    /// A window finished opening, and this is its id.
    ///
    /// Carried separately from `Command(Show)` because the honest moment to
    /// report `Shown` is when the window exists, not when opening it was
    /// requested.
    Opened(iced::window::Id),
    /// A window was closed by the compositor or the user.
    ///
    /// Distinct from [`Message::Dismiss`]: this is the window telling us it is
    /// gone, not a request to make it go.
    Closed(iced::window::Id),
    /// Leave for good.
    ///
    /// The one thing that still ends the process, now that dismissing only
    /// hides. See ADR-0015.
    Quit,
    /// A keyboard event that no widget consumed.
    ///
    /// Carries the whole event rather than a pre-digested action because the
    /// text input takes the printable keys first; what reaches here is exactly
    /// the set the launcher itself has to interpret.
    Keyboard(iced::keyboard::Event),
}
