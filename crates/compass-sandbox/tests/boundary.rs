//! Does the boundary actually bite?
//!
//! Every negative test here has a positive control beside it, because the
//! dangerous outcome is not "the sandbox blocked too much" but "the sandbox
//! blocked nothing and every log said it applied". A denial only means
//! something if the identical operation succeeds without the policy.
//!
//! These run the real launcher against real processes. `compass-sandbox-exec`
//! confines the process it runs in for good, so it cannot be exercised
//! in-process: a test that applied a policy would confine the test binary and
//! every test after it.

use std::path::{Path, PathBuf};
use std::process::Command;

use compass_sandbox::{Mode, Policy};

/// The launcher, as cargo built it for this test.
fn launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_compass-sandbox-exec"))
}

/// Somewhere to read from, and something outside it.
struct Fixture {
    _dir: tempfile::TempDir,
    allowed: PathBuf,
    inside: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).expect("the allowed directory");
    let inside = allowed.join("inside.txt");
    std::fs::write(&inside, b"inside\n").expect("the inside file");
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"outside\n").expect("the outside file");
    Fixture {
        _dir: dir,
        allowed,
        inside,
        outside,
    }
}

/// `cat`, wherever it is on this system.
fn cat() -> PathBuf {
    for candidate in ["/usr/bin/cat", "/bin/cat"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    panic!("no cat on this system; these tests need a real program to exec");
}

/// Whether this kernel enforces Landlock at all.
///
/// Not a skip switch. On a kernel without Landlock a strict policy *must* fail
/// closed, and that is what the tests below assert instead of asserting a
/// denial they cannot produce — the property worth checking on such a machine
/// is that nothing runs unconfined, not that reads are blocked by a feature
/// that is not there.
fn landlock_enforces() -> bool {
    let f = fixture();
    let (ok, stderr) = run(
        &runnable().read(&f.allowed),
        &cat(),
        &[f.inside.to_string_lossy().into_owned()],
    );
    if ok {
        return true;
    }
    assert!(
        stderr.contains("enforced") || stderr.contains("Landlock"),
        "the launcher failed for a reason that is not the kernel: {stderr}"
    );
    eprintln!(
        "note: this kernel does not enforce Landlock ({stderr}); the boundary tests assert \
         fail-closed instead of denial"
    );
    false
}

/// Asserts that a strict policy refuses to run anything here.
fn assert_fails_closed() {
    let f = fixture();
    let (ok, _) = run(
        &runnable().read(&f.allowed),
        &cat(),
        &[f.inside.to_string_lossy().into_owned()],
    );
    assert!(
        !ok,
        "a strict policy ran a program on a kernel that does not enforce it"
    );
}

/// Runs `program` behind `policy` and returns (success, stderr).
fn run(policy: &Policy, program: &Path, args: &[String]) -> (bool, String) {
    let output = policy
        .command(&launcher(), program, args)
        .output()
        .expect("the launcher runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The policy a `cat` needs in order to run at all: the binary and the
/// libraries it loads.
fn can_run_cat() -> Policy {
    Policy::new()
        .read("/usr")
        .read("/lib")
        .read("/lib64")
        .execute("/usr")
        .execute("/lib")
        .execute("/lib64")
}

/// As [`can_run_cat`], dropping paths this system does not have.
fn runnable() -> Policy {
    let mut policy = Policy::new();
    for path in ["/usr", "/lib", "/lib64", "/bin"] {
        if Path::new(path).exists() {
            policy = policy.read(path).execute(path);
        }
    }
    policy
}

#[test]
fn the_control_passes_without_a_policy_at_all() {
    // If this fails, every denial below is meaningless: it would mean `cat`
    // cannot read the file for reasons that have nothing to do with Landlock.
    let f = fixture();
    let plain = Command::new(cat())
        .arg(&f.outside)
        .output()
        .expect("cat runs");
    assert!(
        plain.status.success(),
        "the unsandboxed control failed: {}",
        String::from_utf8_lossy(&plain.stderr)
    );
}

#[test]
fn a_read_inside_the_policy_is_allowed() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    let f = fixture();
    let policy = runnable().read(&f.allowed);
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(
        ok,
        "a read of an allowed path was refused; the policy is too tight to prove anything: \
         {stderr}"
    );
}

#[test]
fn a_read_outside_the_policy_is_denied() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // The assertion. Same launcher, same `cat`, same fixture as the test
    // above; the only difference is which path is being read.
    let f = fixture();
    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(&policy, &cat(), &[f.outside.to_string_lossy().into_owned()]);
    assert!(
        !ok,
        "a read outside the policy succeeded: the ruleset applied and confined nothing"
    );
}

#[test]
fn a_write_to_a_read_only_path_is_denied() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // `read` grants reading, not writing. `sh -c` is used rather than `cat`
    // because a redirect is the shortest way to ask for a write.
    let f = fixture();
    let Some(sh) = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    else {
        panic!("no sh on this system");
    };

    let target = f.allowed.join("new.txt");
    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("echo x > {}", target.to_string_lossy()),
        ],
    );
    assert!(!ok, "a write to a read-only path succeeded");
    assert!(!target.exists(), "the file was created anyway");

    // Control: the same write, with the same everything, where the policy
    // grants writing.
    let policy = runnable().write(&f.allowed);
    let (ok, stderr) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("echo x > {}", target.to_string_lossy()),
        ],
    );
    assert!(ok, "the write control failed: {stderr}");
    assert!(target.exists(), "the write control wrote nothing");
}

