//! Opening a file dialog, and which of the two dialogs opens.
//!
//! A port of `FileChooserService` (`src/server/src/services/file-chooser/`),
//! minus the portal client and the QML dialog.
//!
//! # `false` does not mean failure
//!
//! [`FileChooserService::open_dialog`] returns whether the *portal* handled
//! it. `false` means the caller must put up its own dialog — the C++ comment
//! says so — and a caller that read it as an error would show nothing at all
//! on every desktop without a working portal, which is a large share of them.
//! That is the one rule in this module worth breaking a build over, so it has
//! the first test.

use std::path::PathBuf;

/// What the dialog is being opened for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileChooserOptions {
    /// Whether files may be picked.
    pub can_choose_files: bool,
    /// Whether directories may be picked.
    pub can_choose_directories: bool,
    /// Whether more than one thing may be picked.
    pub allow_multiple_selection: bool,
    /// Whether dotfiles are shown.
    pub show_hidden_files: bool,
    /// Where to start.
    pub current_folder: Option<PathBuf>,
}

impl FileChooserOptions {
    /// The defaults `FileChooserOptions` is declared with: files only, one at
    /// a time, no hidden files.
    #[must_use]
    pub fn new() -> Self {
        Self {
            can_choose_files: true,
            can_choose_directories: false,
            allow_multiple_selection: false,
            show_hidden_files: false,
            current_folder: None,
        }
    }
}

/// Which dialog is up, if any.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Active {
    /// None.
    #[default]
    None,
    /// The portal's, with what it was opened for.
    Portal(FileChooserOptions),
    /// The caller's own.
    Fallback,
}

/// What opening the dialog led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The portal took it; these are the options it was given.
    Portal(FileChooserOptions),
    /// No portal, so the caller must show its own dialog.
    Fallback,
    /// Something was already up; nothing happened.
    AlreadyOpen,
}

impl Opened {
    /// What `openDialog` returns: whether the portal handled it.
    ///
    /// `AlreadyOpen` is `true`, as the C++ early return is — from the caller's
    /// point of view there is a dialog on screen either way, and putting a
    /// second one up over it is the thing to avoid.
    #[must_use]
    pub const fn portal_handled(&self) -> bool {
        !matches!(self, Self::Fallback)
    }
}

/// What ending the dialog produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    /// The chosen paths, or `None` when it was cancelled.
    ///
    /// An empty `Some` is not the same as `None`: the portal can return an
    /// empty list from an accepted dialog, and the C++ still emits
    /// `filesSelected` for it.
    pub selected: Option<Vec<PathBuf>>,
}

/// Turn a `file://` URL from the fallback dialog into a local path.
///
/// A trailing separator is dropped, except on a root — `/` stays `/`, and a
/// Windows drive root `C:/` keeps its slash, because `C:` alone names the
/// process's current directory on that drive rather than the drive itself.
#[must_use]
pub fn to_local_path(path: &str) -> String {
    if path.len() > 1 && path.ends_with('/') && !path.ends_with(":/") {
        return path[..path.len() - 1].to_owned();
    }
    path.to_owned()
}

/// At most one file dialog, and which kind.
#[derive(Debug, Clone, Default)]
pub struct FileChooserService {
    /// What is up.
    active: Active,
}

impl FileChooserService {
    /// A service with nothing open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a dialog of either kind is up.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active != Active::None
    }

    /// Open a dialog for `options`.
    ///
    /// `portal_available` is asked only when nothing is already open, which is
    /// what keeps a second dialog from being created and immediately thrown
    /// away.
    pub fn open_dialog(
        &mut self,
        options: FileChooserOptions,
        portal_available: impl FnOnce() -> bool,
    ) -> Opened {
        if self.is_active() {
            return Opened::AlreadyOpen;
        }

        if !portal_available() {
            self.active = Active::Fallback;
            return Opened::Fallback;
        }

        self.active = Active::Portal(options.clone());
        Opened::Portal(options)
    }

    /// The portal answered with `selected`, or with `None` if it was
    /// cancelled.
    ///
    /// Returns `None` if nothing was open, so a late answer from a dialog
    /// already dismissed does not announce a selection twice.
    pub fn finish(&mut self, selected: Option<Vec<PathBuf>>) -> Option<Closed> {
        if !self.is_active() {
            return None;
        }
        self.active = Active::None;
        Some(Closed { selected })
    }

    /// The caller's own dialog finished, whichever way.
    ///
    /// Distinct from [`Self::finish`] because it never carries a selection:
    /// the fallback dialog reports its own result through its own channel, and
    /// this only says the dialog is gone.
    pub fn notify_fallback_done(&mut self) -> Option<Closed> {
        if self.active != Active::Fallback {
            return None;
        }
        self.active = Active::None;
        Some(Closed { selected: None })
    }

    /// Dismiss whatever is up without a selection.
    pub fn cancel(&mut self) -> Option<Closed> {
        self.finish(None)
    }
}
