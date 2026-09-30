//! Other applications' windows on a wlroots compositor.
//!
//! Two protocols, and they are not two spellings of one thing (`PLAN.md`
//! §3.1):
//!
//! - `zwlr_foreign_toplevel_manager_v1` lists windows, says which one is
//!   active, and can **activate** and **close** them. Sway, Hyprland, niri,
//!   labwc and river all carry it.
//! - `ext_foreign_toplevel_list_v1` only lists: its requests are `stop` and
//!   `destroy`. A compositor with nothing else still gets a switcher that
//!   shows windows, and says plainly that it cannot switch to them.
//!
//! The management protocol is preferred whenever it is advertised. The two
//! lists are never merged — the protocols share no identifier, so pairing
//! their entries would be a guess.
//!
//! # Threading
//!
//! One connection and one thread own the event queue and keep the list
//! current; [`Toplevels`] is a cheap handle that reads a snapshot and sends
//! requests. `wayland-client` proxies are `Send + Sync` and a request from
//! another thread is simply flushed after it is queued, so there is no command
//! channel to keep in step.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// `zwlr_foreign_toplevel_handle_v1.state`'s `activated` value.
const STATE_ACTIVATED: u32 = 2;

/// Which protocol the list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `zwlr_foreign_toplevel_manager_v1`: list, activate, close.
    WlrManagement,
    /// `ext_foreign_toplevel_list_v1`: list only.
    ExtList,
}

impl Source {
    /// Whether windows from this source can be focused and closed.
    #[must_use]
    pub const fn can_act(self) -> bool {
        matches!(self, Self::WlrManagement)
    }
}

/// One window, as the compositor last described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toplevel {
    /// This process's handle for it; stable for the window's lifetime.
    pub id: u32,
    /// Its title.
    pub title: String,
    /// Its `app_id` — the Wayland counterpart of `WM_CLASS`.
    pub app_id: String,
    /// Whether it is the active window. Always `false` from the ext list,
    /// which carries no state.
    pub activated: bool,
    /// Whether it can be activated and closed.
    pub can_act: bool,
}

/// Why a window request failed.
#[derive(Debug, thiserror::Error)]
pub enum ToplevelError {
    /// No compositor to connect to.
    #[error("no Wayland compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// The compositor advertises neither toplevel protocol.
    #[error(
        "the compositor advertises neither zwlr_foreign_toplevel_manager_v1 nor \
         ext_foreign_toplevel_list_v1"
    )]
    Unsupported,
    /// Only the list protocol is available, so windows cannot be acted on.
    #[error(
        "the compositor only lists windows (ext_foreign_toplevel_list_v1); switching and closing \
         need zwlr_foreign_toplevel_manager_v1"
    )]
    ListOnly,
    /// No window has that id any more.
    #[error("no window with id {0}")]
    NoSuchWindow(u32),
    /// There is no seat to activate on.
    #[error("the compositor has no seat to activate a window on")]
    NoSeat,
    /// The connection failed.
    #[error("Wayland connection: {0}")]
    Connection(String),
}

#[derive(Debug, Clone)]
enum Handle {
    Wlr(ZwlrForeignToplevelHandleV1),
    Ext,
}

#[derive(Debug, Clone)]
struct Entry {
    handle: Handle,
    toplevel: Toplevel,
    /// When it last became active; the list is most-recently-used first.
    last_active: u64,
    /// Order of first appearance, the tie-break.
    created: u64,
    /// Whether a `done` has arrived: before it the entry is half described.
    ready: bool,
}

#[derive(Debug, Default)]
struct Snapshot {
    entries: HashMap<ObjectId, Entry>,
    seat: Option<wl_seat::WlSeat>,
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    changes: watch::Sender<u64>,
    running: AtomicBool,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        // Every write is a single field update, so a poisoned lock is still
        // consistent enough to read.
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn notify(&self) {
        self.changes.send_modify(|generation| *generation += 1);
    }
}

/// Pending edits to one window, applied atomically on `done`.
#[derive(Debug, Default, Clone)]
struct Pending {
    title: Option<String>,
    app_id: Option<String>,
    activated: Option<bool>,
}

