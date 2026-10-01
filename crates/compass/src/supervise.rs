//! `compass start`: the launcher window as a child that is started again when
//! it fails.
//!
//! The launcher's event loop owns the UI's Wayland connection, and nothing in
//! the process survives that connection: a protocol error is fatal to it by
//! design (TIL-01 was one, from the clipboard), and so is a panic in a view.
//! `start` used to run the launcher in-process *and* own the engine it had
//! started, so either took the engine, its clipboard history and its indexer
//! down too, and nothing brought the launcher back.
//!
//! So `start` keeps the instance lease and the engine and runs the launcher as
//! `compass launcher-child`. A child that exits cleanly was asked to (the
//! engine went away, or the user quit), and `start` ends with it. One that
//! fails is started again, hidden, for the next summon, unless the engine has
//! gone or the launcher keeps failing, which no restart will fix.
//!
//! The child watches for this process going away and ends with it, so a
//! `start` that is killed does not leave a launcher behind ([`watch_parent`]).

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::cli::Cli;
use crate::session::EngineSession;
use crate::{EXIT_OK, session};

/// Restarts allowed within [`RESTART_WINDOW`] before `start` gives up.
pub const MAX_RESTARTS: usize = 5;
/// The window [`MAX_RESTARTS`] counts in.
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// A breath between a failure and the restart, so a compositor that is going
/// away is gone before the next launcher asks it for a surface.
const RESTART_DELAY: Duration = Duration::from_millis(300);
/// How often a child checks that `start` is still there.
const PARENT_POLL: Duration = Duration::from_secs(1);

/// The restarts made recently, to tell a launcher that failed once from one
/// that cannot run.
#[derive(Debug, Default)]
pub struct Restarts {
    recent: VecDeque<Instant>,
}

impl Restarts {
    /// Records a restart at `now` and says whether it is allowed: no more than
    /// [`MAX_RESTARTS`] within any [`RESTART_WINDOW`].
    pub fn allow(&mut self, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > RESTART_WINDOW)
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= MAX_RESTARTS {
            return false;
        }
        self.recent.push_back(now);
        true
    }
}

/// Runs the launcher as a child of this process until it ends on request,
/// starting it again when it fails. Owns `engine` and stops it on the way out,
/// as `start` always has.
///
/// # Errors
///
/// When the launcher cannot be spawned, when the engine has stopped, or when
/// the launcher failed [`MAX_RESTARTS`] times within [`RESTART_WINDOW`].
pub fn run(cli: &Cli, hidden: bool, mut engine: EngineSession) -> Result<ExitCode> {
    let exe = std::env::current_exe().context("finding the compass executable")?;
    let socket = cli.socket_path();
    let mut restarts = Restarts::default();
    let mut hidden = hidden;
    loop {
        let status = launcher(&exe, socket.as_path(), cli.verbose, hidden)
            .spawn()
            .context("starting the launcher")?
            .wait()
            .context("waiting for the launcher")?;
        if status.success() {
            return Ok(ExitCode::from(EXIT_OK));
        }
        if !engine.is_running() || !engine_answers(&socket)? {
            bail!("the launcher stopped ({status}) and the Compass engine is gone");
        }
        if !restarts.allow(Instant::now()) {
            bail!(
                "the launcher stopped {MAX_RESTARTS} times within {} seconds; not starting it \
                 again. Its errors are above",
                RESTART_WINDOW.as_secs()
            );
        }
        tracing::warn!(%status, "the launcher stopped; starting it again");
        // Back for the next summon rather than on screen out of nowhere.
        hidden = true;
        std::thread::sleep(RESTART_DELAY);
    }
}

/// The child's command line: the same executable, socket and verbosity.
fn launcher(exe: &Path, socket: &Path, verbose: u8, hidden: bool) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("--engine=rust")
        .arg("--socket")
        .arg(socket)
        .args(std::iter::repeat_n("-v", usize::from(verbose)))
        .arg("launcher-child")
        .args(hidden.then_some("--hidden"))
        .stdin(Stdio::null());
    command
}

fn engine_answers(socket: &compass_ipc::SocketPath) -> Result<bool> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), session::listening(socket))
            .await
            .is_ok_and(|answer| answer.unwrap_or(false))
    }))
}

/// In a child: ends the process once `start`, its parent, has gone, which
/// reparents it. `start` stopped by a signal runs no destructor and so cannot
/// stop the child itself.
pub fn watch_parent() {
    let parent = std::os::unix::process::parent_id();
    let spawned = std::thread::Builder::new()
        .name("parent-watch".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(PARENT_POLL);
                if std::os::unix::process::parent_id() != parent {
                    tracing::info!("compass start has gone; the launcher is ending with it");
                    std::process::exit(i32::from(EXIT_OK));
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot watch for compass start going away");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[test]
    fn a_launcher_that_keeps_failing_is_given_up_on() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        for n in 0..MAX_RESTARTS {
            assert!(restarts.allow(start + Duration::from_secs(n as u64)));
        }
        assert!(!restarts.allow(start + Duration::from_secs(10)));
    }

    #[test]
    fn failures_spread_over_time_are_each_restarted() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        for n in 0..(MAX_RESTARTS * 3) {
            let at = start + RESTART_WINDOW.mul_f32(0.5) * n as u32;
            assert!(restarts.allow(at), "restart {n}");
        }
    }

    #[test]
    fn the_child_runs_under_the_same_socket_hidden_after_a_failure() {
        let command = launcher(Path::new("/bin/compass"), Path::new("/run/c.sock"), 2, true);
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--engine=rust",
                "--socket",
                "/run/c.sock",
                "-v",
                "-v",
                "launcher-child",
                "--hidden"
            ]
        );
        assert!(
            crate::Cli::try_parse_from(
                std::iter::once("compass".to_owned()).chain(args.iter().cloned())
            )
            .is_ok_and(|cli| cli.command == crate::Command::LauncherChild { hidden: true })
        );
    }
}
