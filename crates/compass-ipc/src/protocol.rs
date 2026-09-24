//! Versioned request/response types carried over the socket.
//!
//! Every frame on the wire is one [`RequestEnvelope`] (client to server) or one
//! [`ResponseEnvelope`] (server to client). Both carry the [`PROTOCOL_VERSION`]
//! they were produced with, so an old `vicinae` CLI meeting a new engine — or
//! the reverse — gets a clear [`ErrorKind::VersionMismatch`] instead of a
//! postcard decode failure or, worse, a silent misinterpretation.
//!
//! Postcard is **not** self-describing: field order and variant order *are* the
//! schema. Adding a variant in the middle of [`Request`] or [`Response`], or
//! reordering fields, is a breaking change. Append new variants at the end and
//! bump [`PROTOCOL_VERSION`] whenever the meaning of existing bytes changes.

use serde::{Deserialize, Serialize};

/// Version of the request/response schema understood by this build.
///
/// Bumped whenever the postcard encoding of [`Request`] or [`Response`]
/// changes in a way that an older peer would misread.
///
/// # Why v2 bumped for appended variants
///
/// Appending a variant does not change the meaning of any byte a v1 peer can
/// produce, so by the rule above it looks like it should not need a bump. It
/// does, because of the direction the new variants travel: a v2 window sends
/// [`Request::AttachWindow`], and a **v1 engine** decoding it would not get a
/// clean refusal — the variant index is past the end of its `Request` enum, so
/// it gets a postcard decode error and drops the connection with no
/// explanation. The version field exists precisely to turn that into a sentence
/// a human can act on, and it only does so if the number moves.
///
/// Version 3 adds successful-launch reporting to the daemon-owned history.
/// Version 4 adds clipboard history; version 5, fetching an entry's content;
/// version 6, window switching; version 7, pasting, pinning and removing a
/// clipboard entry.
pub const PROTOCOL_VERSION: u16 = 7;

/// A client-to-server frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id, echoed in the matching [`ResponseEnvelope`].
    ///
    /// The current transport is strictly request/response per connection, but
    /// the id is on the wire from day one so pipelining does not need a
    /// protocol bump.
    pub id: u64,
    /// The request itself.
    pub request: Request,
}

impl RequestEnvelope {
    /// Wraps `request` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            request,
        }
    }
}

/// A server-to-client frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id copied from the request this answers.
    ///
    /// Zero when the server could not decode a request far enough to know it.
    pub id: u64,
    /// The response itself.
    pub response: Response,
}

impl ResponseEnvelope {
    /// Wraps `response` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, response: Response) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            response,
        }
    }
}