struct State {
    shared: Arc<Shared>,
    source: Source,
    pending: HashMap<ObjectId, Pending>,
    next_id: u32,
    clock: u64,
}

impl State {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn add(&mut self, handle: Handle, object: ObjectId) {
        self.next_id += 1;
        let created = self.tick();
        let entry = Entry {
            handle,
            toplevel: Toplevel {
                id: self.next_id,
                title: String::new(),
                app_id: String::new(),
                activated: false,
                can_act: self.source.can_act(),
            },
            last_active: 0,
            created,
            ready: false,
        };
        self.shared.lock().entries.insert(object, entry);
    }

    fn pending(&mut self, object: &ObjectId) -> &mut Pending {
        self.pending.entry(object.clone()).or_default()
    }

    fn done(&mut self, object: &ObjectId) {
        let pending = self.pending.remove(object).unwrap_or_default();
        let now = self.tick();
        {
            let mut snapshot = self.shared.lock();
            let Some(entry) = snapshot.entries.get_mut(object) else {
                return;
            };
            if let Some(title) = pending.title {
                entry.toplevel.title = title;
            }
            if let Some(app_id) = pending.app_id {
                entry.toplevel.app_id = app_id;
            }
            if let Some(activated) = pending.activated {
                if activated && !entry.toplevel.activated {
                    entry.last_active = now;
                }
                entry.toplevel.activated = activated;
            }
            entry.ready = true;
        }
        self.shared.notify();
    }

    fn closed(&mut self, object: &ObjectId) {
        self.pending.remove(object);
        let removed = self.shared.lock().entries.remove(object);
        if removed.is_some() {
            self.shared.notify();
        }
    }
}

/// The window list of a wlroots compositor, kept current on its own thread.
#[derive(Clone)]
pub struct Toplevels {
    shared: Arc<Shared>,
    connection: Connection,
    source: Source,
}

impl std::fmt::Debug for Toplevels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toplevels")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Toplevels {
    /// Connects to the compositor in `WAYLAND_DISPLAY` and starts following
    /// its windows.
    ///
    /// # Errors
    ///
    /// [`ToplevelError::Connect`] with no compositor,
    /// [`ToplevelError::Unsupported`] when it has neither protocol.
    pub fn connect() -> Result<Self, ToplevelError> {
        Self::connect_to(Connection::connect_to_env()?)
    }

    /// Follows the windows of the compositor behind `connection`.
    ///
    /// Returns once the initial list has arrived, so the first
    /// [`Toplevels::list`] is already complete.
    ///
    /// # Errors
    ///
    /// As [`Toplevels::connect`].
    pub fn connect_to(connection: Connection) -> Result<Self, ToplevelError> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|err| ToplevelError::Connection(err.to_string()))?;
        let qh = queue.handle();

        let (changes, _) = watch::channel(0);
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            changes,
            running: AtomicBool::new(true),
        });

        let source = if globals
            .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
            .is_ok()
        {
            Source::WlrManagement
        } else if globals
            .bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ())
            .is_ok()
        {
            Source::ExtList
        } else {
            return Err(ToplevelError::Unsupported);
        };
        shared.lock().seat = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=7, ()).ok();

        let mut state = State {
            shared: Arc::clone(&shared),
            source,
            pending: HashMap::new(),
            next_id: 0,
            clock: 0,
        };
        // Two round trips: the first delivers the `toplevel` events, the
        // second the title/app_id/done each new handle sends.
        for _ in 0..2 {
            queue
                .roundtrip(&mut state)
                .map_err(|err| ToplevelError::Connection(err.to_string()))?;
        }

        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("compass-toplevels".to_owned())
            .spawn(move || follow(queue, state, &thread_shared))
            .map_err(|err| ToplevelError::Connection(err.to_string()))?;

        Ok(Self {
            shared,
            connection,
            source,
        })
    }

    /// Which protocol the list comes from.
    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }

    /// Whether the compositor connection is still being followed.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    /// A receiver that changes whenever the window set does.
    #[must_use]
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.shared.changes.subscribe()
    }

    /// The windows, most recently active first.
    ///
    /// A window whose first description has not finished arriving is left
    /// out rather than listed with an empty title.
    #[must_use]
    pub fn list(&self) -> Vec<Toplevel> {
        let snapshot = self.shared.lock();
        let mut entries: Vec<&Entry> = snapshot.entries.values().filter(|e| e.ready).collect();
        entries.sort_by(|a, b| {
            b.last_active
                .cmp(&a.last_active)
                .then(a.created.cmp(&b.created))
        });
        entries.into_iter().map(|e| e.toplevel.clone()).collect()
    }

    fn handle(&self, id: u32) -> Result<(Handle, Option<wl_seat::WlSeat>), ToplevelError> {
        let snapshot = self.shared.lock();
        let entry = snapshot
            .entries
            .values()
            .find(|entry| entry.toplevel.id == id)
            .ok_or(ToplevelError::NoSuchWindow(id))?;
        Ok((entry.handle.clone(), snapshot.seat.clone()))
    }

    /// Focus and raise a window.
    ///
    /// # Errors
    ///
    /// [`ToplevelError::ListOnly`] on the ext list, [`ToplevelError::NoSuchWindow`]
    /// for a window that has closed, [`ToplevelError::NoSeat`] without a seat.
    pub fn activate(&self, id: u32) -> Result<(), ToplevelError> {
        match self.handle(id)? {
            (Handle::Wlr(handle), Some(seat)) => {
                handle.activate(&seat);
                self.flush()
            }
            (Handle::Wlr(_), None) => Err(ToplevelError::NoSeat),
            (Handle::Ext, _) => Err(ToplevelError::ListOnly),
        }
    }

    /// Ask a window to close. The application may refuse, as with a click on
    /// its close button.
    ///
    /// # Errors
    ///
    /// As [`Toplevels::activate`], less the seat.
    pub fn close(&self, id: u32) -> Result<(), ToplevelError> {
        match self.handle(id)? {
            (Handle::Wlr(handle), _) => {
                handle.close();
                self.flush()
            }
            (Handle::Ext, _) => Err(ToplevelError::ListOnly),
        }
    }

    fn flush(&self) -> Result<(), ToplevelError> {
        self.connection
            .flush()
            .map_err(|err| ToplevelError::Connection(err.to_string()))
    }
}

