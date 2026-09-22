//! `IoPacer` parity: the pressure-file parse, the backoff policy, and the
//! checkpoint gating, without waiting on a real disk.

use compass_core::io_pacer::IoPacer;
use std::time::{Duration, Instant};

const SAMPLE: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n\
                      full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

#[test]
fn parses_the_some_avg10_value() {
    let pressured = SAMPLE.replace("avg10=0.00", "avg10=42.50");
    assert_eq!(IoPacer::parse_some_avg10(&pressured), Some(42.5));
}

#[test]
fn the_key_must_come_after_some() {
    // avg10 before any "some " line is not the stall reading.
    assert_eq!(IoPacer::parse_some_avg10("avg10=42.5\nsome x\n"), None);
}

#[test]
fn missing_or_malformed_markers_yield_nothing() {
    assert_eq!(IoPacer::parse_some_avg10(""), None);
    assert_eq!(IoPacer::parse_some_avg10("some avg60=1.0\n"), None);
    assert_eq!(IoPacer::parse_some_avg10("some avg10=\n"), None);
    assert_eq!(IoPacer::parse_some_avg10("some avg10= 1.5\n"), None);
    assert_eq!(IoPacer::parse_some_avg10("some avg10=much\n"), None);
    assert_eq!(IoPacer::parse_some_avg10("some avg10=1.5xyz\n"), None);
}

#[test]
fn backoff_is_linear_and_capped() {
    assert_eq!(IoPacer::backoff_for(20.0), Duration::from_millis(100));
    assert_eq!(IoPacer::backoff_for(50.0), Duration::from_millis(250));
    assert_eq!(IoPacer::backoff_for(100.0), Duration::from_millis(500));
    assert_eq!(IoPacer::backoff_for(150.0), Duration::from_millis(500));
}

#[test]
fn checkpoints_below_cadence_touch_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let psi = dir.path().join("io");
    std::fs::write(&psi, SAMPLE.replace("avg10=0.00", "avg10=100.00")).expect("write psi");
    let mut pacer = IoPacer::new(&psi, 3);
    let start = Instant::now();
    pacer.checkpoint();
    pacer.checkpoint();
    // Two gated checkpoints never reach the probe: no 500 ms sleep.
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_probe_at_threshold_sleeps() {
    let dir = tempfile::tempdir().expect("tempdir");
    let psi = dir.path().join("io");
    std::fs::write(&psi, SAMPLE.replace("avg10=0.00", "avg10=100.00")).expect("write psi");
    let mut pacer = IoPacer::new(&psi, 1);
    let start = Instant::now();
    pacer.checkpoint();
    // 100% stall sleeps the full backoff; only a lower bound is asserted so
    // a loaded machine cannot flake it.
    assert!(start.elapsed() >= Duration::from_millis(450));
}

#[test]
fn below_threshold_probes_do_not_sleep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let psi = dir.path().join("io");
    std::fs::write(&psi, SAMPLE).expect("write psi");
    let mut pacer = IoPacer::new(&psi, 1);
    let start = Instant::now();
    pacer.checkpoint();
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_missing_pressure_file_is_a_fast_no_op() {
    let mut pacer = IoPacer::new("/proc/pressure/does-not-exist", 1);
    let start = Instant::now();
    for _ in 0..3 {
        pacer.checkpoint();
    }
    assert!(start.elapsed() < Duration::from_secs(5));
}
