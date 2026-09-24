//! The engine: the thing that actually runs.
//!
//! Everything else in this workspace is a library, and every `vicinae`
//! subcommand until now has been a *client* looking for a server that did not
//! exist. This module is that server.
//!
//! # Headless on purpose
//!
//! This engine has no window of its own and never opens one. It serves the
//! requests that do not need one — [`Request::Ping`], [`Request::Query`],
//! [`Request::Doctor`], [`Request::Shutdown`] — always, on any machine,
//! display or not.
//!
//! [`Request::Show`], [`Request::Hide`] and [`Request::Toggle`] are forwarded
//! to a **resident launcher window** that attached itself over the same socket
//! ([ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)).
//! With no window attached they are **refused**, with [`ErrorKind::Unsupported`]
//! and a message saying how to start one.
//!
//! Refusing matters more than it looks. `Response::Ack` means "the side effect
//! was performed"; answering `Ack` to `Show` when nothing can be shown would
//! make a client unable to tell a working engine from a windowless one, and
//! would make UI bring-up debug a lie instead of a gap.
//!
//! What it *does* serve is a genuine vertical slice — index the machine's
//! applications, rank a query against them with frecency, answer over the same
//! socket the C++ engine uses — and all of it is exercisable with no display,
//! which is why it is the part built first.

use std::sync::Arc;

use anyhow::{Context, Result};
use compass_core::{AppIndex, Config, FrecencyStore, JsonFrecencyStore, SystemClock};
use compass_ipc::{
    ErrorKind, Listener, ProtocolError, QueryHit, Request, Response, SocketPath, WindowCommand,
    WindowLink, WindowOutcome, protocol::PROTOCOL_VERSION,
};
use tokio::sync::{Mutex, RwLock};

use crate::doctor;
use crate::engine::Engine;

/// The at-most-one launcher window this engine drives.
///
/// # Last window wins
///
/// A second `vicinae ui` **replaces** the first rather than being refused.
/// Dropping the old [`WindowLink`] closes its socket, so that window's
/// `next_command` returns `None` and it exits its loop.
///
/// The alternative — refuse the newcomer — reads better until you ask what
/// happens when a window wedges without closing its socket: the engine would
/// hold a link that answers nothing and refuse every replacement, and the only
/// way out would be restarting the daemon. Replacing has no state that can get
/// stuck. It also means the handler needs no reservation between answering
/// `WindowAttached` and the link actually arriving, which is a window (however
/// narrow) in which an attach that died mid-handshake could strand the slot.
///
/// # Death is noticed on the next push, not before
///
/// Nothing polls the link. A window that dies is discovered when the engine
/// next tries to push to it, at which point the slot is cleared and the request
/// is refused with [`no_window`] — correctly, since by then there is none.
/// Proactively watching would mean a second reader on a socket whose only
/// reader is [`WindowLink::push`]'s reply, so it would have to be built into
/// the link rather than bolted beside it.
type WindowSlot = Arc<Mutex<Option<WindowLink>>>;

/// Where launch history lives, under `$XDG_DATA_HOME`.
///
/// Returns `None` when there is no data home to put it in, which is the
/// read-only-session case the caller degrades into an in-memory store for.
fn frecency_path() -> Option<std::path::PathBuf> {
    Some(compass_core::xdg_dirs::data_home()?.join("vicinae/frecency.json"))
}

/// The engine's state, shared across connections.
///
/// `RwLock` rather than `Mutex` because queries are the hot path and they only
/// read; the write side is frecency updates and, later, reindexing.
pub struct EngineState {
    index: AppIndex,
    frecency: Box<dyn FrecencyStore + Send + Sync>,
    socket: SocketPath,
    /// How many hits a query answers with. `launcher.max_results` from the
    /// user's config, so the wire honours the same limit the UI would.
    max_results: usize,
    /// The attached launcher window, if one is.
    window: WindowSlot,
    /// Clipboard history, once [`crate::clipboard_service::run`] has opened
    /// it. `None` until then, and for good when there is no keyring.
    clipboard: Option<Arc<crate::clipboard_service::ClipboardStore>>,
    /// The GNOME Shell extension's client, once the session bus answered.
    /// `None` until then, and for good without a session bus.
    shell: Option<Arc<compass_shell::ShellClient>>,
    /// Running extension view commands, which the launcher follows.
    views: Arc<crate::extension_runner::Views>,
    /// Search Files, and the file indexer it supervises.
    files: Arc<crate::file_search::FileSearch>,
}

// Hand-written because `dyn FrecencyStore` is not `Debug`, and widening that
// trait to require it would push a formatting concern onto every future store
// implementation for the sake of one log line.
impl std::fmt::Debug for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineState")
            .field("applications", &self.index.len())
            .field("socket", &self.socket)
            .field("max_results", &self.max_results)
            // Deliberately not the link itself: formatting it would need the
            // lock, and `Debug` is used from log lines that must not block.
            .finish_non_exhaustive()
    }
}

