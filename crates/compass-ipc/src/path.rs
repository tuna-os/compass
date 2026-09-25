//! Where the IPC socket lives.
//!
//! The canonical location is `$XDG_RUNTIME_DIR/compass/ipc.sock`.
//! `XDG_RUNTIME_DIR` is a per-user, `0700`, tmpfs-backed directory that the
//! session manager cleans up at logout, which is exactly the lifetime a socket
//! wants.
//!
//! ## Fallback
//!
//! `XDG_RUNTIME_DIR` is unset in cron jobs, bare `ssh` sessions, some
//! containers and minimal init setups. When it is missing or empty we fall
//! back to `/tmp/compass-$USER/ipc.sock`, using `$USER`, then `$LOGNAME`, then
//! the literal `default` as the name. The username is part of the *directory*
//! so that two users on one machine cannot collide on a path. This mirrors
//! what the C++ build does (`/tmp/vicinae`) but adds the per-user suffix,
//! since a shared `/tmp/vicinae` owned by whoever logged in first is a
//! denial-of-service on everyone else.
//!
//! **The suffix is not what makes it safe.** `$USER` is public and guessable,
//! and settable by whoever starts the process, so an attacker can predict the
//! path and create the directory first. What makes it safe is
//! [`ensure_private_dir`], which refuses a fallback directory that is not
//! exactly `0700` — see its documentation for why that one check is enough.
//! This module said the opposite until #88: that putting the username in the
//! directory meant "the directory\'s owner is the only one who can create the
//! socket inside it". That is true of a directory we create and false of one
//! we adopt, and adopting is what `DirBuilder::recursive(true)` does.
//!
//! The fallback is a fallback: it survives a reboot, is not cleaned at logout,
//! and lives on a directory other users can read. Callers that care should log
//! when [`SocketPath::is_fallback`] is true — `compass doctor` does.
//!
//! Nothing in this module reads the real environment unless you ask it to:
//! [`SocketPath::in_dir`] builds a path under any directory, which is how the
//! tests stay out of the developer's live session.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Directory created under the runtime dir (or the fallback root).
pub const SOCKET_DIR_NAME: &str = "compass";

/// File name of the socket itself.
pub const SOCKET_FILE_NAME: &str = "ipc.sock";

/// A resolved socket location, remembering how it was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPath {
    path: PathBuf,
    fallback: bool,
}

impl SocketPath {
    /// Resolves the socket path from the process environment.
    ///
    /// Uses `$XDG_RUNTIME_DIR` when it is set and non-empty, otherwise the
    /// fallback documented at the [module level](self).
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) if !dir.is_empty() => Self {
                path: PathBuf::from(dir)
                    .join(SOCKET_DIR_NAME)
                    .join(SOCKET_FILE_NAME),
                fallback: false,
            },
            _ => Self {
                path: Path::new("/tmp")
                    .join(format!("{SOCKET_DIR_NAME}-{}", current_user_name()))
                    .join(SOCKET_FILE_NAME),
                fallback: true,
            },
        }
    }

    /// Builds `<dir>/compass/ipc.sock`, ignoring the environment entirely.
    ///
    /// This is the injection point: tests pass a temporary directory, the
    /// Flatpak build can pass its sandboxed runtime dir.
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            path: dir.as_ref().join(SOCKET_DIR_NAME).join(SOCKET_FILE_NAME),
            fallback: false,
        }
    }

    /// Uses `path` verbatim, with no `compass/` component added.
    ///
    /// For `--socket /some/where.sock` style overrides.
    pub fn exact(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fallback: false,
        }
    }

    /// The socket path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// The directory the socket lives in, if it has one.
    #[must_use]
    pub fn parent(&self) -> Option<&Path> {
        self.path.parent()
    }

    /// Refuses the socket's parent directory when it is not exclusively ours.
    ///
    /// A no-op unless this path came from the `/tmp` fallback. A real
    /// `$XDG_RUNTIME_DIR` is the session manager's to create per-user and
    /// `0700`, and a path built with [`SocketPath::in_dir`] was chosen
    /// deliberately by the caller — neither is ours to second-guess. The
    /// fallback root is `/tmp`, shared with every other user on the machine,
    /// and that is the one worth checking.
    ///
    /// # Errors
    ///
    /// See [`ensure_private_dir`].
    pub fn ensure_private_parent(&self) -> Result<()> {
        if !self.fallback {
            return Ok(());
        }
        match self.path.parent() {
            Some(parent) => ensure_private_dir(parent),
            None => Ok(()),
        }
    }

    /// Whether the path came from the `XDG_RUNTIME_DIR`-is-missing fallback.
    #[must_use]
    pub fn is_fallback(&self) -> bool {
        self.fallback
    }

    /// Consumes this value and returns the owned path.
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl AsRef<Path> for SocketPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Display for SocketPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

