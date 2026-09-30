//! The client against a scripted helper: [`fake_indexer.py`] speaks the
//! protocol just well enough to configure, answer one canned query, and
//! emit scan events — or die on cue, so the crash paths run for real.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use compass::indexer_client::{ActiveScan, IndexerClient};
use compass::indexer_service::WireScanState;

fn fake_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake_indexer.py")
}

/// A runnable fake in `mode`: a shell wrapper so the mode rides along with
/// the program path, which is all [`IndexerClient::start_with`] takes.
fn fake(mode: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let wrapper = dir.path().join(format!("fake-indexer-{mode}"));
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec python3 {} {mode}\n",
            fake_source().display()
        ),
    )
    .expect("wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
            .expect("executable wrapper");
    }
    (dir, wrapper)
}

/// A client that stops on drop, so no fake outlives its test.
struct Guard {
    client: Arc<IndexerClient>,
}

impl Guard {
    fn start(wrapper: &Path) -> Self {
        let client = IndexerClient::new();
        client.start_with(wrapper);
        Self { client }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.client.stop();
    }
}

/// Spins until the helper runs, so queries never race its spawn: a query
/// sent before the supervisor stores stdin is honestly dropped, but the
/// tests mean to ask a running helper.
fn wait_until_running(client: &IndexerClient, what: &str) {
    let start = Instant::now();
    while !client.is_running() {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for {what} to run");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_until_stopped(client: &IndexerClient, what: &str) {
    let start = Instant::now();
    while client.is_running() {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for {what} to stop");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for(
    scans: &Mutex<Vec<ActiveScan>>,
    mut matches: impl FnMut(&[ActiveScan]) -> bool,
    what: &str,
    timeout: Duration,
) -> Vec<ActiveScan> {
    let start = Instant::now();
    loop {
        let seen = scans.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if matches(&seen) {
            return seen;
        }
        if start.elapsed() > timeout {
            panic!("timed out waiting for {what}, saw {seen:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn query_round_trips_through_a_fake_helper() {
    let (_dir, wrapper) = fake("serve");
    let guard = Guard::start(&wrapper);
    guard.client.configure(vec!["/tmp".to_owned()], vec![]);
    wait_until_running(&guard.client, "query helper");

    let matches = guard.client.query("report", 10, None);
    assert_eq!(matches.len(), 1, "the canned match comes back");
    let found = &matches[0];
    assert_eq!(found.path, PathBuf::from("/tmp/fake/report"));
    assert_eq!(found.rank, 1.5);
    assert_eq!(
        found.category,
        compass_db::query_engine::IndexedFileCategory::Document
    );
    assert_eq!(found.mime_type.as_deref(), Some("text/plain"));
}

#[test]
fn scans_track_until_they_terminate() {
    let (_dir, wrapper) = fake("serve");
    let guard = Guard::start(&wrapper);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    guard
        .client
        .set_scan_callback(move |scan| sink.lock().unwrap().push(scan));

    // The configure scan starts and stays tracked: nothing finishes it.
    guard.client.configure(vec!["/tmp".to_owned()], vec![]);
    wait_until_running(&guard.client, "scan helper");
    wait_for(
        &seen,
        |scans| {
            scans
                .iter()
                .any(|scan| scan.scan_id == 41 && scan.state == WireScanState::Started)
        },
        "configure scan start",
        Duration::from_secs(10),
    );
    assert!(
        guard
            .client
            .active_scans()
            .iter()
            .any(|scan| scan.scan_id == 41),
        "the unfinished scan stays tracked"
    );

    // The query's scan succeeds, which forgets it again.
    let _ = guard.client.query("report", 10, None);
    wait_for(
        &seen,
        |scans| {
            scans
                .iter()
                .any(|scan| scan.scan_id == 41 && scan.state == WireScanState::Succeeded)
        },
        "query scan success",
        Duration::from_secs(10),
    );
    assert!(
        guard.client.active_scans().is_empty(),
        "terminated scans leave the map"
    );
}

#[test]
fn a_crash_interrupts_scans_and_restarts() {
    let (_dir, wrapper) = fake("crash-now");
    let guard = Guard::start(&wrapper);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    guard
        .client
        .set_scan_callback(move |scan| sink.lock().unwrap().push(scan));

    // The configure scan starts, then the helper dies with it tracked: the
    // crash reports it interrupted, and the restart configures again. Each
    // cycle's start and interrupt route back to back — the fake is already
    // dead when its events arrive — so only the prefix is stable, never an
    // exact whole.
    guard.client.configure(vec!["/tmp".to_owned()], vec![]);
    let scans = wait_for(
        &seen,
        |scans| scans.len() >= 3,
        "start, interrupt, restart",
        Duration::from_secs(20),
    );
    let states: Vec<WireScanState> = scans.iter().take(3).map(|scan| scan.state).collect();
    assert_eq!(
        states,
        [
            WireScanState::Started,
            WireScanState::Interrupted,
            WireScanState::Started,
        ],
        "configure starts a scan, the crash interrupts it, the restart starts it again"
    );
    assert!(
        scans.iter().take(3).all(|scan| scan.scan_id == 41),
        "all three events name the same scan"
    );
}

#[test]
fn too_many_crashes_gives_up_on_restart() {
    let (_dir, wrapper) = fake("crash-now");
    let client = IndexerClient::new();
    client.set_base_restart_delay(Duration::from_millis(5));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    client.set_scan_callback(move |scan| sink.lock().unwrap().push(scan));
    client.start_with(&wrapper);

    // Five restarts at 5, 10, 20, 40, 80 milliseconds, then the supervisor
    // gives up: the helper counts as stopped and no further scans start.
    wait_until_running(&client, "crashed helper");
    wait_until_stopped(&client, "crashed helper");
    let settled = seen.lock().unwrap_or_else(PoisonError::into_inner).len();
    assert!(settled >= 3, "the crashes interrupted some scans first");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        seen.lock().unwrap_or_else(PoisonError::into_inner).len(),
        settled,
        "no restart follows the give-up"
    );
    client.stop();
}

#[test]
fn a_missing_helper_answers_nothing() {
    let client = IndexerClient::new();
    client.start_with(Path::new("/tmp/vicinae-no-such-helper"));
    assert!(!client.is_running(), "nothing spawns");
    assert!(client.query("report", 10, None).is_empty());
    client.stop();
}
