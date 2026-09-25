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
    ExtensionCommandStarted {
        /// The command's entrypoint id.
        id: String,
        /// The command's title, for its view until the view names itself.
        title: String,
        /// How it began, or why it could not.
        result: Result<crate::backend::ExtensionStart, String>,
    },
    /// An extension view's latest state, for the session it asked about.
    ExtensionViewLoaded {
        /// Which session.
        session: u64,
        /// Its state, or why it could not be read.
        result: Result<crate::backend::ExtensionViewState, String>,
    },
    /// The search text in an extension's view changed.
    ExtensionQueryChanged(String),
    /// A row in an extension's list was clicked.
    ExtensionItemSelected(usize),
    /// A preference field in the form changed.
    PreferenceEdited(usize, crate::preferences_page::FieldValue),
    /// The preference form was submitted (Enter).
    PreferencesSubmit,
    /// The preferences were kept, or not; on success the command runs.
    PreferencesSaved(Result<(), String>),
    /// A link in an extension's Markdown was clicked.
    ExtensionLinkClicked(String),
    /// The person changed a field of an extension's form: its name and value.
    ExtensionFieldEdited(String, serde_json::Value),
    /// An edit in a form's text area: its name and the editor action.
    ExtensionTextAreaEdited(String, iced::widget::text_editor::Action),
    /// An action or search event reached the extension, or did not.
    ExtensionEventSent(Result<(), String>),
    /// An entry was pinned, unpinned or removed, or could not be; the list
    /// reloads on success and says why on failure.
    ClipboardEntryChanged(Result<(), String>),
    /// A clipboard row was clicked.
    ClipboardSelected(usize),
    /// Leave a command's view for the root list.
    Back,
    /// The window switcher's filter changed.
    WindowsQueryChanged(String),
    /// The engine answered a power or media command.
    BuiltinCommandDone(Result<(), String>),
    /// The emoji picker's filter changed.
    EmojiQueryChanged(String),
    /// An emoji row was clicked, by position in the shown list.
    EmojiSelected(usize),
    /// Search Files' text changed.
    FilesQueryChanged(String),
    /// The debounce for Search Files query `generation` ran out; ask, unless
    /// the text moved on meanwhile.
    FilesDebounced(u64),
    /// Search Files' answer to query `generation` arrived.
    FilesLoaded {
        /// The query it answers; a stale one is dropped.
        generation: u64,
        /// The heading and files, or why there are none.
        result: Result<crate::backend::FileResults, String>,
    },
    /// A file row was clicked, by position.
    FilesSelected(usize),
    /// Opening a file (or showing it in the file browser) finished.
    FileOpened(Result<(), String>),
    /// An edit in a form's text area, by field position.
    PreferenceTextEdited(usize, iced::widget::text_editor::Action),
    /// The snippet list arrived (on opening Manage Snippets, or after a
    /// change), or why it could not be read.
    SnippetsLoaded(Result<Vec<crate::backend::Snippet>, String>),
    /// A snippet was saved (the list after it), or why it was not.
    SnippetSaved(Result<Vec<crate::backend::Snippet>, String>),
    /// A snippet's expansion, to copy, or why it could not be expanded.
    SnippetExpanded(Result<String, String>),
    /// Pasting a snippet finished.
    SnippetPasted(Result<(), String>),
    /// Manage Snippets' filter changed.
    SnippetsQueryChanged(String),
    /// A Manage Snippets row was clicked, by position.
    SnippetSelected(usize),
    /// Create Extension finished, for the extension called `title`: where
    /// it was written, or why not.
    ExtensionCreated {
        /// The extension's title.
        title: String,
        /// Its path, or the reason.
        result: Result<String, String>,
    },
    /// Opening the new extension's folder finished.
    CreatedFolderOpened(Result<(), String>),
    /// Browse Fonts' families arrived, or why they could not be listed.
    FontsLoaded(Result<crate::backend::FontList, String>),
    /// Browse Fonts' search text changed.
    FontsQueryChanged(String),
    /// Browse Fonts' category filter changed, to the option titled so.
    FontsCategoryChanged(String),
    /// A Browse Fonts row was clicked, by position.
    FontSelected(usize),
    /// A family's specimen arrived, or why not.
    FontSpecimenLoaded {
        /// The typeface's name.
        name: String,
        /// The member to draw it with.
        family: String,
        /// The specimen's Markdown.
        result: Result<String, String>,
    },
    /// Set Theme's search text changed.
    ThemesQueryChanged(String),
    /// A Set Theme row was clicked, by position.
    ThemeSelected(usize),
    /// The chosen theme was kept, or why not.
    ThemeSaved(Result<(), String>),
    /// A dmenu list arrived for `token`, or why it could not be fetched.
    DmenuLoaded {
        /// Which list.
        token: u64,
        /// The list.
        result: Result<crate::backend::DmenuList, String>,
    },
    /// The dmenu view's search text changed.
    DmenuQueryChanged(String),
    /// A dmenu entry was clicked, by position.
    DmenuSelected(usize),
    /// The dmenu choice reached the engine, or why it did not.
    DmenuChosen(Result<(), String>),
    /// Run Terminal Program's programs arrived, or why they could not.
    ProgramsLoaded(Result<crate::backend::ProgramList, String>),
    /// Run Terminal Program's text changed.
    ProgramsQueryChanged(String),
    /// A Run Terminal Program row was clicked, by position.
    ProgramSelected(usize),
    /// Running a program finished starting, or why it did not.
    ProgramRan(Result<(), String>),
    /// The script commands arrived, or why they could not be listed.
    ScriptsLoaded(Result<Vec<compass_core::script_scan::ScriptItem>, String>),
    /// A script started: the run to follow, if any.
    ScriptStarted {
        /// Which script.
        id: String,
        /// The arguments it ran with.
        arguments: Vec<String>,
        /// The run, or why it did not start.
        result: Result<Option<u64>, String>,
    },
    /// A report on script run `session` arrived.
    ScriptPolled {
        /// Which run.
        session: u64,
        /// What it printed so far, or why it could not be read.
        result: Result<crate::backend::ScriptOutputState, String>,
    },
    /// The shortcut list arrived, or why it could not be read.
    ShortcutsLoaded(Result<Vec<crate::backend::Shortcut>, String>),
    /// A shortcut was saved (the list after it), or why it was not.
    ShortcutSaved(Result<Vec<crate::backend::Shortcut>, String>),
    /// A shortcut was removed (the list after it), or why it was not.
    ShortcutRemoved(Result<Vec<crate::backend::Shortcut>, String>),
    /// Opening a shortcut finished.
    ShortcutOpened(Result<(), String>),
    /// A shortcut's expanded link, to copy, or why it could not be expanded.
    ShortcutExpanded(Result<String, String>),
    /// Manage Shortcuts' filter changed.
    ShortcutsQueryChanged(String),
    /// A Manage Shortcuts row was clicked, by position.
    ShortcutSelected(usize),
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
    /// A request for `iced_layershell`'s runtime, which takes it before
    /// `update` would see it. See [`crate::surface`].
    Layer(crate::surface::LayerRequest),
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
    /// A remote image an extension's view shows was fetched into the cache,
    /// or could not be.
    ExtensionImageFetched {
        /// The image's URL.
        url: String,
        /// Where it is kept, or why it is not.
        result: Result<std::path::PathBuf, String>,
    },
    /// The text in an extension form's date field changed: its name and
    /// the text as typed.
    ExtensionDateEdited(String, String),
    /// A file picker's button: ask the desktop's file chooser.
    ExtensionChooseFiles {
        /// The field's name.
        name: String,
        /// What it may choose.
        choice: crate::extension_fields::FileChoice,
    },
    /// The file chooser answered: the paths chosen (empty when cancelled),
    /// or why it could not be shown.
    ExtensionFilesChosen {
        /// The field's name.
        name: String,
        /// The chosen paths.
        result: Result<Vec<String>, String>,
    },
}
