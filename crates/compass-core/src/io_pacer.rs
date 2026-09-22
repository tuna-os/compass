//! Cooperative IO throttle, ported from `file-indexer/io-pacer.hpp`.
//!
//! The scanner calls [`IoPacer::checkpoint`] as it walks; every
//! [`IoPacer::CHECKPOINTS_PER_PROBE`]th call re-reads `/proc/pressure/io` and
//! sleeps when the `some avg10` stall percentage is at or above threshold. On
//! kernels without `CONFIG_PSI` the pacer constructs unavailable and every
//! checkpoint is a no-op — same as the C++, where `exists()` fails.
//!
//! Performance notes: [`IoPacer::parse_some_avg10`] works on slices with no
//! allocation and early-outs on the first missing marker; the pressure file
//! is read at most once per probe interval, exactly like the C++. One
//! deliberate strictness: the C++ reads the value with `std::from_chars`,
//! which accepts a valid float prefix before trailing garbage
//! (`avg10=12.5xyz` yields 12.5); this port rejects trailing garbage
//! instead. Real pressure files never contain it, and strictness keeps the
//! parse to a single pass with no manual prefix scan.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Cooperative IO throttle. See the module docs.
pub struct IoPacer {
    psi_path: PathBuf,
    checkpoints_per_probe: usize,
    available: bool,
    counter: usize,
    last_probe: Option<Instant>,
}

impl IoPacer {
    /// Probes happen at most this often; checkpoints between probes only bump
    /// the counter.
    pub const MIN_PROBE_INTERVAL: Duration = Duration::from_millis(250);
    /// Stall percentage at or above which a probe sleeps.
    pub const SOME_AVG10_THRESHOLD: f64 = 20.0;
    /// Longest sleep for one probe, reached at 100% stall.
    pub const MAX_BACKOFF: Duration = Duration::from_millis(500);
    /// Checkpoints between pressure-file probes.
    pub const CHECKPOINTS_PER_PROBE: usize = 256;

    /// A pacer reading `psi_path`, probing every `checkpoints_per_probe`
    /// checkpoints. Unavailable when the path does not exist.
    pub fn new(psi_path: impl Into<PathBuf>, checkpoints_per_probe: usize) -> Self {
        let psi_path = psi_path.into();
        let available = std::fs::exists(&psi_path).unwrap_or(false);
        Self {
            psi_path,
            checkpoints_per_probe,
            available,
            counter: 0,
            last_probe: None,
        }
    }

    /// A pacer on the live pressure file with the default probe cadence.
    pub fn system() -> Self {
        Self::new(Path::new("/proc/pressure/io"), Self::CHECKPOINTS_PER_PROBE)
    }

    /// Maybe sleep so a hot disk can drain. Cheap until a probe is due.
    pub fn checkpoint(&mut self) {
        if !self.available {
            return;
        }
        self.counter = self.counter.saturating_add(1);
        if self.counter < self.checkpoints_per_probe {
            return;
        }
        self.counter = 0;

        let now = Instant::now();
        if self
            .last_probe
            .is_some_and(|last| now - last < Self::MIN_PROBE_INTERVAL)
        {
            return;
        }
        self.last_probe = Some(now);

        let Ok(content) = std::fs::read_to_string(&self.psi_path) else {
            return;
        };
        if let Some(sleep) = Self::sleep_for_content(&content) {
            std::thread::sleep(sleep);
        }
    }

    /// How long to sleep for pressure-file `content`, if at all. Pure, so the
    /// policy is testable without touching the disk or sleeping.
    fn sleep_for_content(content: &str) -> Option<Duration> {
        let pressure = Self::parse_some_avg10(content)?;
        if pressure < Self::SOME_AVG10_THRESHOLD {
            return None;
        }
        Some(Self::backoff_for(pressure))
    }

    /// Sleep for a stall percentage at or above threshold: linear in the
    /// percentage, capped at [`IoPacer::MAX_BACKOFF`].
    pub fn backoff_for(pressure: f64) -> Duration {
        let ratio = (pressure / 100.0).min(1.0);
        Self::MAX_BACKOFF.mul_f64(ratio)
    }

    /// The `some avg10=` stall percentage in pressure-file `content`.
    ///
    /// Finds `"some "`, then `"avg10="` after it, and parses the number up to
    /// the next whitespace with no leading-whitespace skip — matching
    /// `std::from_chars`, which likewise rejects `"avg10= 1.5"`.
    pub fn parse_some_avg10(content: &str) -> Option<f64> {
        const KEY: &str = "avg10=";
        let some = content.find("some ")?;
        let key = content[some..].find(KEY)? + some;
        let rest = &content[key + KEY.len()..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..end];
        if token.is_empty() {
            return None;
        }
        token.parse().ok()
    }
}