fn follow(mut queue: EventQueue<State>, mut state: State, shared: &Shared) {
    loop {
        if let Err(err) = queue.blocking_dispatch(&mut state) {
            tracing::warn!(error = %err, "lost the compositor's window list");
            break;
        }
        if !shared.running.load(Ordering::Acquire) {
            break;
        }
    }
    shared.running.store(false, Ordering::Release);
    shared.lock().entries.clear();
    shared.notify();
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                let object = toplevel.id();
                state.add(Handle::Wlr(toplevel), object);
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.shared.running.store(false, Ordering::Release);
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let object = handle.id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.pending(&object).title = Some(title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.pending(&object).app_id = Some(app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: raw } => {
                state.pending(&object).activated = Some(states(&raw).any(|s| s == STATE_ACTIVATED));
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => state.done(&object),
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.closed(&object);
                handle.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                let object = toplevel.id();
                state.add(Handle::Ext, object);
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                state.shared.running.store(false, Ordering::Release);
            }
            _ => {}
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let object = handle.id();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.pending(&object).title = Some(title);
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.pending(&object).app_id = Some(app_id);
            }
            ext_foreign_toplevel_handle_v1::Event::Done => state.done(&object),
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.closed(&object);
                handle.destroy();
            }
            _ => {}
        }
    }
}

/// The `state` array: native-endian `u32`s, per the Wayland wire format.
fn states(raw: &[u8]) -> impl Iterator<Item = u32> + '_ {
    raw.chunks_exact(4)
        .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_array_is_read_as_native_u32s() {
        let raw: Vec<u8> = [0_u32, STATE_ACTIVATED, 3]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(states(&raw).collect::<Vec<_>>(), [0, STATE_ACTIVATED, 3]);
        // A truncated trailing value is ignored rather than misread.
        assert_eq!(states(&raw[..5]).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn only_the_management_protocol_can_act() {
        assert!(Source::WlrManagement.can_act());
        assert!(!Source::ExtList.can_act());
    }
}
