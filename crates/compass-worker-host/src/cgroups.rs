//! The memory an extension worker may use.
//!
//! Two caps, because neither alone holds everywhere:
//!
//! 1. **The JavaScript heap**, by Node's own `--max-old-space-size`
//!    ([`node_heap_flag`]). The flag is process-wide and bounds every isolate,
//!    the worker thread a command runs in included, whatever that worker's
//!    own `resourceLimits` ask for (the runtime asks for 1000 MB; measured,
//!    the flag wins). It needs nothing from the host, so it holds inside a
//!    Flatpak too, and a runaway extension's worker fails with
//!    `ERR_WORKER_OUT_OF_MEMORY` rather than taking the session with it.
//! 2. **The whole process**, by a transient systemd scope on the *user*
//!    manager holding the worker's pid with `MemoryMax` ([`confine`]). This
//!    caps what the heap flag cannot see (buffers, native modules). The user
//!    manager is the one an unprivileged engine may ask; the system manager
//!    refuses. Inside a Flatpak the manager is not reachable, and the app's
//!    own scope is what bounds it.

use std::io;

/// The most one worker may use, in all.
pub const MEMORY_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// Each JavaScript heap's cap, in MiB: below [`MEMORY_LIMIT_BYTES`], so V8
/// gives up before the kernel has to.
pub const HEAP_LIMIT_MIB: u64 = 160;

const _: () = assert!(HEAP_LIMIT_MIB * 1024 * 1024 < MEMORY_LIMIT_BYTES);

/// The Node flag that caps the heaps.
#[must_use]
pub fn node_heap_flag() -> String {
    format!("--max-old-space-size={HEAP_LIMIT_MIB}")
}

/// The scope a worker for `extension` with `pid` runs in:
/// `compass-extension-<extension>-<pid>.scope`, with anything systemd does not
/// allow in a unit name replaced by `_`.
#[must_use]
pub fn scope_name(extension: &str, pid: u32) -> String {
    let extension: String = extension
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("compass-extension-{extension}-{pid}.scope")
}

/// Moves `pid` into a new scope `scope` on the user's systemd, capped at
/// [`MEMORY_LIMIT_BYTES`]. The scope goes away with the process.
///
/// # Errors
///
/// When there is no session bus, the user manager is not on it (a Flatpak,
/// a container), or it refuses the unit.
pub async fn confine(pid: u32, scope: &str) -> io::Result<()> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let properties: Vec<(&str, zbus::zvariant::Value<'_>)> = vec![
        (
            "Description",
            zbus::zvariant::Value::from("A Compass extension"),
        ),
        ("PIDs", zbus::zvariant::Value::from(vec![pid])),
        ("MemoryMax", zbus::zvariant::Value::from(MEMORY_LIMIT_BYTES)),
        // Past the cap the scope is killed, not swapped into the ground.
        ("MemorySwapMax", zbus::zvariant::Value::from(0_u64)),
        (
            "CollectMode",
            zbus::zvariant::Value::from("inactive-or-failed"),
        ),
    ];
    let _: zbus::zvariant::OwnedObjectPath = proxy
        .call(
            "StartTransientUnit",
            &(
                scope,
                "fail",
                properties,
                Vec::<(&str, Vec<(&str, zbus::zvariant::Value<'_>)>)>::new(),
            ),
        )
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caps_are_256_mib_with_the_heap_below_it() {
        // Raising the cap is a product decision with its own issue, not a
        // one-line tweak.
        assert_eq!(MEMORY_LIMIT_BYTES, 268_435_456);
        assert_eq!(node_heap_flag(), "--max-old-space-size=160");
    }

    #[test]
    fn a_scope_name_is_a_valid_unit_name() {
        assert_eq!(
            scope_name("github", 42),
            "compass-extension-github-42.scope"
        );
        assert_eq!(
            scope_name("@me/my ext", 7),
            "compass-extension-_me_my_ext-7.scope"
        );
    }
}