impl EngineState {
    /// Builds the state by indexing this machine.
    ///
    /// A frecency store that cannot be loaded is **not** fatal: the launcher
    /// works without launch history, just less well ordered, and refusing to
    /// start over a corrupt ranking cache would be a worse failure than the one
    /// it is reporting.
    pub fn from_environment(socket: SocketPath) -> Self {
        let mut index = AppIndex::from_environment();
        tracing::info!(applications = index.len(), "indexed applications");

        // A bad config is reported and then ignored rather than fatal. Refusing
        // to start because `max_results` is misspelled would be a worse outcome
        // than starting with the default and saying so.
        let config = match Config::load() {
            Ok(config) => config,
            Err(err) => {
                let fallback = Config::default();
                tracing::warn!(error = %err, "using default configuration");
                fallback
            }
        };
        let max_results = config.launcher().max_results();
        index.apply_root_config(&config.root_config());

        let frecency = Self::open_frecency();

        // Started with the engine, as `FileExtension::initialized` starts
        // it, so the index is warm by the time anyone searches it.
        let files = crate::file_search::FileSearch::start(
            compass_core::file_search::IndexingSettings::from_preferences(
                config.provider_preferences(compass_core::file_search::PREFERENCES_PROVIDER_ID),
                compass_core::xdg_dirs::home_dir().as_deref(),
            ),
        );

        Self {
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
            shell: None,
            views: Arc::default(),
            files: Arc::new(files),
        }
    }

    /// Opens the launch-history store, degrading to in-memory.
    ///
    /// Not fatal on failure: the launcher works without launch history, just
    /// less well ordered, and refusing to start over a corrupt ranking cache
    /// would be a worse failure than the one it is reporting.
    fn open_frecency() -> Box<dyn FrecencyStore + Send + Sync> {
        let clock = std::sync::Arc::new(SystemClock);
        match frecency_path() {
            Some(path) => match JsonFrecencyStore::open(&path) {
                Ok(store) => Box::new(store),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(), error = %err,
                        "launch history unreadable; ranking by match score alone this session"
                    );
                    Box::new(JsonFrecencyStore::in_memory(clock))
                }
            },
            None => {
                tracing::warn!("no XDG data home; launch history will not persist");
                Box::new(JsonFrecencyStore::in_memory(clock))
            }
        }
    }

    /// Builds state around an already-built index, for tests.
    #[must_use]
    pub fn with_index(
        index: AppIndex,
        frecency: Box<dyn FrecencyStore + Send + Sync>,
        socket: SocketPath,
        max_results: usize,
    ) -> Self {
        Self {
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
            shell: None,
            views: Arc::default(),
            files: Arc::default(),
        }
    }

    /// Makes the Shell extension's client available to requests.
    pub fn set_shell(&mut self, shell: Arc<compass_shell::ShellClient>) {
        self.shell = Some(shell);
    }

    /// Makes clipboard history available to requests.
    pub fn set_clipboard(&mut self, store: Arc<crate::clipboard_service::ClipboardStore>) {
        self.clipboard = Some(store);
    }

    /// The slot holding the attached launcher window.
    ///
    /// Cloned rather than borrowed because the attach callback outlives any
    /// borrow of the state: it runs on the connection's own task for as long
    /// as that window lives.
    #[must_use]
    pub fn window_slot(&self) -> WindowSlot {
        Arc::clone(&self.window)
    }

    /// Number of indexed applications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Ranks `text` against the index.
    ///
    /// # Hits can be ordered "out of order" by score, and that is correct
    ///
    /// Ordering comes from the *combined* score — match quality plus the
    /// frecency boost — while the score reported on the wire is the match score
    /// alone, because that is what the protocol documents and what a client can
    /// interpret. So a frequently launched item with score 90 legitimately
    /// appears above a never-launched one with 95. A client that sorts by the
    /// reported score would undo the ranking; it should preserve the order it
    /// is given.
    #[must_use]
    pub fn query(&self, text: &str) -> Vec<QueryHit> {
        self.index
            .search_root_all(text, Some(self.frecency.as_ref()))
            .into_iter()
            .take(self.max_results)
            .map(|hit| match hit {
                compass_core::RootHit::App(ranked) => app_hit(&ranked),
                compass_core::RootHit::Command {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id(),
                    title: command.title.to_owned(),
                    subtitle: Some(command.subtitle.to_owned()),
                    score: match_score,
                },
                compass_core::RootHit::Extension {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    subtitle: Some(command.extension_title.clone()),
                    score: match_score,
                },
            })
            .collect()
    }
}

