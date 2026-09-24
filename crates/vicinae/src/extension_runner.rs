//! Runs installed extensions' `no-view` commands.
//!
//! The first slice of the extension host in the engine (Phase 4, #7). A run
//! starts the extension runtime (the bundle the C++ engine ships as
//! `vicinae-worker-ts`) under Node, loads the command and handshakes, and then
//! serves the session on a thread of its own:
//! - local storage, from Compass's own encrypted database;
//! - HUDs, failure toasts and notifications, as desktop notifications;
//! - alerts, answered "no", because nothing can show one yet.
//!
//! What it does not do yet, and says so rather than failing obscurely:
//! - draw a `view` command (there is no view renderer in the launcher);
//! - collect preferences (a required one without a default is refused by
//!   name).
//!
//! # When a run is over
//!
//! The runtime never tells the host that a `no-view` command finished: it
//! unloads the command's worker itself and says nothing (the C++ has a FIXME
//! for exactly this). So a run ends when the runtime has been quiet for
//! [`IDLE`], or at [`LIFETIME`] regardless, and the runtime process is then
//! stopped. One runtime per run, so a stuck command cannot hold up the next.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use compass_core::alert::Alert;
use compass_core::extension_commands::ExtensionCommand;
use compass_core::manifest::CommandMode;
use compass_worker_host::Worker;
use compass_worker_host::extension_manager::{
    Capabilities, CommandEnv, LaunchType, LoadOptions, ManagerClient,
};
use compass_worker_host::session::{Router, Session, Turn};
use compass_worker_host::storage_service::StorageService;
use compass_worker_host::tsapi::Deferral;
use compass_worker_host::ui_shell_service::{
    CloseWindow, CommandInfo, Notification, Shell, ToastStyle, UiShellService,
};

/// Overrides where the runtime bundle is looked for.
pub const RUNTIME_ENV: &str = "COMPASS_EXTENSION_RUNTIME";

/// Overrides which Node runs it.
pub const NODE_ENV: &str = "COMPASS_NODE";

/// Overrides where `compass-sandbox-exec` is looked for.
pub const SANDBOX_EXEC_ENV: &str = "COMPASS_SANDBOX_EXEC";

/// Set to `off` to run extensions unconfined, for development on a machine
/// without the launcher. Anything else, or unset, means confined or refused.
pub const SANDBOX_SWITCH_ENV: &str = "COMPASS_EXTENSION_SANDBOX";

/// The sandbox launcher's file name.
pub const SANDBOX_EXEC_NAME: &str = "compass-sandbox-exec";

/// The bundle's file name wherever it is installed.
pub const RUNTIME_FILE_NAME: &str = "extension-runtime.js";

/// Quiet this long after a command's last message, and it is taken as done.
pub const IDLE: Duration = Duration::from_secs(10);

/// No `no-view` run lives longer than this.
pub const LIFETIME: Duration = Duration::from_secs(300);

/// Compass's own database for extensions' local storage (ADR-0017: Compass
/// owns its files; Vicinae's `vicinae.db` is never opened).
pub const STORAGE_DATABASE: &str = "compass-extension-storage.db";

/// How long the runtime has to answer `load`.
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// What runs extensions: Node and the runtime bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    /// The Node executable.
    pub node: PathBuf,
    /// The runtime bundle it runs.
    pub bundle: PathBuf,
    /// `compass-sandbox-exec`, which confines the runtime before it starts;
    /// `None` only when [`SANDBOX_SWITCH_ENV`] is `off`.
    pub sandbox: Option<PathBuf>,
}

impl Runtime {
    /// Finds Node and the bundle: the environment overrides first, then an
    /// installed bundle beside the executable (`../share/vicinae/`) or in the
    /// Flatpak's `/app/share/vicinae/`, and Node on `PATH`.
    ///
    /// # Errors
    ///
    /// A sentence naming what is missing, for the launcher to show.
    pub fn locate() -> Result<Self, String> {
        let bundle = std::env::var_os(RUNTIME_ENV)
            .map(PathBuf::from)
            .or_else(installed_bundle)
            .filter(|path| path.is_file())
            .ok_or_else(|| {
                "Running extensions needs the extension runtime, which this install of \
                 Compass does not include"
                    .to_owned()
            })?;
        let node = std::env::var_os(NODE_ENV)
            .map(PathBuf::from)
            .or_else(|| on_path("node"))
            .ok_or_else(|| "Running extensions needs Node.js, and none was found".to_owned())?;
        let sandbox = if std::env::var_os(SANDBOX_SWITCH_ENV).is_some_and(|v| v == "off") {
            tracing::warn!("{SANDBOX_SWITCH_ENV}=off: extensions run unconfined");
            None
        } else {
            Some(
                std::env::var_os(SANDBOX_EXEC_ENV)
                    .map(PathBuf::from)
                    .or_else(installed_sandbox)
                    .filter(|path| path.is_file())
                    .ok_or_else(|| {
                        "Running extensions needs compass-sandbox-exec, which this install of \
                         Compass does not include, and Compass will not run them unconfined"
                            .to_owned()
                    })?,
            )
        };
        Ok(Self {
            node,
            bundle,
            sandbox,
        })
    }
}

