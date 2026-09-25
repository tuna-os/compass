//! KWin, the KDE Plasma compositor, over KWin scripting on the session bus.
//!
//! Ports `src/server/src/services/window-manager/kde/`. KWin has no socket
//! and no D-Bus call that lists windows; what it has is a JavaScript engine
//! reachable over `org.kde.kwin.Scripting`. So, as the C++ does, the engine
//! owns `org.vicinae.WindowTracker` on the session bus, hands KWin a tracker
//! script that walks `workspace.stackingOrder` once and then forwards every
//! window added, removed, retitled and activated to that name with
//! `callDBus`, and answers from the cache the calls build. Acting on a window
//! (focus, close, fullscreen) is a one-shot script loaded, run and unloaded.
//!
//! Beyond the C++: the tracker also reports each window's virtual desktop and
//! whether it is fullscreen; the desktops themselves are read and switched
//! through KWin's own `org.kde.KWin.VirtualDesktopManager`; a window can be
//! closed and made fullscreen; and the overview is KWin's `Overview` global
//! shortcut, invoked through `org.kde.kglobalaccel`. KWin floats every window
//! already, so there is no floating toggle.
//!
//! KWin's window ids are UUIDs; the rest of the engine speaks `u32` window
//! handles, so each UUID is given a number the first time it is seen, never
//! reused for the life of the process.
//!
//! [`Kwin`] is async inside and offers the same blocking calls as the other
//! providers: each runs on the runtime it was started on and is waited for,
//! with [`super::REQUEST_TIMEOUT`], so call those from a blocking thread.

use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::io::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::StreamExt as _;

use super::{IpcError, OwnWindows, REQUEST_TIMEOUT, WmWindow, WmWorkspace};

/// KWin's bus name.
pub const KWIN_SERVICE: &str = "org.kde.KWin";
/// Where KWin's scripting interface lives.
pub const SCRIPTING_PATH: &str = "/Scripting";
/// KWin's scripting interface: `loadScript`, `unloadScript`.
pub const SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
/// A loaded script's interface: `run`.
pub const SCRIPT_INTERFACE: &str = "org.kde.kwin.Script";
/// The name the tracker script reports to, as the C++ owns it.
pub const TRACKER_SERVICE: &str = "org.vicinae.WindowTracker";
/// Where the tracker object is served.
pub const TRACKER_PATH: &str = "/";
/// The tracker's interface.
pub const TRACKER_INTERFACE: &str = "org.vicinae.WindowTracker";
/// The plugin name the tracker script is loaded under.
pub const TRACKER_PLUGIN: &str = "vicinae-window-tracker";
/// Where KWin's virtual-desktop interface lives.
pub const DESKTOPS_PATH: &str = "/VirtualDesktopManager";
/// KWin's virtual-desktop interface: `desktops`, `current`.
pub const DESKTOPS_INTERFACE: &str = "org.kde.KWin.VirtualDesktopManager";
/// The global-shortcut daemon.
pub const KGLOBALACCEL_SERVICE: &str = "org.kde.kglobalaccel";
/// KWin's component in it.
pub const KWIN_COMPONENT_PATH: &str = "/component/kwin";
/// A component's interface: `invokeShortcut`.
pub const COMPONENT_INTERFACE: &str = "org.kde.kglobalaccel.Component";
/// The name of KWin's overview shortcut.
pub const OVERVIEW_SHORTCUT: &str = "Overview";