/// Runs a Power Management command through logind, or the desktop's session
/// manager for logout, answering with the command's own sentences.
async fn run_power_command(id: &str) -> Response {
    use compass_power::{Action, PowerManager};
    use std::os::unix::fs::MetadataExt;
    let Some(command) = compass_core::power_commands::command(id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no power command has that id",
        ));
    };
    let cannot = || {
        Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            command.cannot_message,
        ))
    };
    let failed = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "power command failed");
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            command.failed_message,
        ))
    };
    let system = match zbus::Connection::system().await {
        Ok(connection) => connection,
        Err(err) => return failed(&err),
    };
    let manager = PowerManager::new(system);
    // The owner of /proc/self is this process's uid, without `unsafe`.
    let uid = std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid());
    let checked = match id {
        "power-off" => Some(Action::PowerOff),
        "reboot" | "soft-reboot" => Some(Action::Reboot),
        "suspend" | "sleep" => Some(Action::Suspend),
        "hibernate" => Some(Action::Hibernate),
        _ => None,
    };
    if let Some(action) = checked {
        match manager.can(action).await {
            Ok(capability) if capability.is_offerable() => {}
            Ok(_) => return cannot(),
            Err(err) => return failed(&err),
        }
    }
    let result = match id {
        "sleep" => manager.sleep().await,
        "soft-reboot" => manager.soft_reboot().await,
        "lock" => manager.lock(uid).await,
        "logout" => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            match zbus::Connection::session().await {
                Ok(session) => {
                    manager
                        .logout(compass_power::logout_target(&desktop), &session, uid)
                        .await
                }
                Err(err) => return failed(&err),
            }
        }
        _ => match checked {
            Some(action) => manager.perform(action, true).await,
            None => return cannot(),
        },
    };
    match result {
        Ok(()) => Response::Ack,
        Err(err) => failed(&err),
    }
}

/// How long a Search Files query may wait on the indexer: under the
/// launcher's own request timeout, so it hears why rather than a timeout.
const FILE_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Opens a file with its default application, or shows it in the file
/// browser, through the same application database extensions open with.
async fn open_file(state: &Arc<RwLock<EngineState>>, path: String, reveal: bool) -> Response {
    use compass_worker_host::application_service::Apps;
    let target = std::path::PathBuf::from(&path);
    if !target.is_absolute() || !target.exists() {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no file exists at that path",
        ));
    }
    let apps = crate::extension_apps::EngineApps::new(
        &state.read().await.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    );
    if reveal {
        apps.show_in_file_browser(&path, true);
        return Response::Ack;
    }
    if apps.open_file(&target) {
        Response::Ack
    } else {
        Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "no application opens this kind of file",
        ))
    }
}

/// The player a media command last acted on, which the next one that names
/// none prefers while it is still running.
static LAST_PLAYER: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `pactl` on the host, bounded by the C++'s timeout.
struct HostPactl;

impl compass_core::audio_control::Pactl for HostPactl {
    fn run(&self, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use wait_timeout::ChildExt;
        let mut child = compass_platform_linux::host_command("pactl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        // Read while waiting, so a long sink list cannot fill the pipe.
        let reader = std::thread::spawn(move || {
            let mut out = String::new();
            stdout.read_to_string(&mut out).map(|_| out)
        });
        let timeout = std::time::Duration::from_millis(compass_core::audio_control::TIMEOUT_MS);
        match child.wait_timeout(timeout).ok()? {
            Some(status) if status.success() => reader.join().ok()?.ok(),
            Some(_) => None,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        }
    }
}

/// Runs a volume command through `pactl`, answering with the sentence the
/// C++ puts in its HUD, or refusing with its toast.
fn run_volume_command(id: &str) -> Result<String, &'static str> {
    use compass_core::media_commands::{
        VOLUME_DOWN_STEP, VOLUME_PRESETS, VOLUME_UP_STEP, mute_message, step_fraction,
        volume_hud_text,
    };
    let audio = compass_core::audio_control::PactlAudioControl::new(HostPactl);
    match id {
        "volume-up" | "volume-down" => {
            let step = if id == "volume-up" {
                VOLUME_UP_STEP
            } else {
                VOLUME_DOWN_STEP
            };
            audio
                .adjust_volume(step_fraction(step))
                .map(volume_hud_text)
                .ok_or("Failed to adjust volume")
        }
        "toggle-mute" => {
            if !audio.toggle_mute() {
                return Err("Failed to toggle mute");
            }
            Ok(mute_message(audio.is_muted(), audio.volume()))
        }
        _ => {
            let percent = VOLUME_PRESETS
                .iter()
                .map(|(percent, _)| *percent)
                .find(|percent| id.strip_prefix("volume-") == Some(&percent.to_string()))
                .ok_or("Failed to set volume")?;
            audio
                .set_volume(step_fraction(percent))
                .map(volume_hud_text)
                .ok_or("Failed to set volume")
        }
    }
}

/// Shows what the C++ puts in its HUD, as a short transient notification:
/// the launcher has already hidden.
async fn show_hud(text: &str) {
    let shown = notify_rust::Notification::new()
        .appname("Vicinae")
        .summary(text)
        .hint(notify_rust::Hint::Transient(true))
        .timeout(notify_rust::Timeout::Milliseconds(1500))
        .show_async()
        .await;
    if let Err(err) = shown {
        tracing::info!(%err, text, "HUD not shown");
    }
}