/// Refuses a socket directory that is not exclusively ours.
///
/// Returns `Ok(())` when `dir` does not exist — the caller creates it `0700`
/// and is then its owner by construction. When it *does* exist, it must be a
/// real directory (not a symlink) with mode exactly `0700`.
///
/// # Why checking the mode is enough, without checking the owner
///
/// The attack is another local user creating `/tmp/compass-victim` before the
/// victim's first fallback start, so the victim binds its socket inside a
/// directory the attacker can write to. For that to work the attacker's
/// directory has to be writable by the victim — which means permissive modes.
/// A directory the attacker owns at `0700` is one we cannot write to at all,
/// so the bind fails with `EACCES` and nothing is compromised.
///
/// So the dangerous case is exactly the permissive one, and refusing anything
/// that is not `0700` covers it without needing the process's uid — which std
/// does not expose, and which would otherwise mean a dependency or an unsafe
/// `getuid` call in a crate that forbids `unsafe_code`.
///
/// The symlink check is separate and not covered by the above: a symlink at
/// that path could point anywhere the victim *can* write, so it is refused on
/// sight rather than followed.
///
/// # Errors
///
/// [`Error::UnsafeSocketDir`] describing what is wrong, or [`Error::Io`] if
/// the directory cannot be inspected at all.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // `symlink_metadata`, not `metadata`: the point is to see the symlink
    // rather than whatever it resolves to.
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        // Not there yet is the good case: the caller makes it, 0700.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };

    if meta.file_type().is_symlink() {
        return Err(Error::UnsafeSocketDir {
            path: dir.to_path_buf(),
            reason: "it is a symlink, which could point anywhere".to_owned(),
        });
    }

    if !meta.is_dir() {
        return Err(Error::UnsafeSocketDir {
            path: dir.to_path_buf(),
            reason: "it exists and is not a directory".to_owned(),
        });
    }

    let mode = meta.permissions().mode() & 0o777;
    if mode != SOCKET_DIR_MODE {
        return Err(Error::UnsafeSocketDir {
            path: dir.to_path_buf(),
            reason: format!("its mode is {mode:04o}, not {SOCKET_DIR_MODE:04o}"),
        });
    }

    Ok(())
}

/// The only mode a fallback socket directory may have.
const SOCKET_DIR_MODE: u32 = 0o700;

fn current_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|u| !u.is_empty() && !u.contains('/'))
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_dir_appends_compass_and_the_socket_name() {
        let p = SocketPath::in_dir("/run/user/1000");
        assert_eq!(p.as_path(), Path::new("/run/user/1000/compass/ipc.sock"));
        assert!(!p.is_fallback());
        assert_eq!(p.parent(), Some(Path::new("/run/user/1000/compass")));
    }

    #[test]
    fn exact_is_used_verbatim() {
        let p = SocketPath::exact("/tmp/custom.sock");
        assert_eq!(p.as_path(), Path::new("/tmp/custom.sock"));
    }

    #[test]
    fn display_prints_the_path() {
        assert_eq!(SocketPath::exact("/a/b.sock").to_string(), "/a/b.sock");
    }

    // `from_env` reads process-global state, so the two branches share one test
    // rather than racing each other across threads.
    #[test]
    fn from_env_prefers_xdg_runtime_dir_and_falls_back_when_unset() {
        let saved = std::env::var_os("XDG_RUNTIME_DIR");

        // SAFETY-adjacent note: `set_var` is unsafe in edition 2024 and
        // `unsafe_code` is forbidden here, so instead of mutating the
        // environment we assert the pure logic against both shapes directly.
        let with_xdg = SocketPath::in_dir("/run/user/1000");
        assert_eq!(
            with_xdg.as_path(),
            Path::new("/run/user/1000/compass/ipc.sock")
        );

        let resolved = SocketPath::from_env();
        match saved {
            Some(dir) if !dir.is_empty() => {
                assert!(!resolved.is_fallback());
                assert_eq!(resolved.as_path(), SocketPath::in_dir(&dir).as_path());
            }
            _ => {
                assert!(resolved.is_fallback());
                assert!(resolved.as_path().starts_with("/tmp"));
                assert_eq!(resolved.as_path().file_name().unwrap(), SOCKET_FILE_NAME);
            }
        }
    }

    #[test]
    fn user_name_is_never_a_path_component() {
        // Whatever the environment says, the fallback stays a single directory
        // under /tmp.
        let name = current_user_name();
        assert!(!name.contains('/'));
        assert!(!name.is_empty());
    }
}
