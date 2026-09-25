//! Running `compass-input-server` and talking to it.
//!
//! `LinuxInputServer` (`src/server/src/services/input-server/`): the helper
//! is started when `input_server.enabled` is on and stopped when it is
//! turned off, restarted after a crash with
//! [`compass_core::input_server::RestartPolicy`]'s doubling backoff, and
//! spoken to in figura's JSON-RPC over its stdin and stdout
//! ([`compass_core::input_server::wire`]). Its stderr goes to the engine's
//! log, line by line.
//!
//! The helper is found as the C++ finds it: `$COMPASS_INPUT_SERVER_BIN`
//! first (how NixOS points at the capability-wrapped copy under
//! `/run/wrappers`; the C++'s `$VICINAE_INPUT_SERVER_BIN` is still read as a
//! fallback), then beside the engine and in `../libexec/compass`.
//! Inside a Flatpak it is not started at all: the sandbox has no
//! `/dev/input` and no `/dev/uinput`, and no manifest permission grants them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_core::input_server::wire::{Call, Event, ServerMessage};
use compass_core::input_server::{CrashAction, MessageBuffer, RestartPolicy, frame};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, watch};

/// The helper's file name.
pub const HELPER_PROGRAM: &str = "compass-input-server";

/// The variable that names the helper outright.
pub const HELPER_ENV: &str = "COMPASS_INPUT_SERVER_BIN";

/// How long a call waits for its answer before it is given up on.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// What the helper tells the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// It started and answered `getCapabilities`: keywords must be
    /// registered (again).
    Ready,
    /// A keyword was typed.
    Trigger(String),
    /// Backspace straight after this keyword expanded.
    Undo(String),
}

/// The helper, as the rest of the engine sees it.
#[derive(Debug, Clone)]
pub struct InputServer {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    /// Whether it is wanted; the supervisor follows this.
    wanted: watch::Sender<bool>,
    /// The running helper's stdin, while there is one.
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// Calls waiting for their answer.
    pending: Mutex<HashMap<i32, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicI32,
    /// Set when the running helper answered its readiness probe.
    answered: AtomicBool,
    status: Mutex<Status>,
    notices: mpsc::UnboundedSender<Notice>,
}

/// What the helper is doing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Up and answered `getCapabilities`.
    pub running: bool,
    /// Its virtual keyboard exists.
    pub injection: bool,
    /// The binary in use.
    pub helper: Option<PathBuf>,
    /// Why it is not running, or not typing.
    pub problem: Option<String>,
}

/// Whether this process is inside a Flatpak sandbox.
#[must_use]
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Where the helper is: `$COMPASS_INPUT_SERVER_BIN`, else beside the engine
/// or in its `libexec`.
#[must_use]
pub fn find_helper() -> Option<PathBuf> {
    if let Some(path) = compass_xdg::brand::env_var_os(HELPER_ENV).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    crate::indexer_client::find_helper_program(HELPER_PROGRAM)
}

impl InputServer {
    /// A helper that is not running yet, and where its notices arrive.
    ///
    /// Must be called inside a tokio runtime: the supervisor is spawned
    /// here and runs for the life of the engine, starting and stopping the
    /// helper as [`Self::set_enabled`] says.
    #[must_use]
    pub fn start(helper: Option<PathBuf>) -> (Self, mpsc::UnboundedReceiver<Notice>) {
        let (wanted, wanted_rx) = watch::channel(false);
        let (notices, notices_rx) = mpsc::unbounded_channel();
        let server = Self {
            shared: Arc::new(Shared {
                wanted,
                stdin: tokio::sync::Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                next_id: AtomicI32::new(1),
                answered: AtomicBool::new(false),
                status: Mutex::new(Status {
                    helper: helper.clone(),
                    ..Status::default()
                }),
                notices,
            }),
        };
        tokio::spawn(supervise(Arc::clone(&server.shared), helper, wanted_rx));
        (server, notices_rx)
    }

    /// Wants the helper running, or not (`LinuxInputServer::setEnabled`).
    pub fn set_enabled(&self, enabled: bool) {
        self.shared.wanted.send_if_modified(|wanted| {
            let changed = *wanted != enabled;
            *wanted = enabled;
            changed
        });
    }

