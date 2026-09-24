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
use compass_worker_host::application_service::ApplicationService;
use compass_worker_host::clipboard_service::{
    Clipboard, ClipboardService, Content, CopyOptions, ReadContent,
};
use compass_worker_host::extension_manager::{
    Capabilities, CommandEnv, LaunchType, LoadOptions, ManagerClient,
};
use compass_worker_host::session::SessionEvents;
use compass_worker_host::session::{Router, Session, Turn};
use compass_worker_host::storage_service::StorageService;
use compass_worker_host::tsapi::Deferral;
use compass_worker_host::ui_service::UiService;
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

/// The local-storage namespace an extension's preference values live in,
/// beside its own `<id>:data`: in the same encrypted database, and out of
/// the extension's reach, since its `LocalStorage` is scoped to `:data`.
#[must_use]
pub fn preferences_namespace(extension_id: &str) -> String {
    format!("compass.preferences:{extension_id}")
}

/// The preference values stored for `extension_id`; empty when none are, or
/// the database will not open.
#[must_use]
pub fn load_preferences(
    storage: &Storage,
    extension_id: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let Some(db) = open_storage(storage) else {
        return serde_json::Map::new();
    };
    let local = compass_local_storage::LocalStorage::new(&db);
    let scoped = local.scoped(&preferences_namespace(extension_id));
    match scoped.list() {
        Ok(values) => values
            .into_iter()
            .map(|(name, value)| (name, value.to_json()))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "could not read extension preferences");
            serde_json::Map::new()
        }
    }
}

/// Keeps `values` for `extension_id`. A null or empty value removes the
/// stored one, so clearing a field falls back to its default.
///
/// # Errors
///
/// A sentence: the database would not open, or a write failed.
pub fn save_preferences(
    storage: &Storage,
    extension_id: &str,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let db = open_storage(storage).ok_or("Compass could not open its extension storage")?;
    let local = compass_local_storage::LocalStorage::new(&db);
    let scoped = local.scoped(&preferences_namespace(extension_id));
    for (name, value) in values {
        let cleared = value.is_null() || value.as_str() == Some("");
        let written = if cleared {
            scoped.remove(name).map(drop)
        } else {
            scoped.set(name, &compass_local_storage::Value::from_json(value))
        };
        written.map_err(|err| format!("Compass could not keep the preference {name}: {err}"))?;
    }
    Ok(())
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
    host: Host,
) -> Result<Started, String> {
    let Host {
        storage,
        shell,
        views,
        preferences,
        arguments,
        apps,
    } = host;

    // The runtime creates these itself, but a sandbox can only grant a path
    // that exists, so they are made first.
    for dir in [tmp_dir(data_dir, command), assets_dir(data_dir, command)] {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::info!(dir = %dir.display(), %err, "could not prepare an extension directory");
        }
    }
    let bundle = [
        compass_worker_host::cgroups::node_heap_flag(),
        runtime.bundle.to_string_lossy().into_owned(),
    ];
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
    confine_memory(pid, &command.extension_id);

    // The watchdog also bounds the handshake: a runtime that never answers
    // `load` is stopped, which ends the read below.
    let activity = Arc::new(Activity::new());
    watch(pid, Arc::clone(&activity), LOAD_TIMEOUT);

    let options = LoadOptions {
        mode: match command.mode {
            CommandMode::View => compass_worker_host::extension_manager::CommandMode::View,
            CommandMode::NoView => compass_worker_host::extension_manager::CommandMode::NoView,
        },
        env: CommandEnv::Production,
        vicinae_path: data_dir.to_string_lossy().into_owned(),
        entrypoint: command.entrypoint.to_string_lossy().into_owned(),
        is_raycast: command.is_raycast,
        command_name: command.name.clone(),
        extension_id: command.extension_id.clone(),
        extension_name: command.extension_name.clone(),
        owner_or_author_name: command.author.clone(),
        arguments,
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
    // A view lives until the person leaves it; only a no-view run is timed.
    let view = (command.mode == CommandMode::View).then(|| views.open(pid));
    if view.is_some() {
        activity.run_forever();
    } else {
        activity.run_for(LIFETIME);
    }

    let title = command.title.clone();
    let name = command.name.clone();
    let namespace = compass_local_storage::namespace_for(&command.extension_id);
    let handle = tokio::runtime::Handle::try_current().ok();
    let started = view
        .as_ref()
        .map_or(Started::Ran, |view| Started::View(view.session));
    std::thread::Builder::new()
        .name(format!("extension {}", command.id))
        .spawn(move || {
            serve(
                worker,
                Served {
                    session_id,
                    title,
                    name,
                    namespace,
                    apps,
                },
                storage,
                ShellClipboard {
                    shell,
                    handle: handle.clone(),
                },
                handle,
                &activity,
                view,
            );
        })
        .map_err(|err| format!("could not start a thread for the command: {err}"))?;
    Ok(started)
}