/// What a client can ask the engine to do.
///
/// Deliberately small: this is the Phase 2 surface (socket, CLI, doctor) and
/// nothing more. New variants go at the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Liveness probe. Answered with [`Response::Pong`].
    Ping,
    /// Toggle the launcher window between shown and hidden.
    Toggle,
    /// Show the launcher window.
    Show,
    /// Hide the launcher window.
    Hide,
    /// Run a root search and return the ranked hits.
    Query {
        /// Raw search text, exactly as typed.
        text: String,
    },
    /// Run the self-diagnostic checks behind `vicinae doctor`.
    Doctor,
    /// Ask the engine to shut down cleanly.
    Shutdown,
    /// Offer this connection as *the* launcher window.
    ///
    /// Answered with [`Response::WindowAttached`], after which the connection
    /// reverses: the engine pushes [`Response::Window`] frames and the window
    /// answers each with [`Request::WindowOutcome`]. See
    /// [`crate::transport::WindowLink`].
    AttachWindow,
    /// A window's answer to one pushed [`WindowCommand`].
    ///
    /// Only legal on an attached connection, where it is a *reply* rather than
    /// a request; the engine never sends a [`Response`] back to it.
    WindowOutcome(WindowOutcome),
    /// Record a successfully completed launch of an indexed item.
    ///
    /// Reports an outcome; it does not launch an application. The daemon
    /// validates the key and updates its history before acknowledging it.
    RecordLaunch {
        /// Stable application/action key, as returned by the index.
        key: String,
    },
    /// Search clipboard history, newest first with pinned entries on top.
    ///
    /// Answered with [`Response::ClipboardHistory`], or with an
    /// [`ErrorKind::Unsupported`] error while the engine has no store — no
    /// keyring, or the store would not open. An empty `query` lists.
    ClipboardHistory {
        /// Search text; empty for the whole history.
        query: String,
        /// Most entries to return. Zero is a bad request.
        limit: u32,
    },
    /// The full content of one clipboard history entry, decrypted.
    ///
    /// Answered with [`Response::ClipboardContent`]; an id that names no entry
    /// is a bad request. Separate from the list because a list row needs only
    /// the preview, and content can be a whole image.
    ClipboardContent {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// The open windows, for the window switcher.
    ///
    /// Answered with [`Response::Windows`], or refused as
    /// [`ErrorKind::Unsupported`] without the GNOME Shell extension, which is
    /// the only way to list windows on GNOME.
    ListWindows,
    /// Focus and raise one window. Answered with [`Response::Ack`].
    ActivateWindow {
        /// [`WindowInfo::id`].
        id: u32,
    },
    /// Ask one window to close. Answered with [`Response::Ack`].
    CloseWindow {
        /// [`WindowInfo::id`].
        id: u32,
    },
    /// Put one clipboard history entry on the clipboard and paste it into the
    /// window focus moves to next.
    ///
    /// Send it while the launcher still has focus and hide the launcher once
    /// it is answered with [`Response::Ack`]: the paste lands after the focus
    /// change. Refused as [`ErrorKind::Unsupported`] without the GNOME Shell
    /// extension, which is the only thing on GNOME that can press a key in
    /// another window; the caller then copies instead.
    ClipboardPaste {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// Pin or unpin one clipboard history entry. Pinned entries list first and
    /// survive eviction. Answered with [`Response::Ack`]; an id that names no
    /// entry is a bad request.
    ClipboardSetPinned {
        /// [`ClipboardEntry::id`].
        id: String,
        /// Pin when true, unpin when false.
        pinned: bool,
    },
    /// Remove one clipboard history entry and its stored content. Answered
    /// with [`Response::Ack`]; an id that names no entry is a bad request.
    ClipboardRemove {
        /// [`ClipboardEntry::id`].
        id: String,
    },
}

/// What the engine answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// Answer to [`Request::Ping`].
    Pong {
        /// Protocol version the server speaks.
        protocol_version: u16,
        /// Process id of the engine, for `doctor` and stale-socket reporting.
        pid: u32,
    },
    /// The requested side effect was performed; there is nothing to report.
    Ack,
    /// Ranked results for a [`Request::Query`].
    QueryResults {
        /// Hits in presentation order: best first.
        hits: Vec<QueryHit>,
    },
    /// Diagnostic report for a [`Request::Doctor`].
    DoctorReport {
        /// One entry per check, in the order they were run.
        checks: Vec<DoctorCheck>,
    },
    /// The engine accepted [`Request::Shutdown`] and is on its way down.
    ShuttingDown,
    /// The request could not be served.
    Error(ProtocolError),
    /// [`Request::AttachWindow`] was accepted; this connection is now the
    /// launcher window's push channel.
    WindowAttached,
    /// A command pushed from the engine to an attached window.
    ///
    /// This is the one frame the engine sends unsolicited. Its envelope id is
    /// allocated by the engine and echoed by the window in the matching
    /// [`Request::WindowOutcome`].
    Window(WindowCommand),
    /// Entries for a [`Request::ClipboardHistory`], in presentation order.
    ClipboardHistory {
        /// Matching entries: pinned first, then most recently copied.
        entries: Vec<ClipboardEntry>,
    },
    /// Content for a [`Request::ClipboardContent`].
    ClipboardContent {
        /// MIME type of the bytes.
        mime_type: String,
        /// The content, exactly as it was copied.
        data: Vec<u8>,
    },
    /// Answer to [`Request::ListWindows`], most recently used first.
    Windows {
        /// Every window the extension reports.
        windows: Vec<WindowInfo>,
    },
}

/// What the engine asks an attached window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowCommand {
    /// Become visible and take focus.
    Show,
    /// Become hidden.
    Hide,
    /// Hide if visible, show if not.
    Toggle,
}