    /// Whether the helper is wanted.
    #[must_use]
    pub fn enabled(&self) -> bool {
        *self.shared.wanted.borrow()
    }

    /// What it is doing.
    #[must_use]
    pub fn status(&self) -> Status {
        self.shared
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Makes `call` and waits for its answer. `None` when the helper is not
    /// running (as the C++ drops calls then) or did not answer in time.
    pub async fn call(&self, call: Call) -> Option<Result<Value, String>> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);
        if !self.shared.write(&call.encode(id)).await {
            self.shared.forget(id);
            return None;
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(answer)) => Some(answer),
            _ => {
                self.shared.forget(id);
                tracing::warn!(method = call.method(), "the input server did not answer");
                None
            }
        }
    }
}

impl Shared {
    /// Writes one framed message; false when there is no helper to take it.
    async fn write(&self, payload: &[u8]) -> bool {
        let mut stdin = self.stdin.lock().await;
        let Some(pipe) = stdin.as_mut() else {
            return false;
        };
        let framed = frame(payload);
        match pipe.write_all(&framed).await {
            Ok(()) => pipe.flush().await.is_ok(),
            Err(error) => {
                tracing::debug!(%error, "the input server's stdin is closed");
                *stdin = None;
                false
            }
        }
    }

    fn forget(&self, id: i32) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
    }

    fn update(&self, change: impl FnOnce(&mut Status)) {
        change(
            &mut self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    /// Answers every waiting call with an error: the helper went away.
    fn fail_pending(&self) {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, tx) in pending {
            let _ = tx.send(Err("the input server exited".to_owned()));
        }
    }
}

/// Starts the helper whenever it is wanted and not running, and restarts it
/// after a crash until the policy gives up.
async fn supervise(
    shared: Arc<Shared>,
    helper: Option<PathBuf>,
    mut wanted: watch::Receiver<bool>,
) {
    let mut policy = RestartPolicy::new();
    loop {
        // Wait until it is wanted.
        while !*wanted.borrow_and_update() {
            policy.set_enabled(false);
            if wanted.changed().await.is_err() {
                return;
            }
        }
        policy.set_enabled(true);

        if in_flatpak() {
            shared.update(|status| {
                status.problem = Some(
                    "inside a Flatpak: the sandbox has no /dev/input or /dev/uinput, so \
                     snippet keywords cannot expand"
                        .to_owned(),
                );
            });
            tracing::warn!("not starting the input server inside a Flatpak");
            if wanted.changed().await.is_err() {
                return;
            }
            continue;
        }
        let Some(program) = helper.clone() else {
            shared.update(|status| {
                status.problem = Some(format!(
                    "{HELPER_PROGRAM} is not installed beside the engine; snippet keywords \
                     will not expand"
                ));
            });
            tracing::warn!("could not find {HELPER_PROGRAM}; snippet expansion will not work");
            if wanted.changed().await.is_err() {
                return;
            }
            continue;
        };

        shared.answered.store(false, Ordering::Relaxed);
        let outcome = run_once(&shared, &program, &mut wanted).await;
        shared.update(|status| {
            status.running = false;
            status.injection = false;
        });
        shared.fail_pending();
        // One that came up gets its attempts back, as `m_crashCount = 0` on
        // the capabilities answer.
        if shared.answered.load(Ordering::Relaxed) {
            policy.ready();
        }
        match outcome {
            Outcome::Stopped => continue,
            Outcome::Exited => {
                // A clean exit is not restarted; it runs again after an off
                // and an on.
                until_unwanted(&mut wanted).await;
                continue;
            }
            Outcome::Crashed => {}
        }
        match policy.handle_crash() {
            CrashAction::Ignore => {}
            CrashAction::RestartAfter(delay) => {
                tracing::warn!(
                    delay_ms = delay,
                    attempt = policy.crash_count(),
                    "the input server crashed; restarting"
                );
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(delay)) => {}
                    changed = wanted.changed() => if changed.is_err() { return },
                }
            }
            CrashAction::GiveUp => {
                let message = format!(
                    "the input server crashed {} times; not restarting it until it is \
                     turned off and on again",
                    policy.crash_count()
                );
                tracing::error!("{message}");
                shared.update(|status| status.problem = Some(message));
                // Wait for an off and an on.
                while *wanted.borrow_and_update() {
                    if wanted.changed().await.is_err() {
                        return;
                    }
                }
                policy.ready();
            }
        }
    }
}