/// Caps the worker's memory on the user's systemd, where it is reachable.
/// The heap flag already bounds the JavaScript side; this is the rest, and a
/// host without it (a Flatpak, a container) still runs the command.
fn confine_memory(pid: u32, extension_id: &str) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let scope = compass_worker_host::cgroups::scope_name(extension_id, pid);
    handle.spawn(async move {
        match compass_worker_host::cgroups::confine(pid, &scope).await {
            Ok(()) => tracing::debug!(%scope, "extension worker memory capped"),
            Err(err) => tracing::info!(
                %err,
                "no systemd user manager to cap the extension's memory; the heap cap still holds"
            ),
        }
    });
}

/// What one run needs from the engine beyond the command itself.
#[derive(Debug, Default)]
pub struct Host {
    /// Local storage, or `None` without a keyring.
    pub storage: Option<Storage>,
    /// The GNOME Shell extension, which is the clipboard on GNOME.
    pub shell: Option<Arc<compass_shell::ShellClient>>,
    /// Where a view command's session is published.
    pub views: Arc<Views>,
    /// The preference values the command reads, already resolved.
    pub preferences: serde_json::Value,
    /// The argument values it was launched with.
    pub arguments: serde_json::Value,
    /// What `open()` and `getApplications()` reach, or `None` to refuse them.
    pub apps: Option<crate::extension_apps::EngineApps>,
}

/// How a run began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// A no-view command, running on its own.
    Ran,
    /// A view command: the launcher follows this session in [`Views`].
    View(u64),
}

struct Served {
    session_id: String,
    title: String,
    name: String,
    namespace: String,
    apps: Option<crate::extension_apps::EngineApps>,
}