fn installed_sandbox() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(SANDBOX_EXEC_NAME)));
    beside_exe
        .into_iter()
        .chain([Path::new("/app/libexec").join(SANDBOX_EXEC_NAME)])
        .find(|path| path.is_file())
}

/// What the runtime may touch while it runs `command`.
///
/// Read: the system (Node's libraries, certificates, ICU data, `/proc`), Node,
/// the bundle and the extension's own directory. Write: only the support and
/// asset directories the runtime creates for this extension. Not `/tmp`:
/// every other process's temporary files are there, so the extension gets its
/// own, [`tmp_dir`], as `TMPDIR`.
/// Execute: Node and the system's programs, so an extension that shells out
/// still can, inside the same boundary. Paths that do not exist are left out,
/// since Landlock cannot name them.
#[must_use]
pub fn policy(
    runtime: &Runtime,
    command: &ExtensionCommand,
    data_dir: &Path,
) -> compass_sandbox::Policy {
    let parent = |path: &Path| path.parent().map(Path::to_path_buf);
    let read = [
        "/usr", "/etc", "/proc", "/sys", "/dev", "/lib", "/lib64", "/bin", "/app",
    ]
    .into_iter()
    .map(PathBuf::from)
    .chain(parent(&runtime.node))
    .chain(parent(&runtime.bundle))
    .chain([command.extension_dir.clone()]);
    let write = [
        support_dir(data_dir, command),
        assets_dir(data_dir, command),
    ];
    let execute = ["/usr/bin", "/bin", "/app/bin"]
        .into_iter()
        .map(PathBuf::from)
        .chain([runtime.node.clone()]);

    let mut policy = compass_sandbox::Policy::new();
    for path in read.filter(|path| path.exists()) {
        policy = policy.read(path);
    }
    for path in write.into_iter().filter(|path| path.exists()) {
        policy = policy.read(path.clone()).write(path);
    }
    for path in execute.filter(|path| path.exists()) {
        policy = policy.execute(path);
    }
    policy
}

/// `<data>/support/<extension id>`, where the runtime keeps an extension's
/// support files and logs.
#[must_use]
pub fn support_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    data_dir.join("support").join(&command.extension_id)
}

/// `<support>/.tmp`, the extension's private temporary directory.
#[must_use]
pub fn tmp_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    support_dir(data_dir, command).join(".tmp")
}

/// `<data>/extensions/<extension id>/assets`, which the runtime creates.
#[must_use]
pub fn assets_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    data_dir
        .join("extensions")
        .join(&command.extension_id)
        .join("assets")
}

fn installed_bundle() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| {
        Some(
            exe.parent()?
                .parent()?
                .join("share/vicinae")
                .join(RUNTIME_FILE_NAME),
        )
    });
    beside_exe
        .into_iter()
        .chain([Path::new("/app/share/vicinae").join(RUNTIME_FILE_NAME)])
        .find(|path| path.is_file())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// Where a run keeps its local storage, and the key that opens it.
#[derive(Clone)]
pub struct Storage {
    /// The database file.
    pub path: PathBuf,
    /// Its SQLCipher key.
    pub key: [u8; compass_crypto::KEY_SIZE],
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Why a command will not run, as a sentence; `None` when it can.
#[must_use]
pub fn refusal(command: &ExtensionCommand) -> Option<String> {
    if command.mode == CommandMode::View {
        return Some(format!(
            "{} shows a view, and Compass cannot draw extension views yet",
            command.title
        ));
    }
    if let Err(missing) = command.default_preferences() {
        return Some(format!(
            "{} needs {} set, and Compass cannot edit extension preferences yet",
            command.title,
            missing.join(", ")
        ));
    }
    None
}

/// Starts `command` and returns once the runtime has loaded it; the run
/// continues on its own thread. Blocking: call it off the async runtime.
///
/// `data_dir` is where the runtime keeps each extension's support and asset
/// directories (`vicinae_path`). `storage` is `None` when there is no keyring;
/// the command then gets "not implemented" for local storage.
///
/// # Errors
///
/// A sentence for the launcher: the command cannot run here, the runtime
/// would not start, or it did not accept the command.
pub fn start(
    runtime: &Runtime,
    command: &ExtensionCommand,
    data_dir: &Path,
    storage: Option<Storage>,
) -> Result<(), String> {
    if let Some(reason) = refusal(command) {
        return Err(reason);
    }
    let preferences = command.default_preferences().unwrap_or_default();

    // The runtime creates these itself, but a sandbox can only grant a path
    // that exists, so they are made first.
    for dir in [tmp_dir(data_dir, command), assets_dir(data_dir, command)] {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::info!(dir = %dir.display(), %err, "could not prepare an extension directory");
        }
    }
    let bundle = [runtime.bundle.to_string_lossy().into_owned()];
    let mut process = match &runtime.sandbox {
        Some(launcher) => {
            policy(runtime, command, data_dir).command(launcher, &runtime.node, &bundle)
        }
        None => {
            let mut process = std::process::Command::new(&runtime.node);
            process.args(bundle);
            process
        }
    };
    process.env("TMPDIR", tmp_dir(data_dir, command));
    let spawned = Worker::spawn(process);
    let mut worker =
        spawned.map_err(|err| format!("The extension runtime would not start: {err}"))?;
    let pid = worker.pid();

