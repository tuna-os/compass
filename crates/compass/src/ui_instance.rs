//! A process-lifetime lease for the UI belonging to one engine socket.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// Holds the single-UI lease until dropped. A busy lease is not an error.
pub fn acquire(socket: &Path) -> std::io::Result<Option<File>> {
    let path = socket.with_extension("ui.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_ui_owns_a_socket_and_exit_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("ipc.sock");
        let owner = acquire(&socket).unwrap().unwrap();
        assert!(acquire(&socket).unwrap().is_none());
        assert!(acquire(&dir.path().join("another.sock")).unwrap().is_some());
        drop(owner);
        assert!(acquire(&socket).unwrap().is_some());
    }
}
