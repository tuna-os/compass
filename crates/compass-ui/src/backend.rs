//! Application search and launch-history services supplied by the composition root.

use std::future::Future;
use std::pin::Pin;

/// An asynchronous backend operation, without a socket dependency in the UI.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The shared application catalog's ranking and successful-launch history.
pub trait ApplicationBackend: std::fmt::Debug + Send + Sync {
    /// Return root-item ENTRYPOINT ids in presentation order.
    ///
    /// `applications:org.mozilla.firefox`, not the launch key
    /// `org.mozilla.firefox.desktop` — this is `QueryHit.id` straight off the
    /// wire, and `AppIndex::position_by_entrypoint` is what resolves it.
    /// `record_launch` below still takes the KEY, because the two ids are
    /// different things and the frecency store is keyed by the launchable.
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>>;

    /// Record an already successful launch; never execute the application again.
    fn record_launch(&self, key: String) -> BackendFuture<'_, ()>;

    /// Run an installed extension's command by its entrypoint id. `Ok` once
    /// the engine has started it; an error is a sentence saying why it could
    /// not, for the launcher to show.
    fn run_extension_command(&self, id: String) -> BackendFuture<'_, ()> {
        let _ = id;
        Box::pin(async { Err("Running extension commands needs the Compass engine".to_owned()) })
    }
}

/// One clipboard history row, as the UI draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardRow {
    /// Stable id, used to fetch the content.
    pub id: String,
    /// What the row says: the start of copied text, or a label.
    pub preview: String,
    /// What kind of thing was copied.
    pub kind: ClipboardRowKind,
    /// Whether it is pinned to the top.
    pub pinned: bool,
    /// For links, the host.
    pub url_host: Option<String>,
}

/// What kind of thing a [`ClipboardRow`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRowKind {
    /// Plain text.
    Text,
    /// A URL.
    Link,
    /// An image.
    Image,
    /// Files.
    File,
    /// Anything else.
    Unknown,
}

/// One entry's full content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardContent {
    /// MIME type of `data`.
    pub mime_type: String,
    /// The content as copied.
    pub data: Vec<u8>,
}

/// Clipboard history, which only the engine holds.
///
/// Separate from [`ApplicationBackend`] because a window can search
/// applications on its own and cannot read clipboard history on its own: the
/// store is the engine's, behind its keyring.
pub trait ClipboardBackend: std::fmt::Debug + Send + Sync {
    /// Entries matching `query`, pinned first then newest; empty lists all.
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>>;

    /// One entry's full content, for copying it back.
    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent>;

    /// Put one entry on the clipboard and paste it into the window focus
    /// moves to next. Ask while the launcher is focused, then hide it. A
    /// refusal (no GNOME Shell extension) means the caller copies instead.
    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()>;

    /// Pin or unpin one entry.
    fn clipboard_set_pinned(&self, id: String, pinned: bool) -> BackendFuture<'_, ()>;

    /// Remove one entry and its stored content.
    fn clipboard_remove(&self, id: String) -> BackendFuture<'_, ()>;
}

/// One open window, as the switcher draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    /// Handle for activate and close.
    pub id: u32,
    /// The window's title.
    pub title: String,
    /// The application's name when recognised, else its `WM_CLASS`.
    pub app: String,
    /// Its `WM_CLASS`, searched at a low weight.
    pub wm_class: String,
    /// The owning process, so the launcher can leave out its own window.
    pub pid: Option<u32>,
    /// Whether it can be closed.
    pub can_close: bool,
}

/// Window switching, which only the engine can do (through the Shell
/// extension).
pub trait WindowBackend: std::fmt::Debug + Send + Sync {
    /// The open windows, the one worth switching to first.
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>>;

    /// Focus and raise a window.
    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()>;

    /// Ask a window to close.
    fn close_window(&self, id: u32) -> BackendFuture<'_, ()>;
}
