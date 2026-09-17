//! The fallback socket directory must be exclusively ours.
//!
//! #88: `Listener::bind` creates the socket's parent with
//! `DirBuilder::recursive(true).mode(0o700)`, and `recursive(true)` **succeeds
//! on a directory that already exists, leaving its mode alone**. On the `/tmp`
//! fallback that means another local user can create `/tmp/vicinae-$USER`
//! first, at permissive modes, and the engine will happily bind its socket
//! inside a directory that user can write to.
//!
//! These drive the predicate directly, against real directories with real
//! modes, because the mode is the whole subject.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use compass_ipc::{Error, SocketPath, ensure_private_dir};

/// A self-cleaning temporary directory, hand-rolled to keep this crate's
/// dependency set exactly what the workspace declares.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cdir-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates `<root>/name` with exactly `mode`.
fn dir_with_mode(root: &Path, name: &str, mode: u32) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).expect("create");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("chmod");
    dir
}

#[test]
fn a_directory_that_does_not_exist_yet_is_fine() {
    // The ordinary first start: the caller creates it 0700 and is its owner by
    // construction, so there is nothing to refuse.
    let tmp = TempDir::new();
    let missing = tmp.path().join("not-created-yet");
    assert!(ensure_private_dir(&missing).is_ok());
}

#[test]
fn our_own_private_directory_is_accepted() {
    let tmp = TempDir::new();
    let dir = dir_with_mode(tmp.path(), "private", 0o700);
    assert!(
        ensure_private_dir(&dir).is_ok(),
        "0700 is exactly what we create"
    );
}

#[test]
fn a_world_writable_directory_is_refused() {
    // THE ATTACK. Another user creates /tmp/vicinae-victim first, permissively,
    // and waits for the victim to bind a socket inside it.
    let tmp = TempDir::new();
    let dir = dir_with_mode(tmp.path(), "hijacked", 0o777);

    match ensure_private_dir(&dir) {
        Err(Error::UnsafeSocketDir { reason, .. }) => {
            assert!(
                reason.contains("0777"),
                "the refusal should name the mode it found: {reason}"
            );
        }
        other => panic!("a 0777 socket directory must be refused, got {other:?}"),
    }
}

#[test]
fn a_group_readable_directory_is_refused_too() {
    // Not only 0777. Anything that is not exactly 0700 means someone other
    // than the owner has been granted something, and the whole point is that
    // nobody has.
    let tmp = TempDir::new();
    for mode in [0o750, 0o770, 0o701, 0o755] {
        let dir = dir_with_mode(tmp.path(), &format!("mode-{mode:o}"), mode);
        assert!(
            matches!(ensure_private_dir(&dir), Err(Error::UnsafeSocketDir { .. })),
            "mode {mode:04o} must be refused"
        );
    }
}

#[test]
fn a_symlink_is_refused_without_being_followed() {
    // Refused on sight rather than resolved: a symlink there could point at
    // anywhere the victim can write, and the mode check would then be reading
    // the target's mode rather than the thing at the path.
    let tmp = TempDir::new();
    let target = dir_with_mode(tmp.path(), "target", 0o700);
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    match ensure_private_dir(&link) {
        Err(Error::UnsafeSocketDir { reason, .. }) => {
            assert!(
                reason.contains("symlink"),
                "the refusal should say it is a symlink: {reason}"
            );
        }
        other => panic!("a symlinked socket directory must be refused, got {other:?}"),
    }
}

#[test]
fn something_that_is_not_a_directory_is_refused() {
    let tmp = TempDir::new();
    let file = tmp.path().join("a-file");
    std::fs::write(&file, b"not a directory").expect("write");
    assert!(matches!(
        ensure_private_dir(&file),
        Err(Error::UnsafeSocketDir { .. })
    ));
}

#[test]
fn an_explicit_directory_is_never_second_guessed() {
    // `in_dir` is the injection point for tests and for the Flatpak's own
    // runtime dir. A caller that names a directory has chosen it, and a
    // permissive one there is not ours to refuse -- only the /tmp fallback is.
    let tmp = TempDir::new();
    let dir = dir_with_mode(tmp.path(), "explicit", 0o777);
    let socket = SocketPath::in_dir(&dir);

    // The socket's parent is `<dir>/vicinae`, not `<dir>` -- and it has to be
    // made, permissively, or this passes because the parent does not exist
    // rather than because the path is not a fallback. A control caught exactly
    // that: removing the `is_fallback` guard left this test green.
    let parent = socket.as_path().parent().expect("a parent");
    std::fs::create_dir_all(parent).expect("create");
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    assert!(
        ensure_private_dir(parent).is_err(),
        "control: that parent is exactly what the predicate refuses"
    );

    assert!(!socket.is_fallback());
    assert!(
        socket.ensure_private_parent().is_ok(),
        "an explicitly chosen directory is the caller's business"
    );
}
