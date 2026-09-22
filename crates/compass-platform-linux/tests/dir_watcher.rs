//! Watching important directories for structural changes, on `notify`.
//!
//! Behaviour tests over real temporary trees and a real inotify backend.
//! Delivery is asynchronous, so every test polls [`drain`] with a bounded
//! wait rather than assuming an event has arrived.
//!
//! [`drain`]: compass_platform_linux::dir_watcher::DirWatcher::drain

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use compass_core::file_walk::IndexWalk;
use compass_core::watch_events::WatchEvent;
use compass_platform_linux::dir_watcher::DirWatcher;

fn walk() -> IndexWalk {
    IndexWalk::new(None)
}

fn home_entries(home: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(home)
        .expect("list fixture home")
        .map(|entry| entry.expect("fixture entry").path())
        .collect()
}

fn watch(home: &Path, walk: IndexWalk) -> DirWatcher {
    let entries = home_entries(home);
    DirWatcher::new(
        Some(home),
        &entries,
        |path| path.is_dir(),
        |path| path.is_symlink(),
        &[],
        walk,
    )
    .expect("start the watcher")
}

/// Drains until an event matches, or the wait runs out.
///
/// Returns everything seen, since the matching event may arrive alongside
/// others the test also wants to look at.
fn drain_until(
    watcher: &mut DirWatcher,
    mut matches: impl FnMut(&WatchEvent) -> bool,
    what: &str,
) -> Vec<WatchEvent> {
    let start = Instant::now();
    let mut seen = Vec::new();
    loop {
        seen.extend(watcher.drain());
        if seen.iter().any(&mut matches) {
            return seen;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("timed out waiting for {what}; saw {seen:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn changed_root(home: &Path) -> WatchEvent {
    WatchEvent::DirectoryChanged(home.to_path_buf())
}

#[test]
fn a_created_file_prompts_for_its_directory() {
    let home = tempfile::tempdir().expect("fixture home");
    let mut watcher = watch(home.path(), walk());

    std::fs::write(home.path().join("new.txt"), "notes").expect("create fixture file");

    let seen = drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the creation prompt",
    );
    assert!(seen.contains(&changed_root(home.path())));
}

#[test]
fn a_new_subdirectory_is_watched_and_prompts() {
    let home = tempfile::tempdir().expect("fixture home");
    let mut watcher = watch(home.path(), walk());

    std::fs::create_dir(home.path().join("sub")).expect("create fixture dir");

    drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the creation prompt",
    );
    assert!(
        watcher
            .watched_directories()
            .contains(&home.path().join("sub"))
    );
}

#[test]
fn the_ceiling_holds_for_created_directories() {
    // Depth 2 is watched; what appears below it prompts but is left to the
    // periodic scan, exactly as if it had existed at startup.
    let home = tempfile::tempdir().expect("fixture home");
    let mut watcher = watch(home.path(), walk());

    // Staged: a directory born before its parent is watched earns no event —
    // the scan owns that gap — so each level settles before the next is made.
    std::fs::create_dir(home.path().join("a")).expect("fixture dir");
    drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "watching a",
    );
    std::fs::create_dir(home.path().join("a/b")).expect("fixture dir");
    drain_until(
        &mut watcher,
        |event| *event == WatchEvent::DirectoryChanged(home.path().join("a")),
        "watching a/b",
    );

    std::fs::create_dir(home.path().join("a/b/c")).expect("too-deep dir");

    let seen = drain_until(
        &mut watcher,
        |event| *event == WatchEvent::DirectoryChanged(home.path().join("a/b")),
        "the parent's prompt",
    );
    assert!(seen.contains(&WatchEvent::DirectoryChanged(home.path().join("a/b"))));
    assert!(
        !watcher
            .watched_directories()
            .contains(&home.path().join("a/b/c"))
    );
}

#[test]
fn a_removed_file_prompts() {
    let home = tempfile::tempdir().expect("fixture home");
    std::fs::write(home.path().join("gone.txt"), "notes").expect("fixture file");
    let mut watcher = watch(home.path(), walk());
    watcher.drain();

    std::fs::remove_file(home.path().join("gone.txt")).expect("remove fixture file");

    let seen = drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the removal prompt",
    );
    assert!(seen.contains(&changed_root(home.path())));
}

