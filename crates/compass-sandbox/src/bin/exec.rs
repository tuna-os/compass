//! Applies a Landlock policy to itself, then becomes the program it was given.
//!
//! This exists because a Landlock ruleset confines the process that applies it.
//! Confining a child otherwise means running code between fork and exec, which
//! is `pre_exec` and is `unsafe`; this workspace forbids `unsafe`. Here the
//! restriction and the `exec` happen in the same process, and Landlock
//! guarantees the restriction survives `execve`.
//!
//! ```text
//! compass-sandbox-exec --read /usr/share --write /tmp/w -- /usr/bin/node worker.js
//! ```
//!
//! The syscall filter goes on after the filesystem boundary. With today's
//! denylist the order makes no difference -- swapping the two changes no test
//! -- because `landlock_*` is not on the list and everything unnamed is
//! allowed. It is written this way for the day the filter becomes an
//! allowlist, when a filter installed first would have to name the three
//! Landlock syscalls or refuse the boundary it is meant to reinforce.
//!
//! `--no-syscall-filter` leaves it off, which is for measuring what the filter
//! costs, not for running an extension.
//!
//! Anything after `--` is the program and its arguments, so a worker whose own
//! flags happen to be spelled like these is not misread.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

/// Refused the arguments.
const EXIT_USAGE: u8 = 64;
/// Could not apply the policy — including a kernel that would not enforce it.
const EXIT_SANDBOX: u8 = 65;
/// Could not exec the program.
const EXIT_EXEC: u8 = 66;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let (policy, program) = match compass_sandbox::parse_args(&args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("compass-sandbox-exec: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let Some((program, program_args)) = program.split_first() else {
        eprintln!("compass-sandbox-exec: nothing to run after `--`");
        return ExitCode::from(EXIT_USAGE);
    };

    // Deliberately before the exec and after nothing: every path the policy
    // names has already been checked to exist, and from here this process is
    // confined whatever happens next.
    if let Err(error) = policy.apply().and_then(|_| policy.apply_limits()) {
        eprintln!("compass-sandbox-exec: {error}");
        return ExitCode::from(EXIT_SANDBOX);
    }

    // After Landlock -- see the module docs for why the order is written this
    // way even though it does not bite yet.
    if policy.syscall_filter
        && let Err(error) =
            compass_sandbox::syscalls::deny(&compass_sandbox::syscalls::default_numbers())
    {
        eprintln!("compass-sandbox-exec: {error}");
        return ExitCode::from(EXIT_SANDBOX);
    }

    // `exec` only returns on failure.
    let error = Command::new(program).args(program_args).exec();
    eprintln!("compass-sandbox-exec: could not run {program}: {error}");
    ExitCode::from(EXIT_EXEC)
}