fn serve(
    worker: Worker,
    served: Served,
    storage: Option<Storage>,
    clipboard: ShellClipboard,
    handle: Option<tokio::runtime::Handle>,
    activity: &Activity,
    view: Option<ViewHandle>,
) {
    let Served {
        session_id,
        title,
        name,
        namespace,
        apps,
    } = served;
    let title = title.as_str();
    let database = storage.and_then(|storage| open_storage(&storage));
    let local = database
        .as_ref()
        .map(compass_local_storage::LocalStorage::new);
    let scoped = local.as_ref().map(|local| local.scoped(&namespace));
    let storage_service = scoped.map(StorageService::new);
    let shell = UiShellService::new(
        HeadlessShell {
            title: title.to_owned(),
            handle,
            alert: std::sync::Mutex::new(None),
            view: view.as_ref().map(|view| view.state.clone()),
        },
        CommandInfo {
            name: name.to_owned(),
            icon: String::new(),
        },
    );
    let clipboard = ClipboardService::new(clipboard);
    let ui = UiService::new();
    let mut router = Router::new().with(&shell).with(&clipboard).with(&ui);
    if let Some(service) = &storage_service {
        router = router.with(service);
    }
    let applications = apps.map(ApplicationService::new);
    if let Some(service) = &applications {
        router = router.with(service);
    }
    let mut session = Session::new(worker, session_id.as_str(), router);
    if let Some(view) = &view {
        view.attach(session.events());
    }
    let mut ended = None;
    loop {
        let turn = match session.pump_once() {
            Ok(turn) => turn,
            Err(err) => {
                if !activity.stopped() {
                    tracing::warn!(command = title, error = %err, "extension session ended");
                    ended = Some(format!("{title} stopped: {err}"));
                }
                break;
            }
        };
        activity.touch();
        match turn {
            Turn::Closed => break,
            Turn::Crashed { reason } => {
                tracing::warn!(command = title, %reason, "extension command crashed");
                ended = Some(format!("{title} crashed: {reason}"));
                break;
            }
            Turn::Answered { method } if method == "UI/render" => {
                if let (Some(view), Some(root)) = (&view, ui.top()) {
                    let depth = u32::try_from(ui.stack().len()).unwrap_or(u32::MAX);
                    view.publish(compass_worker_host::view_model::to_view(&root), depth);
                }
            }
            Turn::Deferred { method, deferral } => {
                // Only an alert defers. A view shows it and the launcher
                // answers; a command with no view has nowhere to show it, and
                // "no" is the answer a dismissed alert gives.
                let alert = shell
                    .shell()
                    .alert
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let (Some(view), Some(alert)) = (&view, alert) {
                    view.ask(alert, deferral);
                    continue;
                }
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
    if let Some(view) = view {
        view.end(ended);
    }
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

    /// A view's run: over only when stopped.
    fn run_forever(&self) {
        self.deadline.store(u64::MAX - 1, Ordering::Relaxed);
        self.last.store(u64::MAX, Ordering::Relaxed);
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
        if deadline == u64::MAX - 1 {
            return false;
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
    /// The alert `show_alert` was last given, for the serving loop to hand
    /// the launcher when the call defers.
    alert: std::sync::Mutex<Option<compass_ipc::ExtensionAlert>>,
    /// A view's state, where its toasts are shown; `None` for a no-view run,
    /// whose toasts become notifications.
    view: Option<tokio::sync::watch::Sender<ViewState>>,
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
        let sent = handle.block_on(
            notify_rust::Notification::new()
                .appname("Vicinae")
                .summary(title)
                .body(body)
                .show_async(),
        );
        if let Err(err) = sent {
            tracing::info!(command = self.title, title, body, error = %err, "not notified");
        }
    }
}

impl Shell for HeadlessShell {
    fn set_toast(&self, title: &str, style: ToastStyle, message: &str) {
        if let Some(view) = &self.view {
            let toast = compass_ipc::ExtensionToast {
                title: title.to_owned(),
                message: message.to_owned(),
                style: match style {
                    ToastStyle::Success => compass_ipc::ExtensionToastStyle::Success,
                    ToastStyle::Info => compass_ipc::ExtensionToastStyle::Info,
                    ToastStyle::Warning => compass_ipc::ExtensionToastStyle::Warning,
                    ToastStyle::Danger => compass_ipc::ExtensionToastStyle::Failure,
                    ToastStyle::Dynamic => compass_ipc::ExtensionToastStyle::Animated,
                },
            };
            view.send_modify(|state| {
                state.version += 1;
                state.toast = Some(toast);
            });
            return;
        }
        // A no-view command's success toast is the same news as the HUD it
        // usually shows next; only a failure is worth interrupting for.
        if style == ToastStyle::Danger {
            self.notify(title, message);
        } else {
            tracing::info!(command = self.title, title, message, "toast");
        }
    }

    fn clear_toast(&self) {
        if let Some(view) = &self.view {
            view.send_modify(|state| {
                if state.toast.take().is_some() {
                    state.version += 1;
                }
            });
        }
    }

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

    fn show_alert(&self, alert: &Alert, _deferral: &Deferral) {
        *self
            .alert
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(compass_ipc::ExtensionAlert {
                title: alert.title.clone(),
                message: alert.message.clone(),
                confirm_text: alert.confirm_text.clone(),
                cancel_text: alert.cancel_text.clone(),
            });
    }
}

/// Running view commands, as the launcher follows them.
///
/// Each session has a [`ViewState`] the launcher long-polls
/// (`ExtensionView`), and a way to reach the extension (`ExtensionEvent`)
/// while the serving thread blocks reading it.
#[derive(Debug, Default)]
pub struct Views {
    next: std::sync::atomic::AtomicU64,
    sessions: std::sync::Mutex<std::collections::HashMap<u64, ViewEntry>>,
}

#[derive(Debug)]
struct ViewEntry {
    state: tokio::sync::watch::Sender<ViewState>,
    events: Arc<std::sync::Mutex<Option<SessionEvents>>>,
    pid: u32,
    /// The call a shown alert holds open.
    deferral: Arc<std::sync::Mutex<Option<Deferral>>>,
}

/// What a view session shows now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Bumped on every change.
    pub version: u64,
    /// The view, as `compass_extension_api::View` JSON; `None` before the
    /// first render.
    pub view: Option<String>,
    /// Why it cannot be drawn (a component Compass does not support), or why
    /// it ended.
    pub problem: Option<String>,
    /// Whether the command has ended.
    pub ended: bool,
    /// How many views the extension has pushed, the root one included.
    pub depth: u32,
    /// A confirmation the extension waits on.
    pub alert: Option<compass_ipc::ExtensionAlert>,
    /// The toast the extension shows over its view.
    pub toast: Option<compass_ipc::ExtensionToast>,
}

impl Views {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, ViewEntry>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn open(self: &Arc<Self>, pid: u32) -> ViewHandle {
        let session = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (state, _) = tokio::sync::watch::channel(ViewState::default());
        let events = Arc::new(std::sync::Mutex::new(None));
        let deferral = Arc::new(std::sync::Mutex::new(None));
        self.lock().insert(
            session,
            ViewEntry {
                state: state.clone(),
                events: Arc::clone(&events),
                pid,
                deferral: Arc::clone(&deferral),
            },
        );
        ViewHandle {
            session,
            state,
            events,
            deferral,
            views: Arc::clone(self),
        }
    }

    /// Follows `session`'s state; `None` for a session that is not running.
    #[must_use]
    pub fn watch(&self, session: u64) -> Option<tokio::sync::watch::Receiver<ViewState>> {
        self.lock()
            .get(&session)
            .map(|entry| entry.state.subscribe())
    }

    /// Sends `handler` with `args` to `session`'s extension.
    ///
    /// # Errors
    ///
    /// A sentence: the session is gone, or its worker is.
    pub fn activate(
        &self,
        session: u64,
        handler: &str,
        args: &[serde_json::Value],
    ) -> Result<(), String> {
        let events = self
            .lock()
            .get(&session)
            .map(|entry| Arc::clone(&entry.events))
            .ok_or_else(|| "That extension view has closed".to_owned())?;
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        events
            .handler_activated(
                &compass_extension_api::action::HandlerId::new(handler),
                args,
            )
            .map_err(|err| format!("The extension did not take it: {err}"))
    }

    /// Answers the alert `session` is showing.
    ///
    /// # Errors
    ///
    /// A sentence: no such session, no alert waiting, or the worker is gone.
    pub fn answer_alert(&self, session: u64, confirmed: bool) -> Result<(), String> {
        let (events, deferral, state) = {
            let sessions = self.lock();
            let entry = sessions
                .get(&session)
                .ok_or_else(|| "That extension view has closed".to_owned())?;
            (
                Arc::clone(&entry.events),
                Arc::clone(&entry.deferral),
                entry.state.clone(),
            )
        };
        let deferral = deferral
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| "That extension is not asking anything".to_owned())?;
        state.send_modify(|state| {
            state.version += 1;
            state.alert = None;
        });
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        events
            .answer(&deferral, serde_json::json!(confirmed))
            .map_err(|err| format!("The extension did not take the answer: {err}"))
    }

    /// Pops `session`'s top view, as Escape on a pushed view does.
    ///
    /// # Errors
    ///
    /// A sentence: the session is gone, or its worker is.
    pub fn pop(&self, session: u64) -> Result<(), String> {
        let events = self
            .lock()
            .get(&session)
            .map(|entry| Arc::clone(&entry.events))
            .ok_or_else(|| "That extension view has closed".to_owned())?;
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        events
            .view_popped()
            .map_err(|err| format!("The extension did not take it: {err}"))
    }

    /// Ends `session`: its runtime is stopped. `false` when it was not running.
    pub fn close(&self, session: u64) -> bool {
        let Some(entry) = self.lock().remove(&session) else {
            return false;
        };
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(entry.pid.to_string())
            .status();
        true
    }
}

/// The serving thread's side of one view session.
struct ViewHandle {
    session: u64,
    state: tokio::sync::watch::Sender<ViewState>,
    events: Arc<std::sync::Mutex<Option<SessionEvents>>>,
    deferral: Arc<std::sync::Mutex<Option<Deferral>>>,
    views: Arc<Views>,
}

impl ViewHandle {
    fn attach(&self, events: SessionEvents) {
        *self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(events);
    }

    fn publish(
        &self,
        view: Result<compass_extension_api::View, compass_worker_host::view_model::Unsupported>,
        depth: u32,
    ) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.depth = depth;
            match view {
                Ok(view) => {
                    state.view = serde_json::to_string(&view).ok();
                    state.problem = None;
                }
                Err(unsupported) => state.problem = Some(unsupported.to_string()),
            }
        });
    }

    /// Shows `alert` and holds `deferral` until the launcher answers.
    fn ask(&self, alert: compass_ipc::ExtensionAlert, deferral: Deferral) {
        *self
            .deferral
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(deferral);
        self.state.send_modify(|state| {
            state.version += 1;
            state.alert = Some(alert);
        });
    }

    fn end(self, why: Option<String>) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.ended = true;
            if why.is_some() {
                state.problem = why;
            }
        });
        self.views.lock().remove(&self.session);
    }
}