/// How one run of the helper ended.
enum Outcome {
    /// It was turned off.
    Stopped,
    /// It exited on its own, cleanly.
    Exited,
    /// It failed to start, or exited abnormally.
    Crashed,
}

/// Runs the helper until it exits or is no longer wanted.
async fn run_once(
    shared: &Arc<Shared>,
    program: &std::path::Path,
    wanted: &mut watch::Receiver<bool>,
) -> Outcome {
    let mut child = match Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let message = format!("could not start {}: {error}", program.display());
            tracing::error!("{message}");
            shared.update(|status| status.problem = Some(message));
            return Outcome::Crashed;
        }
    };
    tracing::info!(helper = %program.display(), "started the input server");

    if let Some(stderr) = child.stderr.take() {
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "compass::input_server::helper", "{line}");
                if line.contains("/dev/uinput") {
                    shared.update(|status| status.problem = Some(line.trim().to_owned()));
                }
            }
        });
    }
    *shared.stdin.lock().await = child.stdin.take();
    let Some(stdout) = child.stdout.take() else {
        return Outcome::Crashed;
    };
    let reader = tokio::spawn(read_messages(Arc::clone(shared), stdout));

    // `getCapabilities` doubles as the readiness probe, as in the C++.
    {
        let server = InputServer {
            shared: Arc::clone(shared),
        };
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            if let Some(Ok(caps)) = server.call(Call::GetCapabilities).await {
                let injection = caps.get("injection").and_then(Value::as_bool) == Some(true);
                shared.update(|status| {
                    status.running = true;
                    status.injection = injection;
                    if injection {
                        status.problem = None;
                    } else if status.problem.is_none() {
                        status.problem = Some(
                            "the input server cannot create a virtual keyboard; keywords are \
                             detected but nothing is typed"
                                .to_owned(),
                        );
                    }
                });
                shared.answered.store(true, Ordering::Relaxed);
                let _ = shared.notices.send(Notice::Ready);
            }
        });
    }

    let outcome = tokio::select! {
        status = child.wait() => {
            match status {
                Ok(status) if status.success() => {
                    tracing::info!("the input server exited");
                    Outcome::Exited
                }
                Ok(status) => {
                    tracing::warn!(%status, "the input server exited abnormally");
                    Outcome::Crashed
                }
                Err(error) => {
                    tracing::warn!(%error, "lost the input server");
                    Outcome::Crashed
                }
            }
        }
        () = until_unwanted(wanted) => {
            stop(shared, &mut child).await;
            Outcome::Stopped
        }
    };
    *shared.stdin.lock().await = None;
    reader.abort();
    outcome
}

async fn until_unwanted(wanted: &mut watch::Receiver<bool>) {
    loop {
        if !*wanted.borrow_and_update() {
            return;
        }
        if wanted.changed().await.is_err() {
            return;
        }
    }
}

/// Closes stdin, which the helper takes as its cue to exit, and kills it if
/// it has not within a second.
async fn stop(shared: &Shared, child: &mut Child) {
    *shared.stdin.lock().await = None;
    if tokio::time::timeout(Duration::from_secs(1), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    tracing::info!("stopped the input server");
}

async fn read_messages(shared: Arc<Shared>, mut stdout: tokio::process::ChildStdout) {
    let mut buffer = MessageBuffer::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        for message in buffer.push(&chunk[..read]) {
            match ServerMessage::decode(&message) {
                Ok(ServerMessage::Reply { id, result }) => {
                    let waiting = shared
                        .pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&id);
                    if let Some(tx) = waiting {
                        let _ = tx.send(result);
                    }
                }
                Ok(ServerMessage::Event(Event::Trigger(trigger))) => {
                    let _ = shared.notices.send(Notice::Trigger(trigger));
                }
                Ok(ServerMessage::Event(Event::Undo(trigger))) => {
                    let _ = shared.notices.send(Notice::Undo(trigger));
                }
                Ok(ServerMessage::Unknown(method)) => {
                    tracing::debug!(method, "an input server message this build ignores");
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to route an input server message");
                }
            }
        }
    }
}
