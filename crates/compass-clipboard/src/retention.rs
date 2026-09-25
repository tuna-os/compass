//! How long history is kept: the eviction threshold and when to sweep next.
//!
//! Ports the timing half of `ClipboardService::setHistoryEvictionThreshold`,
//! `armEvictionTimer` and `runEvictionPass`, and the threshold the clipboard
//! extension parses from its `evictionThreshold` preference. The sweep itself
//! is [`crate::write::evict_older_than`] and [`crate::write::oldest_evictable`].
//!
//! Times here are milliseconds since the epoch, which is what this store
//! keeps in `updated_at` (the C++ keeps seconds; the Vicinae importer
//! converts).

use std::time::Duration;

/// How long after the threshold is set before the first sweep:
/// `MISCONFIGURATION_GRACE_DELAY`. A person who picks "15 minutes" when they
/// meant "1 month" gets a minute to notice before the history goes.
pub const MISCONFIGURATION_GRACE: Duration = Duration::from_secs(60);

/// The longest the timer is ever armed for (`maxDelay`, six hours), so a
/// clock change or a suspend cannot push the next sweep out indefinitely.
pub const MAX_DELAY: Duration = Duration::from_secs(6 * 60 * 60);

/// The shortest (one second), so a sweep that finds something already due
/// does not spin.
pub const MIN_DELAY: Duration = Duration::from_secs(1);

/// The `evictionThreshold` dropdown's options, as `(title, value)`: the value
/// is seconds, or `never`.
pub const PRESETS: &[(&str, &str)] = &[
    ("Never", "never"),
    ("15 minutes", "900"),
    ("1 hour", "3600"),
    ("1 day", "86400"),
    ("1 week", "604800"),
    ("1 month", "2592000"),
    ("1 year", "31536000"),
];

/// The threshold an `evictionThreshold` value names: seconds as a positive
/// integer, anything else (`never`, a negative or zero count, garbage, no
/// value) meaning history is kept for ever.
#[must_use]
pub fn parse_threshold(value: Option<&str>) -> Option<Duration> {
    let seconds: i64 = value?.trim().parse().ok()?;
    u64::try_from(seconds)
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

/// How long to wait before the next sweep, given the oldest evictable entry:
/// until it crosses the threshold, and a second more so it has, clamped to
/// [`MIN_DELAY`]..=[`MAX_DELAY`].
#[must_use]
pub fn next_delay(oldest_ms: i64, threshold: Duration, now_ms: i64) -> Duration {
    let threshold_ms = i64::try_from(threshold.as_millis()).unwrap_or(i64::MAX);
    let due_in_ms = oldest_ms
        .saturating_add(threshold_ms)
        .saturating_sub(now_ms)
        .saturating_add(1000);
    let due_in = Duration::from_millis(u64::try_from(due_in_ms).unwrap_or(0));
    due_in.clamp(MIN_DELAY, MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threshold_is_positive_seconds_and_everything_else_is_never() {
        assert_eq!(parse_threshold(Some("900")), Some(Duration::from_secs(900)));
        assert_eq!(parse_threshold(Some("never")), None);
        assert_eq!(parse_threshold(Some("0")), None);
        assert_eq!(parse_threshold(Some("-5")), None);
        assert_eq!(parse_threshold(None), None);
        for (_, value) in &PRESETS[1..] {
            assert!(parse_threshold(Some(value)).is_some(), "{value}");
        }
    }

    #[test]
    fn the_next_sweep_is_when_the_oldest_entry_crosses_the_threshold() {
        let hour = Duration::from_secs(3600);
        let now = 10_000_000;
        // Copied 59 minutes ago: due in a minute, plus the second's margin.
        assert_eq!(
            next_delay(now - 59 * 60 * 1000, hour, now),
            Duration::from_secs(61)
        );
        // Already overdue: the minimum, not zero.
        assert_eq!(next_delay(now - 2 * 3600 * 1000, hour, now), MIN_DELAY);
        // A year's threshold still sweeps within six hours.
        let year = Duration::from_secs(31_536_000);
        assert_eq!(next_delay(now, year, now), MAX_DELAY);
    }
}