/// The clipboard an extension reaches: the GNOME Shell extension's.
struct ShellClipboard {
    shell: Option<Arc<compass_shell::ShellClient>>,
    handle: Option<tokio::runtime::Handle>,
}

impl ShellClipboard {
    fn run<T>(
        &self,
        what: &str,
        call: impl FnOnce(
            Arc<compass_shell::ShellClient>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = compass_shell::Result<T>> + Send>,
        >,
    ) -> Option<T> {
        let (Some(shell), Some(handle)) = (&self.shell, &self.handle) else {
            tracing::info!(
                what,
                "no GNOME Shell extension; the clipboard call did nothing"
            );
            return None;
        };
        match handle.block_on(call(Arc::clone(shell))) {
            Ok(value) => Some(value),
            Err(err) => {
                tracing::info!(what, error = %err, "clipboard call failed");
                None
            }
        }
    }

    fn selection(content: Content) -> Option<compass_shell::ClipboardContent> {
        match content {
            Content::NoData => None,
            Content::Text(text) => Some(compass_shell::ClipboardContent::text(text)),
            Content::Html {
                text: Some(text), ..
            } => Some(compass_shell::ClipboardContent::text(text)),
            Content::Html { html, text: None } => Some(compass_shell::ClipboardContent::binary(
                html.into_bytes(),
                "text/html",
            )),
            Content::Urls(urls) => Some(compass_shell::ClipboardContent::binary(
                urls.join("\r\n").into_bytes(),
                "text/uri-list",
            )),
        }
    }
}

impl Clipboard for ShellClipboard {
    fn copy(&self, content: Content, _options: CopyOptions) {
        let Some(selection) = Self::selection(content) else {
            return;
        };
        self.run("copy", move |shell| {
            Box::pin(async move { shell.set_clipboard(&selection).await })
        });
    }