/// The tracker script. Seeds the cache from `stackingOrder`, then follows
/// KWin's window signals for the rest of its session. The C++ script, with
/// each window's first desktop and fullscreen state added to `add`, and
/// `add` re-sent when either changes.
pub const TRACKER_JS: &str = r#"
(function () {
  const SVC = "org.vicinae.WindowTracker";
  const P = "/";
  const IF = "org.vicinae.WindowTracker";

  function desktopOf(w) {
    if (w.onAllDesktops) return "";
    const d = w.desktops;
    return d && d.length > 0 ? String(d[0].id) : "";
  }

  function push(w) {
    if (!w || !w.normalWindow) return;
    callDBus(SVC, P, IF, "add",
      String(w.internalId),
      String(w.resourceClass || ""),
      String(w.resourceName || ""),
      String(w.caption || ""),
      w.pid | 0,
      desktopOf(w),
      w.fullScreen ? 1 : 0);
  }

  function hook(w) {
    if (!w || !w.normalWindow) return;
    push(w);
    const again = function () { push(w); };
    if (w.captionChanged) w.captionChanged.connect(again);
    if (w.desktopsChanged) w.desktopsChanged.connect(again);
    if (w.fullScreenChanged) w.fullScreenChanged.connect(again);
  }

  try {
    const list = workspace.stackingOrder;
    for (let i = 0; i < list.length; ++i) hook(list[i]);

    const a = workspace.activeWindow;
    callDBus(SVC, P, IF, "activated", a ? String(a.internalId) : "");

    workspace.windowAdded.connect(hook);
    workspace.windowRemoved.connect(function (w) {
      if (w) callDBus(SVC, P, IF, "remove", String(w.internalId));
    });
    workspace.windowActivated.connect(function (w) {
      callDBus(SVC, P, IF, "activated", w ? String(w.internalId) : "");
    });
  } catch (e) {
    callDBus(SVC, P, IF, "error", String(e));
  }
})();
"#;

/// The one-shot script: `__TARGET__` and `__ACTION__` are replaced with
/// JavaScript string literals before it is loaded.
const ONE_SHOT_JS: &str = r#"
(function () {
  const target = __TARGET__;
  const action = __ACTION__;
  const list = workspace.stackingOrder;
  for (let i = 0; i < list.length; ++i) {
    const w = list[i];
    if (!w || String(w.internalId) !== target) continue;
    if (action === "focus") workspace.activeWindow = w;
    else if (action === "close") w.closeWindow();
    else if (action === "fullscreen") w.fullScreen = !w.fullScreen;
    return;
  }
})();
"#;

/// What a one-shot script does to its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `workspace.activeWindow = w` (the C++ focus script).
    Focus,
    /// `w.closeWindow()`.
    Close,
    /// `w.fullScreen = !w.fullScreen`.
    Fullscreen,
}

impl Action {
    /// Its name in the script.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Close => "close",
            Self::Fullscreen => "fullscreen",
        }
    }
}

/// The one-shot script that does `action` to the window KWin knows as
/// `internal_id`.
#[must_use]
pub fn one_shot_script(internal_id: &str, action: Action) -> String {
    let literal = |text: &str| serde_json::Value::from(text).to_string();
    ONE_SHOT_JS
        .replace("__TARGET__", &literal(internal_id))
        .replace("__ACTION__", &literal(action.as_str()))
}

/// Whether this is a Plasma session on Wayland, from the environment: a
/// `kde` entry in `$XDG_CURRENT_DESKTOP` (`Environment::isPlasmaDesktop`)
/// and a Wayland display (`isWaylandSession`).
#[must_use]
pub fn is_plasma_wayland(var: impl Fn(&str) -> Option<OsString>) -> bool {
    let plasma = var("XDG_CURRENT_DESKTOP").is_some_and(|desktops| {
        desktops
            .to_string_lossy()
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("kde"))
    });
    plasma && var("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
}

/// Why the tracker could not be started.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The bus refused.
    #[error("{0}")]
    Bus(#[from] zbus::Error),
    /// Another process owns the tracker's name.
    #[error("{TRACKER_SERVICE} is owned by another process (another vicinae running?)")]
    NameTaken,
    /// Not started inside a Tokio runtime.
    #[error("the KWin provider needs a Tokio runtime")]
    NoRuntime,
}

#[derive(Debug, Clone)]
struct Tracked {
    internal_id: String,
    handle: u32,
    resource_class: String,
    caption: String,
    pid: i32,
    desktop: String,
    fullscreen: bool,
}

/// What the tracker script has told us.
#[derive(Debug, Default)]
struct Cache {
    /// In the order KWin first reported them.
    windows: Vec<Tracked>,
    focused: Option<String>,
    /// Internal ids, most recently activated first.
    recent: Vec<String>,
    handles: HashMap<String, u32>,
    next_handle: u32,
}

impl Cache {
    fn reset(&mut self) {
        self.windows.clear();
        self.focused = None;
        self.recent.clear();
    }

    fn handle_for(&mut self, internal_id: &str) -> u32 {
        if let Some(handle) = self.handles.get(internal_id) {
            return *handle;
        }
        self.next_handle += 1;
        self.handles
            .insert(internal_id.to_owned(), self.next_handle);
        self.next_handle
    }