/// Runs a media command on the default player over MPRIS. What the C++ shows
/// in its HUD goes out as a short-lived notification, since the launcher has
/// already hidden.
async fn run_media_command(id: &str) -> Response {
    use compass_core::media_commands::{self, NoPlayer};
    let refuse =
        |message: String| Response::Error(ProtocolError::new(ErrorKind::Unsupported, message));
    let failed = |message: &str, err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "media command failed");
        Response::Error(ProtocolError::new(ErrorKind::Internal, message))
    };
    // Without media control, the media extension registers only the volume
    // commands.
    if media_commands::registered_commands(false)
        .iter()
        .any(|command| command == id)
    {
        let owned = id.to_owned();
        let answer = tokio::task::spawn_blocking(move || run_volume_command(&owned)).await;
        return match answer {
            Ok(Ok(hud)) => {
                show_hud(&hud).await;
                Response::Ack
            }
            Ok(Err(message)) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            Err(err) => failed("Failed to adjust volume", &err),
        };
    }
    let failure = match id {
        "play-pause" => "Failed to toggle playback",
        "next-track" => "Failed to skip to the next track",
        "previous-track" => "Failed to skip to the previous track",
        _ => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no media command has that id",
            ));
        }
    };
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => return failed(failure, &err),
    };
    let players = match control.players().await {
        Ok(players) => players,
        Err(err) => return failed(failure, &err),
    };
    let last = LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(index) = compass_media::default_player(&players, last.as_deref()) else {
        return refuse(media_commands::no_player_message(&NoPlayer::NothingRunning));
    };
    let found = &players[index];
    let player = media_commands::MediaPlayer {
        id: found.id.clone(),
        identity: found.identity.clone(),
        title: found.title.clone(),
        artist: found.artist.clone(),
        playing: found.status == compass_media::PlaybackStatus::Playing,
        can_go_next: found.can_go_next,
        can_go_previous: found.can_go_previous,
    };
    let (result, hud) = match id {
        "play-pause" => (
            control.play_pause(&player.id).await,
            media_commands::play_pause_message(&player),
        ),
        "next-track" => {
            if let Some(refusal) = media_commands::skip_refusal(&player, true) {
                return refuse(refusal);
            }
            (control.next(&player.id).await, "Next Track".to_owned())
        }
        _ => {
            if let Some(refusal) = media_commands::skip_refusal(&player, false) {
                return refuse(refusal);
            }
            (
                control.previous(&player.id).await,
                "Previous Track".to_owned(),
            )
        }
    };
    if let Err(err) = result {
        return failed(failure, &err);
    }
    *LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player.id);
    show_hud(&hud).await;
    Response::Ack
}

async fn run_extension_command(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    arguments_json: Option<String>,
) -> Response {
    let Some(command) = state.read().await.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let given = match arguments_json.as_deref().map(serde_json::from_str) {
        None => None,
        Some(Ok(serde_json::Value::Object(given))) => Some(given),
        Some(_) => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "the arguments are not a JSON object",
            ));
        }
    };
    let runtime = match crate::extension_runner::Runtime::locate() {
        Ok(runtime) => runtime,
        Err(reason) => return Response::Error(ProtocolError::new(ErrorKind::Unsupported, reason)),
    };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("vicinae"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Running extensions needs a data directory, and $XDG_DATA_HOME and $HOME are unset",
        ));
    };
    let storage = extension_storage(&data_dir).await;
    let stored = match storage.clone() {
        Some(storage) => {
            let extension = command.extension_id.clone();
            tokio::task::spawn_blocking(move || {
                crate::extension_runner::load_preferences(&storage, &extension)
            })
            .await
            .unwrap_or_default()
        }
        None => serde_json::Map::new(),
    };
    let preferences = match command.preferences_with(&stored) {
        Ok(preferences) => preferences,
        Err(missing) if storage.is_none() => {
            let names: Vec<&str> = missing.iter().map(|p| p.title.as_str()).collect();
            return Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                format!(
                    "{} needs {} set, and without a keyring Compass has nowhere safe to keep it",
                    command.title,
                    names.join(", ")
                ),
            ));
        }
        Err(_) => {
            return Response::ExtensionNeedsPreferences {
                title: command.title.clone(),
                fields: preference_fields(&command, &stored),
            };
        }
    };
    let arguments = match command.arguments_with(given.as_ref()) {
        Ok(arguments) => arguments,
        Err(_) => {
            return Response::ExtensionNeedsArguments {
                title: command.title.clone(),
                fields: argument_fields(&command, given.as_ref()),
            };
        }
    };
    let host = crate::extension_runner::Host {
        storage,
        shell: state.read().await.shell.clone(),
        views: Arc::clone(&state.read().await.views),
        preferences,
        arguments,
        apps: Some(crate::extension_apps::EngineApps::new(
            &state.read().await.index,
            compass_xdg::mimeapps::Lists::from_environment(),
            tokio::runtime::Handle::current(),
        )),
    };
    let started = tokio::task::spawn_blocking(move || {
        crate::extension_runner::start(&runtime, &command, &data_dir, host)
    })
    .await;
    match started {
        Ok(Ok(started)) => {
            let recorded = tokio::task::spawn_blocking({
                let state = Arc::clone(state);
                move || state.blocking_write().frecency.record_launch(&id)
            })
            .await;
            if !matches!(recorded, Ok(Ok(()))) {
                tracing::warn!("could not record running an extension command");
            }
            match started {
                crate::extension_runner::Started::Ran => Response::Ack,
                crate::extension_runner::Started::View(session) => {
                    Response::ExtensionStarted { session }
                }
            }
        }
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the extension task failed: {err}"),
        )),
    }
}