    // The watchdog also bounds the handshake: a runtime that never answers
    // `load` is stopped, which ends the read below.
    let activity = Arc::new(Activity::new());
    watch(pid, Arc::clone(&activity), LOAD_TIMEOUT);

    let options = LoadOptions {
        mode: compass_worker_host::extension_manager::CommandMode::NoView,
        env: CommandEnv::Production,
        vicinae_path: data_dir.to_string_lossy().into_owned(),
        entrypoint: command.entrypoint.to_string_lossy().into_owned(),
        is_raycast: command.is_raycast,
        command_name: command.name.clone(),
        extension_id: command.extension_id.clone(),
        extension_name: command.extension_name.clone(),
        owner_or_author_name: command.author.clone(),
        arguments: serde_json::json!({}),
        preferences,
        launch_context: serde_json::Value::Null,
        launch_type: LaunchType::User,
        capabilities: Capabilities::default(),
        fallback_text: None,
        cwd: None,
    };
    let load_id = ManagerClient::new(&mut worker)
        .load(&options)
        .map_err(|err| format!("The extension runtime did not take the command: {err}"))?;
    let session_id = loop {
        let message = worker
            .next_message()
            .map_err(|err| format!("The extension runtime failed: {err}"))?
            .ok_or_else(|| "The extension runtime stopped before loading the command".to_owned())?;
        if message.id != Some(load_id) {
            continue;
        }
        break message
            .result
            .as_ref()
            .and_then(|result| result.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("The extension runtime would not load {}", command.title))?
            .to_owned();
    };
    ManagerClient::new(&mut worker)
        .ready(&session_id)
        .map_err(|err| format!("The extension runtime failed: {err}"))?;
    activity.touch();
    activity.run_for(LIFETIME);