    fn internal_id(&self, handle: &str) -> Option<String> {
        let handle: u32 = handle.parse().ok()?;
        self.windows
            .iter()
            .find(|window| window.handle == handle)
            .map(|window| window.internal_id.clone())
    }

    fn window(&self, tracked: &Tracked) -> WmWindow {
        WmWindow {
            id: tracked.handle.to_string(),
            title: tracked.caption.clone(),
            wm_class: tracked.resource_class.clone(),
            pid: u32::try_from(tracked.pid).ok().filter(|pid| *pid > 0),
            workspace: (!tracked.desktop.is_empty()).then(|| tracked.desktop.clone()),
            bounds: None,
            focused: self.focused.as_deref() == Some(tracked.internal_id.as_str()),
            fullscreen: tracked.fullscreen,
        }
    }
}

/// The object the tracker script calls.
struct TrackerObject {
    cache: Arc<Mutex<Cache>>,
}

fn lock(cache: &Mutex<Cache>) -> MutexGuard<'_, Cache> {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[zbus::interface(name = "org.vicinae.WindowTracker")]
impl TrackerObject {
    /// A window appeared, or changed its caption, desktop or fullscreen state.
    #[zbus(name = "add")]
    #[allow(clippy::too_many_arguments)]
    fn add(
        &self,
        id: String,
        resource_class: String,
        _resource_name: String,
        caption: String,
        pid: i32,
        desktop: String,
        fullscreen: i32,
    ) {
        let mut cache = lock(&self.cache);
        let handle = cache.handle_for(&id);
        let tracked = Tracked {
            internal_id: id,
            handle,
            resource_class,
            caption,
            pid,
            desktop,
            fullscreen: fullscreen != 0,
        };
        match cache
            .windows
            .iter_mut()
            .find(|known| known.internal_id == tracked.internal_id)
        {
            Some(known) => *known = tracked,
            None => cache.windows.push(tracked),
        }
    }

    /// A window closed.
    #[zbus(name = "remove")]
    fn remove(&self, id: String) {
        let mut cache = lock(&self.cache);
        cache.windows.retain(|window| window.internal_id != id);
        cache.recent.retain(|known| *known != id);
        if cache.focused.as_deref() == Some(id.as_str()) {
            cache.focused = None;
        }
    }

    /// Focus moved to `id`, or to nothing when it is empty.
    #[zbus(name = "activated")]
    fn activated(&self, id: String) {
        let mut cache = lock(&self.cache);
        if id.is_empty() {
            cache.focused = None;
            return;
        }
        cache.recent.retain(|known| *known != id);
        cache.recent.insert(0, id.clone());
        cache.focused = Some(id);
    }

    /// The script threw.
    #[zbus(name = "error")]
    fn error(&self, message: String) {
        tracing::warn!(%message, "KWin tracker script error");
    }
}

struct Inner {
    connection: zbus::Connection,
    cache: Arc<Mutex<Cache>>,
    runtime: tokio::runtime::Handle,
    watcher: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(watcher) = self
            .watcher
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            watcher.abort();
        }
    }
}