/// Every argument `command` takes, as the launcher's form draws it, with what
/// was already entered.
fn argument_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    given: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::ArgumentType;
    use compass_ipc::PreferenceFieldKind;
    command
        .arguments
        .iter()
        .map(|argument| compass_ipc::PreferenceField {
            name: argument.name.clone(),
            // Raycast labels an argument by its placeholder; it has no title.
            title: if argument.placeholder.is_empty() {
                argument.name.clone()
            } else {
                argument.placeholder.clone()
            },
            description: String::new(),
            placeholder: argument.placeholder.clone(),
            required: argument.required,
            kind: match argument.argument_type {
                ArgumentType::Text => PreferenceFieldKind::Text,
                ArgumentType::Password => PreferenceFieldKind::Password,
                ArgumentType::Dropdown => PreferenceFieldKind::Dropdown {
                    options: argument
                        .data
                        .iter()
                        .flatten()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
            },
            value_json: given
                .and_then(|given| given.get(&argument.name))
                .map(ToString::to_string),
        })
        .collect()
}

/// Every preference `command` reads, as the launcher's form draws it.
fn preference_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    stored: &serde_json::Map<String, serde_json::Value>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::PreferenceKind;
    use compass_ipc::PreferenceFieldKind;
    command
        .preferences
        .iter()
        .map(|preference| compass_ipc::PreferenceField {
            name: preference.name.clone(),
            title: preference.title.clone(),
            description: preference.description.clone(),
            placeholder: preference.placeholder.clone(),
            required: preference.required,
            kind: match &preference.kind {
                PreferenceKind::TextField => PreferenceFieldKind::Text,
                PreferenceKind::Password => PreferenceFieldKind::Password,
                PreferenceKind::Checkbox { label } => PreferenceFieldKind::Checkbox {
                    label: label.clone(),
                },
                PreferenceKind::Dropdown { options } => PreferenceFieldKind::Dropdown {
                    options: options
                        .iter()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
                PreferenceKind::AppPicker => PreferenceFieldKind::Unsupported {
                    declared: "appPicker".to_owned(),
                },
                PreferenceKind::FilePicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "file".to_owned(),
                },
                PreferenceKind::DirectoryPicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "directory".to_owned(),
                },
                PreferenceKind::Unknown { declared } => PreferenceFieldKind::Unsupported {
                    declared: declared.clone(),
                },
            },
            value_json: stored
                .get(&preference.name)
                .or(preference.default.as_ref())
                .map(ToString::to_string),
        })
        .collect()
}

async fn set_extension_preferences(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    values_json: String,
) -> Response {
    let Some(command) = state.read().await.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let values: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(&values_json) {
            Ok(values) => values,
            Err(err) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("values_json is not a JSON object: {err}"),
                ));
            }
        };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("vicinae"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "no data directory",
        ));
    };
    let Some(storage) = extension_storage(&data_dir).await else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "without a keyring Compass has nowhere safe to keep extension preferences",
        ));
    };
    let saved = tokio::task::spawn_blocking(move || {
        crate::extension_runner::save_preferences(&storage, &command.extension_id, &values)
    })
    .await;
    match saved {
        Ok(Ok(())) => Response::Ack,
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the preferences task failed: {err}"),
        )),
    }
}

/// How long an `ExtensionView` is held open waiting for a change.
const VIEW_POLL: std::time::Duration = std::time::Duration::from_secs(20);

async fn extension_view(state: &Arc<RwLock<EngineState>>, session: u64, after: u64) -> Response {
    let Some(mut watch) = state.read().await.views.watch(session) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no extension view is running with that session",
        ));
    };
    // A timeout is an answer too: the same version, so the launcher asks again.
    let _ = tokio::time::timeout(VIEW_POLL, watch.wait_for(|view| view.version > after)).await;
    let view = watch.borrow().clone();
    Response::ExtensionView {
        version: view.version,
        view_json: view.view,
        problem: view.problem,
        ended: view.ended,
        depth: view.depth,
        alert: view.alert,
        toast: view.toast,
    }
}

/// Extensions' local storage, keyed from Compass's own master key; `None`
/// (and local storage answered "not implemented") without a keyring.
async fn extension_storage(data_dir: &std::path::Path) -> Option<crate::extension_runner::Storage> {
    let keyring = match crate::clipboard_service::Oo7Store::connect().await {
        Ok(keyring) => keyring,
        Err(err) => {
            tracing::info!(error = %err, "no keyring; extension storage unavailable");
            return None;
        }
    };
    match crate::clipboard_service::master_key(&keyring).await {
        Ok(master) => Some(crate::extension_runner::Storage {
            path: data_dir.join(crate::extension_runner::STORAGE_DATABASE),
            key: compass_crypto::keys::derive_all(&master).database,
        }),
        Err(err) => {
            tracing::info!(error = %err, "no master key; extension storage unavailable");
            None
        }
    }
}