    fn paste(&self, content: Content) {
        let Some(selection) = Self::selection(content) else {
            return;
        };
        self.run("paste", move |shell| {
            Box::pin(async move {
                shell.set_clipboard(&selection).await?;
                shell.paste(&[]).await
            })
        });
    }

    fn clear(&self) {
        self.run("clear", |shell| {
            Box::pin(async move {
                shell
                    .set_clipboard(&compass_shell::ClipboardContent::text(""))
                    .await
            })
        });
    }

    fn read(&self) -> ReadContent {
        self.run("read", |shell| {
            Box::pin(async move { shell.clipboard().await })
        })
        .map(|content| ReadContent {
            text: content.as_text().unwrap_or_default().to_owned(),
            ..ReadContent::default()
        })
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage(dir: &Path) -> Storage {
        Storage {
            path: dir.join(STORAGE_DATABASE),
            key: [3; compass_crypto::KEY_SIZE],
        }
    }

    #[test]
    fn preferences_round_trip_and_clearing_one_removes_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = storage(dir.path());
        assert!(load_preferences(&storage, "github").is_empty());

        let values = serde_json::json!({"token": "ghp_x", "limit": 50, "private": true});
        save_preferences(&storage, "github", values.as_object().unwrap()).expect("saved");
        assert_eq!(
            serde_json::Value::Object(load_preferences(&storage, "github")),
            serde_json::json!({"token": "ghp_x", "limit": 50.0, "private": true}),
            "strings and booleans come back as they went in, and a number as the \
             double JavaScript would have had anyway"
        );
        assert!(
            load_preferences(&storage, "other").is_empty(),
            "per extension"
        );

        let cleared = serde_json::json!({"token": ""});
        save_preferences(&storage, "github", cleared.as_object().unwrap()).expect("saved");
        assert!(!load_preferences(&storage, "github").contains_key("token"));
    }

    #[test]
    fn preferences_are_out_of_the_extensions_own_storage() {
        // An extension's LocalStorage is scoped to `<id>:data`. Keeping
        // preferences there would let it read and rewrite its own token.
        assert_ne!(
            preferences_namespace("github"),
            compass_local_storage::namespace_for("github")
        );
    }
}