/// The KWin provider: the tracker's cache, and the scripts that act.
#[derive(Clone)]
pub struct Kwin {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Kwin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kwin")
            .field("windows", &lock(&self.inner.cache).windows.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Kwin {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Kwin {}

static ONE_SHOTS: AtomicU32 = AtomicU32::new(0);

impl Kwin {
    /// Serves the tracker on `connection`, owns its name, loads the tracker
    /// script into KWin, and reloads it whenever KWin comes back
    /// (`QDBusServiceWatcher` in the C++). KWin not answering yet is not an
    /// error: the tracker is loaded when it appears.
    ///
    /// # Errors
    ///
    /// When the tracker cannot be served or its name is owned by another
    /// process, or outside a Tokio runtime.
    pub async fn start(connection: zbus::Connection) -> Result<Self, StartError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| StartError::NoRuntime)?;
        let cache = Arc::new(Mutex::new(Cache::default()));
        connection
            .object_server()
            .at(
                TRACKER_PATH,
                TrackerObject {
                    cache: Arc::clone(&cache),
                },
            )
            .await?;
        let reply = connection
            .request_name_with_flags(
                TRACKER_SERVICE,
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await;
        match reply {
            Ok(
                zbus::fdo::RequestNameReply::PrimaryOwner
                | zbus::fdo::RequestNameReply::AlreadyOwner,
            ) => {}
            Ok(_) | Err(zbus::Error::NameTaken) => return Err(StartError::NameTaken),
            Err(err) => return Err(err.into()),
        }
        let kwin = Self {
            inner: Arc::new(Inner {
                connection,
                cache,
                runtime,
                watcher: Mutex::new(None),
            }),
        };
        let watcher = kwin.watch().await?;
        *kwin
            .inner
            .watcher
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(watcher);
        if let Err(err) = kwin.load_tracker().await {
            tracing::info!(error = %err, "KWin tracker not loaded yet");
        }
        Ok(kwin)
    }

    /// Follows KWin leaving and coming back: its windows are forgotten when
    /// it goes, and the tracker reloaded when it returns.
    async fn watch(&self) -> Result<tokio::task::JoinHandle<()>, zbus::Error> {
        let dbus = zbus::fdo::DBusProxy::new(&self.inner.connection).await?;
        let mut changes = dbus
            .receive_name_owner_changed_with_args(&[(0, KWIN_SERVICE)])
            .await?;
        let weak = Arc::downgrade(&self.inner);
        Ok(tokio::spawn(async move {
            while let Some(change) = changes.next().await {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let kwin = Self { inner };
                let Ok(args) = change.args() else {
                    continue;
                };
                if args.new_owner().is_some() {
                    if let Err(err) = kwin.load_tracker().await {
                        tracing::warn!(error = %err, "could not load the KWin tracker");
                    }
                } else {
                    lock(&kwin.inner.cache).reset();
                }
            }
        }))
    }

    /// Unloads the tracker script and gives up the tracker's name: the
    /// C++'s `aboutToQuit` handler.
    pub async fn stop(&self) {
        if let Err(err) = self.unload(TRACKER_PLUGIN).await {
            tracing::debug!(error = %err, "could not unload the KWin tracker");
        }
        let _ = self.inner.connection.release_name(TRACKER_SERVICE).await;
    }

    async fn scripting(&self) -> Result<zbus::Proxy<'static>, zbus::Error> {
        zbus::Proxy::new(
            &self.inner.connection,
            KWIN_SERVICE,
            SCRIPTING_PATH,
            SCRIPTING_INTERFACE,
        )
        .await
    }

    async fn unload(&self, plugin: &str) -> Result<bool, zbus::Error> {
        self.scripting()
            .await?
            .call("unloadScript", &(plugin,))
            .await
    }

    /// Writes `source` where KWin can read it, loads it as `plugin`, runs
    /// it, and returns once `run` has answered: KWin opens the file then,
    /// not at `loadScript`, so the file lives until after.
    async fn load_and_run(&self, source: &str, plugin: &str) -> Result<(), IpcError> {
        let mut file = tempfile::Builder::new()
            .prefix("vicinae-kwin-")
            .suffix(".js")
            .tempfile()?;
        file.write_all(source.as_bytes())?;
        file.flush()?;
        let path = file.path().to_string_lossy().into_owned();
        let id: i32 = self
            .scripting()
            .await
            .map_err(bus)?
            .call("loadScript", &(path.as_str(), plugin))
            .await
            .map_err(bus)?;
        if id < 0 {
            return Err(IpcError::Refused(format!(
                "KWin would not load {plugin} ({id})"
            )));
        }
        let script = zbus::Proxy::new(
            &self.inner.connection,
            KWIN_SERVICE,
            format!("{SCRIPTING_PATH}/Script{id}"),
            SCRIPT_INTERFACE,
        )
        .await
        .map_err(bus)?;
        let ran: Result<(), zbus::Error> = script.call("run", &()).await;
        drop(file);
        ran.map_err(bus)
    }

    /// (Re)loads the tracker, forgetting what the last one reported.
    async fn load_tracker(&self) -> Result<(), IpcError> {
        let _ = self.unload(TRACKER_PLUGIN).await;
        lock(&self.inner.cache).reset();
        self.load_and_run(TRACKER_JS, TRACKER_PLUGIN).await
    }

    /// Runs `action` on the window with the handle `id`.
    ///
    /// # Errors
    ///
    /// When no such window is tracked, or KWin refuses or cannot be reached.
    pub async fn act(&self, id: &str, action: Action) -> Result<(), IpcError> {
        let internal_id = lock(&self.inner.cache)
            .internal_id(id)
            .ok_or_else(|| IpcError::Refused(format!("no window {id}")))?;
        let plugin = format!(
            "vicinae-{}-{}-{}",
            action.as_str(),
            std::process::id(),
            ONE_SHOTS.fetch_add(1, Ordering::Relaxed)
        );
        let ran = self
            .load_and_run(&one_shot_script(&internal_id, action), &plugin)
            .await;
        let _ = self.unload(&plugin).await;
        ran
    }

    async fn desktops_proxy(&self) -> Result<zbus::Proxy<'static>, IpcError> {
        zbus::Proxy::new(
            &self.inner.connection,
            KWIN_SERVICE,
            DESKTOPS_PATH,
            DESKTOPS_INTERFACE,
        )
        .await
        .map_err(bus)
    }