fn app_hit(ranked: &compass_core::apps::ApplicationRootHit<'_>) -> QueryHit {
    QueryHit {
        // The ENTRYPOINT id, not the desktop key. The protocol
        // documents this field as the "stable identifier of the
        // underlying root item", and `AppIndex` already builds one
        // (`app_root_item` -> `entrypoint_id(APPS_PROVIDER_ID, ...)`);
        // this put `AppItem::key` there instead, so the wire carried
        // `host--byobu.desktop` for the item every other part of the
        // system — and the C++ engine — calls
        // `applications:host--byobu`.
        id: ranked.entrypoint_id.to_owned(),
        title: ranked.item.display_name(),
        subtitle: None,
        // `match_score`, not `score`. `Ranked::score` is the combined
        // value that includes the frecency boost and is not bounded,
        // whereas the wire documents `0..=100` on compass-search's
        // scale. See the ordering note on `query`.
        score: ranked.match_score,
    }
}

/// The three requests that need a launcher window, when none is connected.
///
/// [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
/// makes the window a resident process that connects to this daemon, so the
/// honest answer is about a *connection* rather than about the build. That is
/// the more useful thing to be told: "start the window" is actionable, "this
/// engine is headless" was not.
///
/// The property this preserves is the one that matters. A client can still
/// tell "no window" from "the window was shown", which is the only thing
/// standing between an honest gap and a `toggle` that silently does nothing.
fn no_window(what: &str) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        format!(
            "cannot {what}: no launcher window is connected to this engine. \
             Start one with `vicinae ui`. Query and doctor work without it."
        ),
    ))
}

/// Forwards `command` to the attached window, if there is one.
///
/// Clears the slot when the push fails. That is **hygiene, not behaviour**: a
/// push to a dead socket fails anyway, so the refusal a client sees is the same
/// either way — a control confirmed the end-to-end tests pass with the clearing
/// removed. What it buys is releasing the file descriptor and not paying a
/// doomed write on every subsequent request. See [`WindowSlot`].
pub(crate) async fn forward(slot: &WindowSlot, command: WindowCommand, what: &str) -> Response {
    let mut guard = slot.lock().await;

    let Some(link) = guard.as_mut() else {
        return no_window(what);
    };

    match link.push(command).await {
        Ok(WindowOutcome::Shown | WindowOutcome::Hidden) => Response::Ack,
        Ok(WindowOutcome::Failed(reason)) => {
            // The window is alive and said no. Keeping the link is the point:
            // a compositor that refused one activation will very likely accept
            // the next, and dropping the window over it would turn a recoverable
            // refusal into a dead launcher.
            tracing::warn!(reason = %reason, "the launcher window refused a command");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("the launcher window could not {what}: {reason}"),
            ))
        }
        Err(err) => {
            tracing::info!(error = %err, "the launcher window went away");
            *guard = None;
            no_window(what)
        }
    }
}

