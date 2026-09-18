//! Calls one denied syscall and says what the kernel answered.
//!
//! The boundary tests need a program that issues a syscall on the denylist on
//! purpose. Nothing on a stock system does that reliably: `unshare` needs
//! namespace privileges the machine may not give (a GitHub runner does not),
//! and `strace` is not installed everywhere. So this is that program.
//!
//! It asks `ptrace(PTRACE_ATTACH, 0)`. Process 0 does not exist, so the answer
//! without a filter is `ESRCH` — *the call went through and the kernel refused
//! the argument*. Behind the filter the answer is `EPERM`, before the kernel
//! ever looks at the argument. Two different errnos from one call, needing no
//! privilege, no namespace and no second process.
//!
//! It exits 0 either way and prints `errno=<name>`: the test reads the name,
//! because "the program failed" is exactly the ambiguity being avoided.

use nix::sys::ptrace;
use nix::unistd::Pid;

fn main() {
    // `Pid::from_raw(0)` is the "attach to pid 0" that cannot succeed.
    let answer = match ptrace::attach(Pid::from_raw(0)) {
        Ok(()) => "ok".to_owned(),
        Err(errno) => format!("{errno:?}"),
    };
    println!("errno={answer}");
}
