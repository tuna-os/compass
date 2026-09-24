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
}
