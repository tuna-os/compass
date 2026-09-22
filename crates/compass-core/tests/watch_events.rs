//! What a filesystem event turns into: which scans get enqueued.
//!
//! Read off `FileSystemWatcher` (`src/file-indexer/src/file-system-watcher.cpp`).

use std::path::{Path, PathBuf};

use compass_core::incremental_scan::Mode;
use compass_core::watch_events::{
    BACKGROUND_UPDATE_DEPTH, BACKGROUND_UPDATE_INTERVAL_SECS, DYNAMIC_WATCH_COUNT, ScanRequest,
    WatchConfig, WatchEvent, background_sweep_requests, dynamic_watch_dirs, requests_for_event,
    should_scan_path,
};

fn config() -> WatchConfig {
    WatchConfig {
        entrypoints: vec![PathBuf::from("/home/ada"), PathBuf::from("/media/data")],
        excluded_paths: vec![PathBuf::from("/home/ada/Videos")],
        excluded_filenames: vec!["target".to_owned()],
    }
}

fn is_anything_a_directory(path: &Path) -> bool {
    let _ = path;
    true
}

#[test]
fn the_shipped_cadence_is_what_it_is() {
    // A minute between sweeps, five levels deep, two thousand dynamic watches.
    // Read through the constants everywhere else, so nothing else would notice
    // them changing.
    assert_eq!(BACKGROUND_UPDATE_INTERVAL_SECS, 60);
    assert_eq!(BACKGROUND_UPDATE_DEPTH, 5);
    assert_eq!(DYNAMIC_WATCH_COUNT, 2048);
}

#[test]
fn a_covered_path_earns_a_scan() {
    let config = config();
    assert!(should_scan_path(&config, Path::new("/home/ada/notes.txt")));
    assert!(should_scan_path(
        &config,
        Path::new("/media/data/photo.jpg")
    ));
}

#[test]
fn an_uncovered_path_earns_nothing() {
    let config = config();
    assert!(!should_scan_path(&config, Path::new("/tmp/download")));
}

#[test]
fn an_excluded_path_earns_nothing_even_under_an_entrypoint() {
    let config = config();
    assert!(!should_scan_path(
        &config,
        Path::new("/home/ada/Videos/holiday.mp4")
    ));
}

#[test]
fn the_decision_is_on_the_directory_not_the_spelling() {
    // a/../b is the same directory as b, and the router decides on the
    // directory.
    let config = config();
    assert!(should_scan_path(
        &config,
        Path::new("/home/ada/a/../notes.txt")
    ));
}

#[test]
fn a_change_earns_one_debounced_pruned_scan_of_the_normalized_directory() {
    let event = WatchEvent::DirectoryChanged(PathBuf::from("/home/ada/a/../code"));

    assert_eq!(
        requests_for_event(&config(), &event),
        [ScanRequest {
            path: PathBuf::from("/home/ada/code"),
            mode: Mode::Pruned,
            debounced: true,
            max_depth: None,
            excluded_paths: vec![PathBuf::from("/home/ada/Videos")],
            excluded_filenames: vec!["target".to_owned()],
        }]
    );
}

#[test]
fn a_change_outside_the_roots_earns_nothing() {
    let event = WatchEvent::DirectoryChanged(PathBuf::from("/tmp/download"));

    assert!(requests_for_event(&config(), &event).is_empty());
}

#[test]
fn a_change_under_an_exclusion_earns_nothing() {
    let event = WatchEvent::DirectoryChanged(PathBuf::from("/home/ada/Videos"));

    assert!(requests_for_event(&config(), &event).is_empty());
}

#[test]
fn a_degraded_watcher_rescans_the_covered_entrypoints() {
    let requests = requests_for_event(&config(), &WatchEvent::Degraded);

    // Both entrypoints are covered and neither is excluded: each earns its own
    // debounced pruned scan, since the watcher can no longer say what changed.
    assert_eq!(
        requests
            .iter()
            .map(|request| request.path.clone())
            .collect::<Vec<_>>(),
        [PathBuf::from("/home/ada"), PathBuf::from("/media/data")]
    );
    assert!(
        requests
            .iter()
            .all(|request| request.mode == Mode::Pruned && request.debounced)
    );
}

#[test]
fn a_degraded_watcher_skips_an_excluded_entrypoint() {
    let mut config = config();
    config.entrypoints.push(PathBuf::from("/home/ada/Videos"));

    let requests = requests_for_event(&config, &WatchEvent::Degraded);

    assert!(
        !requests
            .iter()
            .any(|request| request.path == Path::new("/home/ada/Videos"))
    );
}

#[test]
fn the_sweep_scans_every_directory_entrypoint_five_deep_without_waiting() {
    let requests = background_sweep_requests(&config(), is_anything_a_directory);

    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.mode == Mode::Exhaustive
                && !request.debounced
                && request.max_depth == Some(BACKGROUND_UPDATE_DEPTH))
    );
}

#[test]
fn the_sweep_skips_an_entrypoint_that_stopped_being_a_directory() {
    let requests = background_sweep_requests(&config(), |path| path != Path::new("/media/data"));

    assert_eq!(
        requests
            .iter()
            .map(|request| request.path.clone())
            .collect::<Vec<_>>(),
        [PathBuf::from("/home/ada")]
    );
}

#[test]
fn the_sweep_sends_entrypoints_as_configured_not_normalized() {
    // handleEvent normalizes; the sweep enqueues the entrypoint as held.
    let config = WatchConfig {
        entrypoints: vec![PathBuf::from("/home/ada//code")],
        ..WatchConfig::default()
    };

    let requests = background_sweep_requests(&config, is_anything_a_directory);

    assert_eq!(
        requests
            .iter()
            .map(|request| request.path.clone())
            .collect::<Vec<_>>(),
        [PathBuf::from("/home/ada//code")]
    );
}

#[test]
fn dynamic_watches_keep_only_what_would_earn_scans() {
    let recent = vec![
        PathBuf::from("/home/ada/code"),
        PathBuf::from("/tmp/download"),
        PathBuf::from("/home/ada/Videos"),
    ];

    assert_eq!(
        dynamic_watch_dirs(&config(), recent),
        [PathBuf::from("/home/ada/code")]
    );
}

#[test]
fn dynamic_watches_are_capped_at_the_shipped_count() {
    let recent: Vec<PathBuf> = (0..DYNAMIC_WATCH_COUNT + 2)
        .map(|n| PathBuf::from(format!("/home/ada/dir{n}")))
        .collect();

    assert_eq!(
        dynamic_watch_dirs(&config(), recent).len(),
        DYNAMIC_WATCH_COUNT
    );
}