    /// The virtual desktops, in KWin's order.
    ///
    /// # Errors
    ///
    /// When KWin cannot be reached.
    pub async fn desktops(&self) -> Result<Vec<WmWorkspace>, IpcError> {
        let rows: Vec<(u32, String, String)> = self
            .desktops_proxy()
            .await?
            .get_property("desktops")
            .await
            .map_err(bus)?;
        let cache = lock(&self.inner.cache);
        Ok(rows
            .into_iter()
            .map(|(position, id, name)| WmWorkspace {
                has_fullscreen: cache
                    .windows
                    .iter()
                    .any(|window| window.fullscreen && window.desktop == id),
                name: if name.is_empty() { id.clone() } else { name },
                number: i32::try_from(position).ok().map(|n| n + 1),
                monitor: None,
                id,
            })
            .collect())
    }

    /// The current virtual desktop.
    ///
    /// # Errors
    ///
    /// As [`Self::desktops`].
    pub async fn current_desktop(&self) -> Result<Option<WmWorkspace>, IpcError> {
        let current: String = self
            .desktops_proxy()
            .await?
            .get_property("current")
            .await
            .map_err(bus)?;
        Ok(self
            .desktops()
            .await?
            .into_iter()
            .find(|desktop| desktop.id == current))
    }

    /// Switches to the virtual desktop `id`.
    ///
    /// # Errors
    ///
    /// As [`Self::desktops`], and when KWin refuses.
    pub async fn switch_desktop(&self, id: &str) -> Result<(), IpcError> {
        self.desktops_proxy()
            .await?
            .set_property("current", id)
            .await
            .map_err(|err| IpcError::Refused(err.to_string()))
    }

    /// Invokes KWin's `Overview` shortcut.
    ///
    /// # Errors
    ///
    /// When the shortcut daemon cannot be reached.
    pub async fn overview(&self) -> Result<(), IpcError> {
        let component = zbus::Proxy::new(
            &self.inner.connection,
            KGLOBALACCEL_SERVICE,
            KWIN_COMPONENT_PATH,
            COMPONENT_INTERFACE,
        )
        .await
        .map_err(bus)?;
        component
            .call::<_, _, ()>("invokeShortcut", &(OVERVIEW_SHORTCUT,))
            .await
            .map_err(bus)
    }

    /// Every tracked window, in the order KWin reported them.
    #[must_use]
    pub fn windows(&self) -> Vec<WmWindow> {
        let cache = lock(&self.inner.cache);
        cache
            .windows
            .iter()
            .map(|tracked| cache.window(tracked))
            .collect()
    }

    /// The window KWin says is active.
    #[must_use]
    pub fn focused_window(&self) -> Option<WmWindow> {
        self.windows().into_iter().find(|window| window.focused)
    }

    /// The most recently activated window that is not one of `own`, on the
    /// desktop `current` or on all of them when `current` is given.
    #[must_use]
    pub fn recent_window(&self, own: &OwnWindows, current: Option<&str>) -> Option<WmWindow> {
        let cache = lock(&self.inner.cache);
        cache
            .recent
            .iter()
            .filter_map(|id| cache.windows.iter().find(|w| w.internal_id == *id))
            .map(|tracked| cache.window(tracked))
            .filter(|window| !own.contains(window))
            .find(|window| match (current, window.workspace.as_deref()) {
                (Some(current), Some(on)) => current == on,
                _ => true,
            })
    }

