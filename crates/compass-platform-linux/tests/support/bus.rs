//! Copied from `compass-shell/tests/support/bus.rs`.
//!
//! Duplicated rather than shared: cargo has no way to depend on another
//! crate's test support without making it part of that crate's public API,
//! and a `compass-testkit` module for eight lines of process handling would
//! put a `dbus-daemon` dependency in everything that links the testkit.
//! A private D-Bus session bus, one per test.
//!
//! Tests must not depend on an ambient bus: `DBUS_SESSION_BUS_ADDRESS` may be
//! absent in CI, and sharing a bus between tests makes them order-dependent.
//! Each [`TestBus`] spawns its own `dbus-daemon --session` and hands out its
//! address, so tests are isolated and parallel-safe.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

pub struct TestBus {
    child: Child,
    address: String,
}

impl TestBus {
    /// Start a private session bus.
    ///
    /// Returns `None` when there is no `dbus-daemon` on this machine, so the
    /// caller can skip loudly rather than pass silently.
    pub fn start() -> Option<Self> {
        let mut child = match Command::new("dbus-daemon")
            .args(["--session", "--print-address", "--nofork"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
        };

        let stdout = child.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("dbus-daemon should print its address");
        let address = line.trim().to_owned();
        assert!(
            address.starts_with("unix:"),
            "unexpected dbus-daemon address: {address:?}"
        );

        Some(Self { child, address })
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The environment variable that turns a skip into a failure.
///
/// A skip is the right behaviour on a developer machine without `dbus-daemon`
/// and the wrong behaviour in CI, where 21 tests quietly not running looks
/// exactly like 21 tests passing. The GitHub Actions Ubuntu image ships
/// `dbus-daemon` today — checked in the log of a real run, not assumed — and
/// nothing guaranteed it would keep doing so. The Rust workflow sets this, so
/// an image change fails the build instead of hollowing out the suite.
pub const REQUIRE_ENV: &str = "COMPASS_REQUIRE_DBUS";

/// Start a private bus, or return `None` after a loud message.
///
/// Tests call this as the first line and `return` on `None`, so a machine
/// without `dbus-daemon` skips visibly instead of passing silently. When
/// [`REQUIRE_ENV`] is set to anything other than `0`, there is no skip: the
/// absence of `dbus-daemon` is a failure.
pub fn start_or_skip(test_name: &str) -> Option<TestBus> {
    match TestBus::start() {
        Some(bus) => Some(bus),
        None => {
            let required = std::env::var(REQUIRE_ENV).is_ok_and(|v| v != "0");
            assert!(
                !required,
                "{test_name} needs a `dbus-daemon` binary and there is none on PATH. \
                 {REQUIRE_ENV} is set, so this is a failure rather than a skip: the D-Bus \
                 suite is the whole of Suite 3a and an environment that cannot run it must \
                 not report success."
            );
            eprintln!(
                "\n\
                 ############################################################\n\
                 # SKIPPED {test_name}: no `dbus-daemon` binary on PATH.\n\
                 # compass-platform-linux's KWin tests exercise a REAL session bus and\n\
                 # cannot be meaningfully faked. Install dbus to run them\n\
                 # (Fedora: dbus-daemon, Debian/Ubuntu: dbus-bin).\n\
                 #\n\
                 # Set {REQUIRE_ENV}=1 to make this a failure instead, which is\n\
                 # what CI does.\n\
                 ############################################################\n"
            );
            None
        }
    }
}
