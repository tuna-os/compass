//! What "the selected text" means, and what it means when there isn't any.
//!
//! A port of `AbstractSelectionService` and its Linux and dummy backends
//! (`src/server/src/services/selection/`).
//!
//! # Two sources, because one of them is often unavailable
//!
//! On Wayland the PRIMARY selection can only be read by a client the
//! compositor has granted `wlr-data-control` to. Where that works, the
//! clipboard service pushes every change and this holds the latest. Where it
//! does not, the C++ falls back to asking Qt — which the comment calls "best
//! effort ... not accurate", because Qt can only see a selection made in a
//! window that is not ours, and only sometimes.
//!
//! The fallback order is the whole content of this module, and it matters:
//! reading Qt first would mean a stale answer whenever the accurate source
//! *does* work.

/// What the Linux backend says when neither source has anything.
pub const NO_SELECTION_ERROR: &str = "Unable to get selected text";

/// What the fallback backend says on a platform with no implementation.
///
/// Distinct from [`NO_SELECTION_ERROR`] on purpose: an extension author
/// reading "unable to get selected text" would look for a selection, and
/// reading this one knows to stop.
pub const UNSUPPORTED_ERROR: &str = "Selected text is not supported on this platform";

/// A selection read, or the message explaining why not.
pub type SelectionResult = Result<String, String>;

/// Reads the PRIMARY selection, preferring the accurate source.
#[derive(Debug, Clone, Default)]
pub struct LinuxSelectionService {
    /// The latest PRIMARY selection the clipboard service pushed.
    primary_text: String,
}

impl LinuxSelectionService {
    /// A service that has not been told anything yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a PRIMARY selection change, as `primarySelectionChanged` does.
    ///
    /// An empty update is stored as emptiness rather than ignored: a selection
    /// that was cleared is not still there, and keeping the old text would
    /// hand an extension something the person deselected.
    pub fn set_primary_text(&mut self, text: impl Into<String>) {
        self.primary_text = text.into();
    }

    /// The selected text, from the accurate source or the fallback.
    ///
    /// `fallback` is Qt's `QClipboard::Selection`, asked only when the
    /// accurate source is empty.
    ///
    /// # Errors
    ///
    /// [`NO_SELECTION_ERROR`] when neither source has anything.
    pub fn selected_text(&self, fallback: impl FnOnce() -> String) -> SelectionResult {
        if !self.primary_text.is_empty() {
            return Ok(self.primary_text.clone());
        }
        let text = fallback();
        if text.is_empty() {
            return Err(NO_SELECTION_ERROR.to_owned());
        }
        Ok(text)
    }
}

/// The backend on a platform with no way to read a selection.
#[derive(Debug, Clone, Copy, Default)]
pub struct DummySelectionService;

impl DummySelectionService {
    /// Always [`UNSUPPORTED_ERROR`].
    ///
    /// # Errors
    ///
    /// Always.
    pub fn selected_text(&self) -> SelectionResult {
        Err(UNSUPPORTED_ERROR.to_owned())
    }
}