    /// Runs `work` on the provider's runtime and waits for it, for callers
    /// on a blocking thread.
    fn block<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, IpcError>> + Send + 'static,
    ) -> Result<T, IpcError> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.inner.runtime.spawn(async move {
            let _ = tx.send(work.await);
        });
        rx.recv_timeout(REQUEST_TIMEOUT).unwrap_or_else(|_| {
            Err(IpcError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "KWin did not answer in time",
            )))
        })
    }

    /// [`Self::act`], blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::act`].
    pub fn act_blocking(&self, id: &str, action: Action) -> Result<(), IpcError> {
        let (kwin, id) = (self.clone(), id.to_owned());
        self.block(async move { kwin.act(&id, action).await })
    }

    /// [`Self::desktops`], blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::desktops`].
    pub fn desktops_blocking(&self) -> Result<Vec<WmWorkspace>, IpcError> {
        let kwin = self.clone();
        self.block(async move { kwin.desktops().await })
    }

    /// [`Self::current_desktop`], blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::desktops`].
    pub fn current_desktop_blocking(&self) -> Result<Option<WmWorkspace>, IpcError> {
        let kwin = self.clone();
        self.block(async move { kwin.current_desktop().await })
    }

    /// [`Self::switch_desktop`], blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::switch_desktop`].
    pub fn switch_desktop_blocking(&self, id: &str) -> Result<(), IpcError> {
        let (kwin, id) = (self.clone(), id.to_owned());
        self.block(async move { kwin.switch_desktop(&id).await })
    }

    /// [`Self::overview`], blocking.
    ///
    /// # Errors
    ///
    /// As [`Self::overview`].
    pub fn overview_blocking(&self) -> Result<(), IpcError> {
        let kwin = self.clone();
        self.block(async move { kwin.overview().await })
    }

    /// Whether KWin owns its name on the bus (the C++ `ping`), blocking.
    #[must_use]
    pub fn ping(&self) -> bool {
        let kwin = self.clone();
        self.block(async move {
            let dbus = zbus::fdo::DBusProxy::new(&kwin.inner.connection)
                .await
                .map_err(bus)?;
            let name = zbus::names::BusName::try_from(KWIN_SERVICE)
                .map_err(|err| IpcError::Parse(err.to_string()))?;
            dbus.name_has_owner(name)
                .await
                .map_err(|err| IpcError::Refused(err.to_string()))
        })
        .unwrap_or(false)
    }
}

fn bus(err: zbus::Error) -> IpcError {
    match err {
        zbus::Error::MethodError(name, message, _) => IpcError::Refused(format!(
            "{name}{}",
            message.map(|m| format!(": {m}")).unwrap_or_default()
        )),
        other => IpcError::Io(std::io::Error::other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn plasma_on_wayland_is_a_kde_entry_and_a_display() {
        assert!(is_plasma_wayland(env(&[
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ])));
        assert!(is_plasma_wayland(env(&[
            ("XDG_CURRENT_DESKTOP", "foo:kde"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ])));
        assert!(
            !is_plasma_wayland(env(&[("XDG_CURRENT_DESKTOP", "KDE")])),
            "Plasma on X11 is the X11 provider's"
        );
        assert!(
            !is_plasma_wayland(env(&[
                ("XDG_CURRENT_DESKTOP", "KDEish"),
                ("WAYLAND_DISPLAY", "wayland-0"),
            ])),
            "the entry is compared whole"
        );
        assert!(!is_plasma_wayland(env(&[("WAYLAND_DISPLAY", "wayland-0")])));
    }

    #[test]
    fn a_one_shot_script_quotes_its_target() {
        let script = one_shot_script("{a\"b}", Action::Close);
        assert!(script.contains(r#"const target = "{a\"b}";"#), "{script}");
        assert!(script.contains(r#"const action = "close";"#), "{script}");
        assert!(!script.contains("__"), "{script}");
    }

    #[test]
    fn handles_are_numbered_once_and_never_reused() {
        let mut cache = Cache::default();
        assert_eq!(cache.handle_for("a"), 1);
        assert_eq!(cache.handle_for("b"), 2);
        assert_eq!(cache.handle_for("a"), 1);
        cache.reset();
        assert_eq!(cache.handle_for("c"), 3);
        assert_eq!(cache.handle_for("b"), 2);
    }
}