/// Answers one request.
///
/// Separated from the serve loop so the whole request surface is testable
/// without a socket, and so the serve loop stays about lifecycle.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },

        Request::Query { text } => {
            let state = state.read().await;
            Response::QueryResults {
                hits: state.query(&text),
            }
        }

        Request::Doctor => {
            let (socket, engine) = {
                let state = state.read().await;
                (state.socket.clone(), Engine::Rust)
            };
            Response::DoctorReport {
                checks: doctor::run_on_this_machine(&socket, engine).await.checks,
            }
        }

        Request::RecordLaunch { key } => {
            let state = Arc::clone(state);
            // JSON persistence is blocking disk work. Keep it off the async
            // executor, and serialize updates through the daemon's store.
            tokio::task::spawn_blocking(move || {
                let mut state = state.blocking_write();
                if state.index.get(&key).is_none() && compass_core::commands::by_id(&key).is_none()
                {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        "launch key is neither an application nor a builtin command",
                    ));
                }
                match state.frecency.record_launch(&key) {
                    Ok(()) => Response::Ack,
                    Err(error) => {
                        tracing::warn!(%error, "could not persist launch history");
                        Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            "could not persist launch history",
                        ))
                    }
                }
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "launch history task failed");
                Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "launch history task failed",
                ))
            })
        }

        Request::ClipboardHistory { query, limit } => {
            if limit == 0 {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "a clipboard history request must ask for at least one entry",
                ));
            }
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            // SQLite is blocking I/O; keep it off the executor.
            match tokio::task::spawn_blocking(move || store.history(&query, limit)).await {
                Ok(Ok(entries)) => Response::ClipboardHistory { entries },
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard history task failed: {err}"),
                )),
            }
        }

        Request::ClipboardContent { id } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            match tokio::task::spawn_blocking(move || store.content(&id)).await {
                Ok(Ok(Some((mime_type, data)))) => Response::ClipboardContent { mime_type, data },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard content task failed: {err}"),
                )),
            }
        }

        Request::ListWindows => {
            // wlroots compositors list windows over Wayland; never on GNOME.
            if crate::wlroots::detect().await.is_some() {
                let state = state.read().await;
                if let Some(response) = crate::window_service::wlroots_list(&state.index).await {
                    return response;
                }
            }
            let (shell, index_state) = (state.read().await.shell.clone(), Arc::clone(state));
            let Some(shell) = shell else {
                return Response::Error(crate::window_service::no_bus("Window switching"));
            };
            match shell.list_windows().await {
                Ok(windows) => {
                    let state = index_state.read().await;
                    Response::Windows {
                        windows: crate::window_service::rows(
                            windows.into_iter().map(Into::into).collect(),
                            &state.index,
                        ),
                    }
                }
                Err(err) => {
                    Response::Error(crate::window_service::refusal(&err, "Window switching"))
                }
            }
        }

        Request::ActivateWindow { id } | Request::CloseWindow { id } => {
            let close = matches!(request, Request::CloseWindow { .. });
            let what = if close {
                "Closing a window"
            } else {
                "Switching to a window"
            };
            if let Some(response) = crate::window_service::wlroots_act(id, close, what).await {
                return response;
            }
            let Some(shell) = state.read().await.shell.clone() else {
                return Response::Error(crate::window_service::no_bus(what));
            };
            let id = compass_shell::WindowId(id);
            let done = if close {
                shell.close_window(id).await
            } else {
                shell.activate_window(id).await
            };
            match done {
                Ok(()) => Response::Ack,
                Err(err) => Response::Error(crate::window_service::refusal(&err, what)),
            }
        }

        Request::RunPowerCommand { id } => run_power_command(&id).await,
        Request::RunMediaCommand { id } => run_media_command(&id).await,

        Request::SearchFiles { query, category } => {
            let files = Arc::clone(&state.read().await.files);
            let category = category
                .as_deref()
                .and_then(compass_core::file_search::category_for_key);
            let search = tokio::task::spawn_blocking(move || files.search(&query, category));
            match tokio::time::timeout(FILE_SEARCH_TIMEOUT, search).await {
                Ok(Ok(Ok(found))) => Response::Files {
                    heading: found.heading.to_owned(),
                    files: found.files,
                },
                Ok(Ok(Err(message))) => {
                    Response::Error(ProtocolError::new(ErrorKind::Unsupported, message))
                }
                Ok(Err(err)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("file search task failed: {err}"),
                )),
                Err(_) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "the file indexer did not answer in time",
                )),
            }
        }

        Request::OpenFile { path, reveal } => open_file(state, path, reveal).await,
        Request::RunExtensionCommand { id, arguments_json } => {
            run_extension_command(state, id, arguments_json).await
        }
        Request::ExtensionView { session, after } => extension_view(state, session, after).await,
        Request::ExtensionEvent {
            session,
            handler,
            args_json,
        } => {
            let args: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
                Ok(args) => args,
                Err(err) => {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        format!("args_json is not a JSON array: {err}"),
                    ));
                }
            };
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.activate(session, &handler, &args))
                .await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension event task failed: {err}"),
                )),
            }
        }
        Request::ExtensionAlertAnswer { session, confirmed } => {
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.answer_alert(session, confirmed)).await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the alert answer task failed: {err}"),
                )),
            }
        }
        Request::SetExtensionPreferences { id, values_json } => {
            set_extension_preferences(state, id, values_json).await
        }
        Request::ExtensionPop { session } => {
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.pop(session)).await {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension pop task failed: {err}"),
                )),
            }
        }
        Request::CloseExtension { session } => {
            state.read().await.views.close(session);
            Response::Ack
        }
        Request::OAuthRedirect { url } => match crate::extension_runner::oauth_redirect(&url) {
            Ok(()) => Response::Ack,
            Err(reason) => Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason)),
        },

        Request::ClipboardSetPinned { .. } | Request::ClipboardRemove { .. } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            let changed = tokio::task::spawn_blocking(move || match request {
                Request::ClipboardSetPinned { id, pinned } => store.set_pinned(&id, pinned),
                Request::ClipboardRemove { id } => store.remove(&id),
                _ => unreachable!("matched above"),
            })
            .await;
            match changed {
                Ok(Ok(true)) => Response::Ack,
                Ok(Ok(false)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard update task failed: {err}"),
                )),
            }
        }

        Request::ClipboardPaste { id } => {
            const WHAT: &str = "Pasting";
            let (shell, store) = {
                let state = state.read().await;
                (state.shell.clone(), state.clipboard.clone())
            };
            let Some(shell) = shell else {
                return Response::Error(crate::window_service::no_bus(WHAT));
            };
            let Some(store) = store else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            let (mime_type, data) =
                match tokio::task::spawn_blocking(move || store.content(&id)).await {
                    Ok(Ok(Some(content))) => content,
                    Ok(Ok(None)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::BadRequest,
                            "no clipboard history entry has that id",
                        ));
                    }
                    Ok(Err(err)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            err.to_string(),
                        ));
                    }
                    Err(err) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            format!("clipboard content task failed: {err}"),
                        ));
                    }
                };
            let terminals = {
                let state = state.read().await;
                compass_core::app_service::AppService::new(&state.index).terminal_window_classes()
            };
            let terminals: Vec<&str> = terminals.iter().map(String::as_str).collect();
            let content = compass_shell::ClipboardContent::binary(data, mime_type);
            let pasted = match shell.set_clipboard(&content).await {
                Ok(()) => shell.paste(&terminals).await,
                Err(err) => Err(err),
            };
            match pasted {
                Ok(()) => Response::Ack,
                Err(err) => Response::Error(crate::window_service::refusal(&err, WHAT)),
            }
        }

        // Handled by the serve loop, which owns the shutdown signal; reaching
        // here means the loop did not intercept it.
        Request::Shutdown => Response::ShuttingDown,

        Request::Toggle | Request::Show | Request::Hide => {
            let (slot, command, what) = {
                let state = state.read().await;
                match request {
                    Request::Toggle => (
                        state.window_slot(),
                        WindowCommand::Toggle,
                        "toggle a window",
                    ),
                    Request::Show => (state.window_slot(), WindowCommand::Show, "show a window"),
                    _ => (state.window_slot(), WindowCommand::Hide, "hide a window"),
                }
            };
            // The read lock is released before the push: a window that takes a
            // moment to answer must not block queries from other clients.
            forward(&slot, command, what).await
        }

        // Accepting is the whole decision: the transport reads this response,
        // writes it, and then hands the connection over as a `WindowLink`. The
        // engine never refuses an attach — see `WindowSlot` for why replacing
        // beats refusing.
        Request::AttachWindow => Response::WindowAttached,

        // An outcome is a *reply* on an attached connection, never a request.
        // Arriving here means a peer sent one on an ordinary connection, which
        // is a confused client rather than a window.
        Request::WindowOutcome(outcome) => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!(
                "{outcome:?} is a reply to a pushed window command, not a request;                  send AttachWindow first and answer the commands that follow"
            ),
        )),
    }
}