    let title = command.title.clone();
    let name = command.name.clone();
    let namespace = compass_local_storage::namespace_for(&command.extension_id);
    let handle = tokio::runtime::Handle::try_current().ok();
    std::thread::Builder::new()
        .name(format!("extension {}", command.id))
        .spawn(move || {
            serve(
                worker,
                &session_id,
                &title,
                &name,
                &namespace,
                storage,
                handle,
                &activity,
            );
        })
        .map_err(|err| format!("could not start a thread for the command: {err}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn serve(
    worker: Worker,
    session_id: &str,
    title: &str,
    name: &str,
    namespace: &str,
    storage: Option<Storage>,
    handle: Option<tokio::runtime::Handle>,
    activity: &Activity,
) {
    let database = storage.and_then(|storage| open_storage(&storage));
    let local = database
        .as_ref()
        .map(compass_local_storage::LocalStorage::new);
    let scoped = local.as_ref().map(|local| local.scoped(namespace));
    let storage_service = scoped.map(StorageService::new);
    let shell = UiShellService::new(
        HeadlessShell {
            title: title.to_owned(),
            handle,
        },
        CommandInfo {
            name: name.to_owned(),
            icon: String::new(),
        },
    );
    let mut router = Router::new().with(&shell);
    if let Some(service) = &storage_service {
        router = router.with(service);
    }
    let mut session = Session::new(worker, session_id, router);
    loop {
        let turn = match session.pump_once() {
            Ok(turn) => turn,
            Err(err) => {
                if !activity.stopped() {
                    tracing::warn!(command = title, error = %err, "extension session ended");
                }
                break;
            }
        };
        activity.touch();
        match turn {
            Turn::Closed => break,
            Turn::Crashed { reason } => {
                tracing::warn!(command = title, %reason, "extension command crashed");
                break;
            }
            Turn::Deferred { method, deferral } => {
                // Only an alert defers, and nothing can show one yet: "no" is
                // the answer a dismissed alert gives.
                tracing::info!(command = title, %method, "answered a dialog with no");
                if let Err(err) = session.answer_deferred(&deferral, serde_json::json!(false)) {
                    tracing::warn!(command = title, error = %err, "could not answer a dialog");
                    break;
                }
            }
            Turn::Answered { .. } | Turn::Nothing | Turn::OtherSession { .. } => {}
        }
    }
    activity.stop();
}

fn open_storage(storage: &Storage) -> Option<compass_sqlcipher_sys::Database> {
    let opened = compass_sqlcipher_sys::Database::open(&storage.path, &storage.key)
        .map_err(|err| err.to_string())
        .and_then(|db| {
            compass_db::vicinae::run(&db)
                .map(|()| db)
                .map_err(|err| err.to_string())
        });
    match opened {
        Ok(db) => Some(db),
        Err(err) => {
            tracing::warn!(path = %storage.path.display(), %err, "extension storage unavailable");
            None
        }
    }
}

/// When a run last said anything, and whether it has been stopped.
struct Activity {
    started: Instant,
    last: AtomicU64,
    deadline: AtomicU64,
    stopped: AtomicBool,
}

impl Activity {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            last: AtomicU64::new(0),
            deadline: AtomicU64::new(u64::MAX),
            stopped: AtomicBool::new(false),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn touch(&self) {
        self.last.store(self.elapsed_ms(), Ordering::Relaxed);
    }

    /// From now on, the run is over after [`IDLE`] of quiet or `lifetime`.
    fn run_for(&self, lifetime: Duration) {
        let lifetime = u64::try_from(lifetime.as_millis()).unwrap_or(u64::MAX);
        self.deadline.store(
            self.elapsed_ms().saturating_add(lifetime),
            Ordering::Relaxed,
        );
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Whether the run should be ended now. Before [`Self::run_for`], only
    /// `handshake` bounds it.
    fn expired(&self, handshake: Duration) -> bool {
        let now = self.elapsed_ms();
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline == u64::MAX {
            return now > u64::try_from(handshake.as_millis()).unwrap_or(u64::MAX);
        }
        let idle = u64::try_from(IDLE.as_millis()).unwrap_or(u64::MAX);
        now > deadline || now.saturating_sub(self.last.load(Ordering::Relaxed)) > idle
    }
}

/// Stops the runtime at `pid` once the run is over, which closes its output
/// and ends [`serve`]'s loop.
fn watch(pid: u32, activity: Arc<Activity>, handshake: Duration) {
    let spawned = std::thread::Builder::new()
        .name("extension watchdog".to_owned())
        .spawn(move || {
            while !activity.stopped() {
                if activity.expired(handshake) {
                    activity.stop();
                    // The runtime is our own child; SIGTERM lets Node end its
                    // worker threads, and `serve` then reads end-of-file.
                    // kill(1) rather than kill(2): this crate forbids unsafe.
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .status();
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(error = %err, "no watchdog for an extension run");
    }
}

/// The shell around a command with no view: what it would show on screen
/// becomes a desktop notification.
struct HeadlessShell {
    title: String,
    handle: Option<tokio::runtime::Handle>,
}

impl HeadlessShell {
    fn notify(&self, title: &str, body: &str) {
        let Some(handle) = &self.handle else {
            tracing::info!(
                command = self.title,
                title,
                body,
                "no runtime to notify from"
            );
            return;
        };
        let notification = compass_notify::Notification::new(title, body);
        let sent = handle.block_on(async {
            let connection = zbus::Connection::session()
                .await
                .map_err(|err| err.to_string())?;
            compass_notify::send(&connection, &notification)
                .await
                .map_err(|err| err.to_string())
        });
        if let Err(err) = sent {
            tracing::info!(command = self.title, title, body, error = %err, "not notified");
        }
    }
}

impl Shell for HeadlessShell {
    fn set_toast(&self, title: &str, style: ToastStyle, message: &str) {
        // A no-view command's success toast is the same news as the HUD it
        // usually shows next; only a failure is worth interrupting for.
        if style == ToastStyle::Danger {
            self.notify(title, message);
        } else {
            tracing::info!(command = self.title, title, message, "toast");
        }
    }

    fn clear_toast(&self) {}

    fn close_window(&self, _options: CloseWindow) {}

    fn show_hud(&self, text: &str) {
        self.notify(&self.title, text);
    }

    fn pop_to_root(&self, _clear_search: bool) {}

    fn push_view(&self, _command: &CommandInfo) {}

    fn pop_view(&self) {}

    fn set_search_text(&self, _text: &str) {}

    fn selected_text(&self) -> Result<String, String> {
        Err("Selected text is not available to extensions in Compass yet".to_owned())
    }

    fn send_notification(&self, notification: &Notification) {
        self.notify(&notification.title, &notification.body);
    }

    fn show_alert(&self, _alert: &Alert, _deferral: &Deferral) {}
}
