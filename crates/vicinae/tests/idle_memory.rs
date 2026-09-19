//! Idle RSS of the engine, measured on every commit.
//!
//! #13 asks for "per-commit RSS tracking with a >5% regression gate" and that
//! could not be built where the number already lived. The VM tier measures the
//! sandboxed engine (`checks.sh launcher-rss`), but the VM tier has no `push`
//! trigger — nightly, `workflow_dispatch`, and since the trigger change, `v*`
//! tags. Merging to `main` produces no tier run, so **there has never been a
//! per-commit series to regress against.**
//!
//! This is the Tier-1 half: a number on every PR and every commit, from a job
//! that already runs in under a minute.
//!
//! # What this is not
//!
//! **Not the §8.5 SLA.** That row says "< 30 MB" and §8.5 is explicit that
//! "the number users see is the sandboxed one" — measured inside the Flatpak.
//! This measures the bare binary on the runner, with no sandbox, no portal and
//! no compositor. The two numbers are not interchangeable and this one must
//! never be quoted as the SLA being met.
//!
//! What it is good for is the thing the sandboxed number cannot do: appear on
//! every commit, so a change that doubles the engine's resident set is visible
//! in the PR that caused it rather than in a nightly run nobody reads.
//!
//! # Why it reports rather than gates, for now
//!
//! ADR-0010: a threshold is measured before it is invented. A >5% regression
//! gate needs a stored baseline from `main`, which needs a series, which needs
//! this to have been running for a while. So this prints the figure and
//! asserts only a **sanity ceiling** — set well above the measured value, to
//! catch a leak or a runaway allocation rather than to police megabytes.
//!
//! Measured here, debug, four consecutive runs: **24 768 / 24 792 / 24 860 /
//! 24 844 kB** — a spread of 0.4%. That stability is the useful part: it says a
//! 5% regression gate would be signal rather than noise once there is a
//! baseline to compare against, which is not true of every number in §8.5.
//!
//! # The figure is only comparable within one build profile
//!
//! These are **debug** numbers, because `cargo nextest run --workspace` is what
//! CI runs and that is a debug build. A release engine will differ, so
//! comparing a release reading against this series would manufacture a
//! regression that is really a profile change. Whatever eventually stores the
//! baseline has to key it on the profile, and this is the note that says so
//! before someone finds out the hard way.
//!
//! No release figure is recorded here yet: measuring one needs a release build
//! of the workspace, and the machine this was written on ran out of disk
//! allowance before finishing it. Left unmeasured rather than estimated.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How many generated entries the engine indexes before being measured.
///
/// A real desktop is a few hundred; the Bluefin harvest is 757. Using a
/// realistic count rather than an empty index matters because an engine
/// measured with nothing loaded would report a number no user ever sees.
const ENTRIES: usize = 757;

fn binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("vicinae")
}

fn status_kb(pid: u32, field: &str) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let prefix = format!("{field}:");
    status
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

struct Engine {
    child: Child,
    _dirs: TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> (Engine, u32) {
    let dirs = TempDir::new().expect("tempdir");
    let data = dirs.path().join("data/applications");
    std::fs::create_dir_all(&data).expect("applications dir");

    for i in 0..ENTRIES {
        std::fs::write(
            data.join(format!("generated-{i}.desktop")),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Generated Application {i}\n\
                 Comment=A synthetic entry standing in for a real one\n\
                 Exec=/usr/bin/true --instance {i}\n\
                 Icon=application-x-executable\n"
            ),
        )
        .expect("write entry");
    }

    let socket = dirs.path().join("ipc.sock");
    let child = Command::new(binary())
        .arg("--socket")
        .arg(&socket)
        .arg("serve")
        .env("XDG_DATA_DIRS", dirs.path().join("data"))
        .env("XDG_DATA_HOME", dirs.path().join("data-home"))
        .env("XDG_CONFIG_HOME", dirs.path().join("config"))
        .env("HOME", dirs.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the engine");

    let pid = child.id();
    let engine = Engine { child, _dirs: dirs };

    // Measured only once it answers: a process that has not finished starting
    // has not finished allocating, and the number would depend on how fast the
    // runner happened to be.
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut listening = false;
    while Instant::now() < deadline {
        let answered = Command::new(binary())
            .arg("--socket")
            .arg(socket.as_path())
            .arg("ping")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if answered {
            listening = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        listening,
        "the engine never answered a ping, so whatever RSS it has is not an idle engine's"
    );

    (engine, pid)
}

#[test]
fn the_idle_engine_reports_its_resident_set() {
    if status_kb(std::process::id(), "VmRSS").is_none() {
        println!("skipping: /proc/self/status is unreadable, so this is not Linux");
        return;
    }

    let (engine, pid) = start();

    // Settle, so what is measured is a resting engine rather than one still
    // finishing its first index pass.
    std::thread::sleep(Duration::from_millis(500));

    let rss = status_kb(pid, "VmRSS").expect("the engine's VmRSS");
    let hwm = status_kb(pid, "VmHWM").expect("the engine's VmHWM");

    println!(
        "engine idle, {ENTRIES} entries indexed, UNSANDBOXED: VmRSS {rss} kB ({} MB), \
         peak VmHWM {hwm} kB ({} MB)",
        rss / 1024,
        hwm / 1024
    );
    println!(
        "  NOT the §8.5 SLA. That row is < 30 MB measured inside the Flatpak; this is the \
         bare binary on a runner. Per-commit trend signal only — see the module docs."
    );

    // A floor, for the reason every other measurement here has one: an
    // unreadable or already-dead process reports a small number, and a small
    // number passes a ceiling. The VM tier's RSS gate had exactly this defect,
    // passing on `0 kB`.
    assert!(
        rss > 1024,
        "the engine reported {rss} kB, which is not a running engine — the measurement is \
         what is wrong, not the engine"
    );

    // The sanity ceiling. Deliberately loose: its job is to catch a leak or a
    // runaway allocation, not to police megabytes. The real budget is the
    // sandboxed one in §8.5 and it is gated in the VM tier at 20 MB.
    const CEILING_KB: u64 = 150 * 1024;
    assert!(
        hwm < CEILING_KB,
        "the idle engine peaked at {hwm} kB, over the {CEILING_KB} kB sanity ceiling. This is \
         not the SLA — it is loose enough that crossing it means something is leaking or \
         allocating without bound, which is worth a look before the number is tightened"
    );

    drop(engine);
}