/// What an attached window reports back after acting on a [`WindowCommand`].
///
/// [`Self::Shown`] and [`Self::Hidden`] report the state the window ended in,
/// not the command it was given — which is the only useful answer to
/// [`WindowCommand::Toggle`], and lets a caller of `show` on an
/// already-visible window learn that nothing changed without a second round
/// trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowOutcome {
    /// The window is now visible.
    Shown,
    /// The window is now hidden.
    Hidden,
    /// The window could not carry the command out, with a reason to print.
    Failed(String),
}

/// One ranked search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryHit {
    /// Stable identifier of the underlying root item.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Secondary display text, when the item has one.
    pub subtitle: Option<String>,
    /// Match score in `0..=100`, matching `compass-search`'s scale.
    pub score: u32,
}

/// One clipboard history entry, as a list row needs it.
///
/// The payload itself is not sent: rows show [`preview`](Self::preview), and
/// copying an entry back is a separate request once there is one to make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardEntry {
    /// Stable id of the entry.
    pub id: String,
    /// Text shown in the row: the start of copied text, or a label such as
    /// "Image" for content that has none.
    pub preview: String,
    /// MIME type of the preferred representation.
    pub mime_type: String,
    /// What kind of thing was copied.
    pub kind: ClipboardKind,
    /// Whether the entry is pinned to the top.
    pub pinned: bool,
    /// When it was last copied, in milliseconds since the Unix epoch.
    pub updated_at: i64,
    /// For links, the host, so a row can say where it points.
    pub url_host: Option<String>,
}

/// One open window, as the switcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Handle for [`Request::ActivateWindow`] and [`Request::CloseWindow`].
    pub id: u32,
    /// The window's title.
    pub title: String,
    /// Its `WM_CLASS`.
    pub wm_class: String,
    /// The application it belongs to, when the engine recognised one.
    pub app_name: Option<String>,
    /// That application's icon name.
    pub app_icon: Option<String>,
    /// The owning process, so a client can leave out its own windows.
    pub pid: Option<u32>,
    /// Workspace index, when known.
    pub workspace: Option<i32>,
    /// Whether it has focus right now.
    pub focused: bool,
    /// Whether it can be closed.
    pub can_close: bool,
}

/// What kind of thing a [`ClipboardEntry`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardKind {
    /// Plain text.
    Text,
    /// A URL.
    Link,
    /// An image.
    Image,
    /// One or more files.
    File,
    /// Anything the store kept but could not classify.
    Unknown,
}

/// One diagnostic check performed by `vicinae doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Short machine-ish name, e.g. `"portal.global-shortcuts"`.
    pub name: String,
    /// Outcome of the check.
    pub status: DoctorStatus,
    /// Human-readable detail, when there is something to say.
    pub detail: Option<String>,
}

/// Outcome of a single [`DoctorCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorStatus {
    /// Working as intended.
    Ok,
    /// Degraded but usable.
    Warn,
    /// Broken.
    Fail,
}

/// A failure reported by the server in [`Response::Error`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Machine-readable category.
    pub kind: ErrorKind,
    /// Human-readable explanation, safe to print to a terminal.
    pub message: String,
}

impl ProtocolError {
    /// Builds an error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

/// Category of a [`ProtocolError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Peer speaks a protocol version this build cannot serve.
    VersionMismatch,
    /// The request was understood but this build does not implement it.
    Unsupported,
    /// The request was malformed.
    BadRequest,
    /// The handler failed.
    Internal,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::VersionMismatch => "version mismatch",
            Self::Unsupported => "unsupported",
            Self::BadRequest => "bad request",
            Self::Internal => "internal error",
        };
        f.write_str(s)
    }
}

/// Builds the [`Response::Error`] the server sends when a peer's envelope
/// carries a version this build does not speak.
#[must_use]
pub fn version_mismatch(peer: u16) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::VersionMismatch,
        format!(
            "peer speaks compass-ipc protocol v{peer}, this build speaks v{PROTOCOL_VERSION}; \
             restart the client and the engine from the same build"
        ),
    ))
}
