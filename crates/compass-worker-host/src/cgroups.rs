//! cgroups v2 memory limit for extension workers.
//!
//! Phase 4 confines each worker to 256 MB (268435456 bytes) via
//! `memory.max` in its cgroup. The helper is deliberately trivial so the
//! host can call it after creating the cgroup directory but before moving the
//! worker's pid into `cgroup.procs`.

use std::io;
use std::path::Path;

pub const MEMORY_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// Write `MEMORY_LIMIT_BYTES` to `<cgroup>/memory.max`.
///
/// The cgroup directory must already exist (created by the host via
/// `mkdir` under `/sys/fs/cgroup/compass/<extension-id>` or via systemd's
/// `TransientUnit`). Failures are propagated so the host can decide to
/// refuse the worker rather than run it unconfined.
pub fn limit_memory(cgroup: &Path) -> io::Result<()> {
    std::fs::write(cgroup.join("memory.max"), MEMORY_LIMIT_BYTES.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_memory_writes_256mib() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Simulate a cgroup directory with a memory.max file.
        let cgroup = dir.path().join("compass.test");
        std::fs::create_dir_all(&cgroup).unwrap();
        std::fs::write(cgroup.join("memory.max"), b"").unwrap();
        limit_memory(&cgroup).unwrap();
        let written = std::fs::read_to_string(cgroup.join("memory.max")).unwrap();
        assert_eq!(written, MEMORY_LIMIT_BYTES.to_string());
        assert_eq!(written, "268435456");
    }

    #[test]
    fn limit_memory_fails_when_cgroup_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope/memory.max");
        let cgroup = dir.path().join("nope");
        // No directory -> write fails.
        assert!(limit_memory(&cgroup).is_err());
        assert!(!missing.exists());
    }
}