#[test]
fn a_program_the_policy_does_not_allow_cannot_be_run() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // Execute is separate from read on purpose: a worker that may read its
    // own directory is not thereby allowed to run anything in it.
    let f = fixture();
    let policy = Policy::new().read(&f.allowed);
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(
        !ok,
        "a program outside the policy's execute paths was run anyway: {stderr}"
    );
}

#[test]
fn a_policy_that_names_a_missing_path_refuses_before_running_anything() {
    // No kernel branch: the check happens before the ruleset is built, so it
    // holds whether or not Landlock is enforced here.
    // Otherwise the rule would contribute nothing and the policy would read as
    // granting access it does not grant.
    let f = fixture();
    let policy = runnable().read(&f.allowed).read("/definitely/not/here");
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(!ok, "a policy naming a missing path was applied anyway");
    assert!(
        stderr.contains("/definitely/not/here"),
        "the refusal must name the path: {stderr}"
    );
}

#[test]
fn nothing_after_the_separator_is_refused_rather_than_silently_doing_nothing() {
    let output = Command::new(launcher())
        .args(["--read", "/usr", "--"])
        .output()
        .expect("the launcher runs");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("nothing to run"),
        "the launcher should say what was wrong"
    );
}

#[test]
fn an_unknown_flag_is_refused() {
    let output = Command::new(launcher())
        .args(["--allow-everything", "--", "/bin/true"])
        .output()
        .expect("the launcher runs");
    assert!(
        !output.status.success(),
        "an unknown flag must not be ignored: a policy that quietly skips one is weaker than \
         it reads"
    );
}

#[test]
fn the_policy_applies_to_a_grandchild_too() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // Landlock is inherited, and the worker will spawn things. `sh -c` runs
    // `cat` as its own child; the boundary has to hold there as well, or
    // every confinement is one `execve` deep.
    let f = fixture();
    let Some(sh) = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    else {
        panic!("no sh on this system");
    };

    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("cat {}", f.outside.to_string_lossy()),
        ],
    );
    assert!(!ok, "a grandchild read a path the policy denies");

    let (ok, stderr) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("cat {}", f.inside.to_string_lossy()),
        ],
    );
    assert!(ok, "the grandchild control failed: {stderr}");
}

#[test]
fn best_effort_is_not_the_default() {
    // A strict policy on a kernel that will not enforce it fails; this test
    // cannot force that kernel, so what it checks is that the flag has to be
    // asked for, in the arguments that actually reach the launcher.
    let strict = can_run_cat();
    assert_eq!(strict.mode, Mode::Strict);
    assert!(!strict.args().contains(&"--best-effort".to_owned()));
    assert!(
        strict
            .mode(Mode::BestEffort)
            .args()
            .contains(&"--best-effort".to_owned())
    );
}