/// Binds the socket and serves until shutdown.
///
/// Returns when a client sends [`Request::Shutdown`] or the process is asked to
/// terminate. The socket file is removed on the way out by [`Listener`]'s drop.
///
/// `hotkey` asks the engine to bind the global launcher shortcut. Turning it
/// off is not only a test affordance: on GNOME, binding means a permission
/// prompt, and a user whose compositor already binds a key to `vicinae toggle`
/// should not be asked for one they will not use.
pub async fn run(socket: &SocketPath, hotkey: bool) -> Result<()> {
    // Before binding, not after: on the `/tmp` fallback the directory may
    // already exist and belong to someone else, and `DirBuilder::recursive`
    // adopts a directory rather than correcting it. See #88.
    socket
        .ensure_private_parent()
        .with_context(|| format!("checking the engine socket directory for {socket}"))?;

    let listener = Listener::bind(socket.as_path())
        .await
        .with_context(|| format!("binding the engine socket at {socket}"))?;

    let state = Arc::new(RwLock::new(EngineState::from_environment(socket.clone())));
    tracing::info!(socket = %socket, "engine listening");

    // A channel rather than a flag: `Shutdown` must be answered *before* the
    // loop stops, or the client sees its connection drop and reports a crash
    // instead of a clean stop.
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);

    let window_slot = state.read().await.window_slot();

    // Detached: the hotkey is a convenience, the socket is the contract. A
    // portal that never answers must not keep the engine from listening, so
    // this is spawned rather than awaited or raced against the serve loop.
    // Detached for the same reason: a keyring that is slow or absent costs
    // clipboard history, never the socket.
    tokio::spawn(crate::clipboard_service::run(Arc::clone(&state)));

    // The Shell extension, for window switching. Connecting only fails with
    // no session bus at all; an absent extension is reported per request.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            match compass_shell::ShellClient::connect_session().await {
                Ok(client) => state.write().await.set_shell(Arc::new(client)),
                Err(err) => {
                    tracing::warn!(error = %err, "no session bus; window switching unavailable")
                }
            }
        });
    }

    if hotkey {
        tokio::spawn(crate::hotkey::run(Arc::clone(&state)));
    } else {
        tracing::info!("not binding the launcher hotkey (--no-hotkey)");
    }

    let serving = {
        let state = Arc::clone(&state);
        listener.serve_with_shutdown_attach(
            move |request| {
                let state = Arc::clone(&state);
                let stop_tx = stop_tx.clone();
                async move {
                    if matches!(request, Request::Shutdown) {
                        tracing::info!("shutdown requested by a client");
                        // Buffered, so the response is written first and this
                        // never blocks the handler.
                        let _ = stop_tx.try_send(());
                        return Response::ShuttingDown;
                    }
                    handle(&state, request).await
                }
            },
            move |link| {
                let window_slot = Arc::clone(&window_slot);
                async move {
                    tracing::info!("a launcher window attached");
                    // Replaces any previous window; see `WindowSlot`. The old
                    // link drops here, closing that window's socket.
                    let replaced = window_slot.lock().await.replace(link).is_some();
                    if replaced {
                        tracing::info!("the previous launcher window was replaced");
                    }
                }
            },
            async move {
                let _ = stop_rx.recv().await;
            },
        )
    };

    serving.await.context("serving the engine socket")?;
    tracing::info!("engine stopped");
    Ok(())
}
