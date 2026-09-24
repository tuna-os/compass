//! Who owns a worker process across its whole life: start it, pump it,
//! restart it after it dies.
//!
//! [`Session`] already routes one worker's turns; what it cannot do is
//! survive the worker. A dead session borrows a dead process, and building a
//! new one needs a new worker plus a new router, so every owner would
//! reinvent the same loop: ensure, pump, and on [`Turn::Closed`](crate::session::Turn::Closed)
//! or [`Turn::Crashed`](crate::session::Turn::Crashed) decide whether to
//! restart. This is that loop, owned in one place.
//!
//! The C++ manager reports the crash and leaves the process dead —
//! extensions stay broken until something restarts them — so restart here is
//! explicit, never automatic: [`restart`](Supervisor::restart) shuts the
//! session down, and the next [`ensure_running`](Supervisor::ensure_running)
//! starts a fresh worker. An owner that wants the C++ behavior stops
//! pumping; one that wants a self-healing worker restarts and keeps going,
//! and either way every terminal turn is observed exactly once.
//!
//! The router is rebuilt on every start because [`Router`] borrows its
//! services: they live with the owner, which hands a fresh router over each
//! time, the way the services outlive any one session.

use crate::session::{Router, Session};
use crate::{Worker, WorkerError};

/// Owns one worker's lifetimes: the current [`Session`] plus the count of
/// starts behind it.
pub struct Supervisor<'a> {
    session_id: String,
    spawn: Box<dyn FnMut() -> Result<Worker, WorkerError> + Send + 'a>,
    session: Option<Session<'a>>,
    starts: u64,
}

impl std::fmt::Debug for Supervisor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("session_id", &self.session_id)
            .field("running", &self.is_running())
            .field("starts", &self.starts)
            .finish()
    }
}

impl<'a> Supervisor<'a> {
    /// Stages a supervisor: nothing runs until [`ensure_running`](Self::ensure_running).
    ///
    /// `spawn` builds one worker — a plain [`Worker::spawn`] command or a
    /// sandboxed one — and may fail, in which case the failure is reported
    /// every time instead of cached.
    pub fn new(
        session_id: impl Into<String>,
        spawn: impl FnMut() -> Result<Worker, WorkerError> + Send + 'a,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            spawn: Box::new(spawn),
            session: None,
            starts: 0,
        }
    }

    /// Starts the worker when none runs, handing it `router`.
    ///
    /// Routers borrow their services, so the owner keeps the services and
    /// builds a router per start; a running session is left alone, router
    /// included.
    ///
    /// # Errors
    ///
    /// Whatever the spawn closure fails with.
    pub fn ensure_running(&mut self, router: Router<'a>) -> Result<(), WorkerError> {
        if self.session.is_none() {
            let worker = (self.spawn)()?;
            self.session = Some(Session::new(worker, &self.session_id, router));
            self.starts += 1;
        }
        Ok(())
    }

    /// The live session, if one runs.
    #[must_use]
    pub fn session_mut(&mut self) -> Option<&mut Session<'a>> {
        self.session.as_mut()
    }

    /// Whether a session currently runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    /// How many workers have been started, restarts included.
    #[must_use]
    pub fn starts(&self) -> u64 {
        self.starts
    }

    /// Shuts the live session down and forgets it; the next
    /// [`ensure_running`](Self::ensure_running) starts a fresh worker.
    ///
    /// Call this after a terminal [`Turn`](crate::session::Turn) to restart,
    /// or anytime to deliberately cycle the worker. Restarting is polite —
    /// the session
    /// goes through [`Session::shutdown`], never a bare drop, because a
    /// dropped [`Worker`] would outlive its host unreaped.
    ///
    /// # Errors
    ///
    /// Whatever [`Session::shutdown`] can fail with; the session is gone
    /// either way.
    pub fn restart(&mut self) -> Result<(), WorkerError> {
        match self.session.take() {
            Some(session) => session.shutdown().map(|_| ()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Turn;
    use std::process::Command;

    fn supervisor() -> Supervisor<'static> {
        Supervisor::new("s", || Worker::spawn(Command::new("cat")))
    }

    #[test]
    fn ensure_starts_once_and_restart_cycles_the_worker() {
        let mut supervisor = supervisor();
        assert!(!supervisor.is_running());
        assert_eq!(supervisor.starts(), 0);

        supervisor.ensure_running(Router::new()).expect("spawn cat");
        supervisor
            .ensure_running(Router::new())
            .expect("second ensure is free");
        assert!(supervisor.is_running());
        assert_eq!(supervisor.starts(), 1);

        // A full turn through a real pipe: the request echo reads back as
        // a response, which is nobody's event and therefore nothing.
        let session = supervisor.session_mut().expect("session");
        session
            .worker_mut()
            .request("Manager/load", serde_json::json!({}))
            .expect("request");
        match session.pump_once().expect("pump") {
            Turn::Nothing => {}
            turn => panic!("the echoed response is nothing, got {turn:?}"),
        }

        // Kill the worker: the next pump reports the death...
        supervisor
            .session_mut()
            .expect("session")
            .worker_mut()
            .kill()
            .expect("kill");
        match supervisor
            .session_mut()
            .expect("session")
            .pump_once()
            .expect("pump")
        {
            Turn::Closed => {}
            turn => panic!("a dead worker reads closed, got {turn:?}"),
        }

        // ...and restart plus ensure brings a fresh one; the generation
        // count says so.
        supervisor.restart().expect("restart");
        assert!(!supervisor.is_running());
        supervisor.ensure_running(Router::new()).expect("respawn");
        assert_eq!(supervisor.starts(), 2);
        supervisor.restart().expect("final restart");
    }

    #[test]
    fn restarting_idle_is_free() {
        let mut supervisor = supervisor();
        supervisor.restart().expect("nothing to restart");
        assert_eq!(supervisor.starts(), 0);
    }
}