#[test]
fn a_renamed_file_prompts() {
    let home = tempfile::tempdir().expect("fixture home");
    std::fs::write(home.path().join("old.txt"), "notes").expect("fixture file");
    let mut watcher = watch(home.path(), walk());
    watcher.drain();

    std::fs::rename(home.path().join("old.txt"), home.path().join("new.txt"))
        .expect("rename fixture file");

    let seen = drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the rename prompt",
    );
    assert!(seen.contains(&changed_root(home.path())));
}

#[test]
fn rewriting_a_file_does_not_prompt() {
    // The mask watches creation, removal, and renames — not content edits. A
    // rewrite changes no structure, and the periodic scan's cutoff catches it.
    let home = tempfile::tempdir().expect("fixture home");
    std::fs::write(home.path().join("notes.txt"), "before").expect("fixture file");
    let mut watcher = watch(home.path(), walk());
    // The fixture predates the watch, so there is nothing to settle on: one
    // clearing drain, then the probe rewrite.
    watcher.drain();

    std::fs::write(home.path().join("notes.txt"), "after").expect("rewrite fixture file");
    std::thread::sleep(Duration::from_millis(300));

    assert!(watcher.drain().is_empty());
}

#[test]
fn a_removed_watched_directory_is_dropped_quietly() {
    // The directory itself went away: its watch is cleaned up like IN_IGNORED,
    // with no event for it — but its parent still reports the removal.
    let home = tempfile::tempdir().expect("fixture home");
    std::fs::create_dir(home.path().join("sub")).expect("fixture dir");
    let mut watcher = watch(home.path(), walk());
    assert!(
        watcher
            .watched_directories()
            .contains(&home.path().join("sub"))
    );

    std::fs::remove_dir(home.path().join("sub")).expect("remove fixture dir");

    let seen = drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the parent's prompt",
    );
    assert!(seen.contains(&changed_root(home.path())));
    assert!(
        !seen.contains(&WatchEvent::DirectoryChanged(home.path().join("sub"))),
        "no event for the directory itself"
    );
    assert!(
        !watcher
            .watched_directories()
            .contains(&home.path().join("sub"))
    );
}

#[test]
fn a_filtered_new_directory_prompts_but_is_not_watched() {
    // node_modules is excluded by the shipped list: its appearance is still a
    // change worth scanning, but it never costs a watch.
    let home = tempfile::tempdir().expect("fixture home");
    let mut watcher = watch(home.path(), walk());

    std::fs::create_dir(home.path().join("node_modules")).expect("filtered dir");

    let seen = drain_until(
        &mut watcher,
        |event| *event == changed_root(home.path()),
        "the parent's prompt",
    );
    assert!(seen.contains(&changed_root(home.path())));
    assert!(
        !watcher
            .watched_directories()
            .contains(&home.path().join("node_modules"))
    );
}

#[test]
fn dynamic_directories_are_leaves() {
    // Dynamic watches come from the recent-directories list and sit at the
    // deepest depth: nothing is extended below them.
    let home = tempfile::tempdir().expect("fixture home");
    let elsewhere = tempfile::tempdir().expect("dynamic dir");
    let mut watcher = watch(home.path(), walk());

    watcher.set_dynamic_directories(&[elsewhere.path().to_path_buf()]);
    assert!(
        watcher
            .watched_directories()
            .contains(&elsewhere.path().to_path_buf())
    );

    std::fs::create_dir(elsewhere.path().join("sub")).expect("dir under a leaf");
    std::thread::sleep(Duration::from_millis(300));

    assert!(
        !watcher
            .watched_directories()
            .contains(&elsewhere.path().join("sub")),
        "no extension below a dynamic leaf"
    );
}

#[test]
fn dynamic_directories_stop_being_watched_when_they_leave_the_list() {
    let home = tempfile::tempdir().expect("fixture home");
    let elsewhere = tempfile::tempdir().expect("dynamic dir");
    let mut watcher = watch(home.path(), walk());
    watcher.set_dynamic_directories(&[elsewhere.path().to_path_buf()]);
    assert!(
        watcher
            .watched_directories()
            .contains(&elsewhere.path().to_path_buf())
    );

    watcher.set_dynamic_directories(&[]);

    assert!(
        !watcher
            .watched_directories()
            .contains(&elsewhere.path().to_path_buf())
    );
}

#[test]
fn the_roots_are_watched_and_the_budget_holds() {
    let home = tempfile::tempdir().expect("fixture home");
    let watcher = watch(home.path(), walk());

    assert!(
        watcher
            .root_directories()
            .contains(&home.path().to_path_buf())
    );
    assert!(
        watcher
            .watched_directories()
            .contains(&home.path().to_path_buf())
    );
    assert!(!watcher.budget_exhausted());
}
