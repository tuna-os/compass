//! cgroups v2 memory limit for extension workers.
//!
//! Phase 4 confines each worker to 256 MB (268435456 bytes) via
//! `memory.max` in its cgroup. Two mechanisms are supported, in order:
//!
//! 1. **systemd `TransientUnit`** via `org.freedesktop.systemd1` — preferred
//!    on a systemd host (and the only thing that works inside a Flatpak where
//!    `/sys/fs/cgroup` is not writable). The host asks systemd to create a
//!    transient scope with `MemoryMax` set, then moves the worker's pid into
//!    it.
//! 2. **Direct `memory.max` write** — fallback when systemd is not available
//!    (tests, containers without systemd). The cgroup directory must already
//!    exist.
//!
//! The helper is deliberately small so the host can call it after creating the
//! cgroup directory but before moving the worker's pid into `cgroup.procs`.

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

/// Try systemd `TransientUnit` first, fallback to direct write.
///
/// On a systemd host this creates a transient scope
/// `compass-extension-<id>.scope` with `MemoryMax` set via
/// `org.freedesktop.systemd1.Manager.StartTransientUnit`. When systemd is
/// not reachable (no bus, no permission, inside a test container) the
/// direct `limit_memory` path is used. The `cgroup` path is still the
/// filesystem location for the fallback; systemd's scope path is derived
/// from `scope_name`.
///
/// Returns the mechanism that succeeded, so the host can log degradation.
pub async fn limit_memory_via_systemd(
    cgroup: &Path,
    scope_name: &str,
) -> io::Result<LimitMechanism> {
    if let Ok(mechanism) = try_systemd_transient(scope_name).await {
        return Ok(mechanism);
    }
    limit_memory(cgroup)?;
    Ok(LimitMechanism::DirectWrite)
}

/// Which mechanism confined the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitMechanism {
    /// systemd `TransientUnit` with `MemoryMax`.
    SystemdTransient,
    /// Direct `memory.max` write.
    DirectWrite,
}

async fn try_systemd_transient(scope_name: &str) -> io::Result<LimitMechanism> {
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let props: Vec<(String, zbus::zvariant::Value<'_>)> = vec![(
        "MemoryMax".to_owned(),
        zbus::zvariant::Value::new(MEMORY_LIMIT_BYTES),
    )];
    let _: zbus::zvariant::OwnedObjectPath = proxy
        .call(
            "StartTransientUnit",
            &(
                scope_name,
                "fail",
                props,
                Vec::<(String, zbus::zvariant::Value<'_>)>::new(),
            ),
        )
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(LimitMechanism::SystemdTransient)
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

    #[tokio::test]
    async fn systemd_fallback_writes_directly_when_no_bus() {
        // In a container without a system bus, limit_memory_via_systemd must
        // fallback to direct write rather than fail the worker. The fallback
        // is what keeps the host usable on non-systemd images and in tests.
        let dir = tempfile::tempdir().expect("tempdir");
        let cgroup = dir.path().join("compass.fallback");
        std::fs::create_dir_all(&cgroup).unwrap();
        std::fs::write(cgroup.join("memory.max"), b"").unwrap();
        let mechanism = limit_memory_via_systemd(&cgroup, "compass-test-fallback.scope")
            .await
            .expect("fallback succeeds");
        assert_eq!(mechanism, LimitMechanism::DirectWrite);
        let written = std::fs::read_to_string(cgroup.join("memory.max")).unwrap();
        assert_eq!(written, "268435456");
    }

    #[test]
    fn memory_limit_is_256_mib_constant() {
        // Typed guard: raising the cap is a deliberate product decision with
        // its own issue, not a one-line tweak. The constant is the gate.
        assert_eq!(MEMORY_LIMIT_BYTES, 256 * 1024 * 1024);
        assert_eq!(MEMORY_LIMIT_BYTES, 268435456);
    }
}
