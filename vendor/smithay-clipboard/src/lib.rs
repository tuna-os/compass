//! Smithay Clipboard
//!
//! Provides access to the Wayland clipboard for gui applications. The user
//! should have surface around.
//!
//! # The Compass patch
//!
//! Upstream 0.7.3 creates a `wl_data_device` (and a primary selection device)
//! when a seat gains a keyboard and releases them when it loses one, and its
//! `Drop` tears the whole worker down, releasing them too. Both toolkits that
//! Compass runs on drop and re-create their `Clipboard` with their windows
//! (`iced_winit` on the last window's close, `iced_layershell` whenever no
//! surface is left), so the launcher released its devices on every hide.
//!
//! Releasing a device on a connection shared with the UI is not safe. The
//! compositor can send `data_offer(new_id)` on the device before it has read
//! the release; libwayland (1.22, at least) discards an event to a zombie
//! proxy without reserving the server-allocated id it carries, so the client's
//! idea of the next server id falls behind, and the next `data_offer` on any
//! device fails with "not a valid new object id" — a fatal protocol error that
//! takes the launcher's whole connection down (Compass TIL-01, reproduced on
//! headless Sway with a keyboard coming and going while another client copies).
//!
//! So, here:
//!
//! - a device, once created for a seat, lives until the seat itself goes away,
//!   whatever happens to the seat's keyboard (`state.rs`);
//! - one worker serves each display for as long as it runs, and `Clipboard` is
//!   a handle to it: dropping one does not stop the worker, and the next
//!   `Clipboard::new` for the same display reuses it, devices and all.
//!
//! A side effect worth having: what the launcher copied just before hiding
//! stays on the clipboard, because the data source no longer goes with the
//! window.

#![deny(clippy::all, clippy::if_not_else, clippy::enum_glob_use)]
use std::ffi::c_void;
use std::io::Result;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use sctk::reexports::calloop::channel::{self, Sender};
use sctk::reexports::client::Connection;
use sctk::reexports::client::backend::Backend;

mod mime;
mod state;
mod worker;

/// One display's worker, shared by every `Clipboard` made for that display.
struct Worker {
    display: usize,
    request_sender: Sender<worker::Command>,
    request_receiver: Receiver<Result<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|thread| !thread.is_finished())
    }
}

/// The running workers, by display address. A worker whose connection died
/// has finished; a later `Clipboard::new` for that address replaces it.
static WORKERS: Mutex<Vec<Arc<Mutex<Worker>>>> = Mutex::new(Vec::new());

/// Access to a Wayland clipboard.
pub struct Clipboard {
    worker: Arc<Mutex<Worker>>,
}

impl Clipboard {
    /// Creates new clipboard which will be running on its own thread with its
    /// own event queue to handle clipboard requests.
    ///
    /// # Safety
    ///
    /// `display` must be a valid `*mut wl_display` pointer, and it must remain
    /// valid for as long as the connection is used: the worker it starts
    /// outlives this `Clipboard` (see the module docs).
    pub unsafe fn new(display: *mut c_void) -> Self {
        let address = display as usize;
        let mut workers = WORKERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        workers.retain(|worker| {
            worker.lock().map(|worker| worker.is_running()).unwrap_or(false)
        });
        if let Some(worker) = workers.iter().find(|worker| {
            worker.lock().map(|worker| worker.display == address).unwrap_or(false)
        }) {
            return Self { worker: worker.clone() };
        }

        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);

        // Create channel to send data to clipboard thread.
        let (request_sender, rx_chan) = channel::channel();
        // Create channel to get data from the clipboard thread.
        let (clipboard_reply_sender, request_receiver) = mpsc::channel();

        let name = String::from("smithay-clipboard");
        let thread = worker::spawn(name, connection, rx_chan, clipboard_reply_sender);

        let worker = Arc::new(Mutex::new(Worker {
            display: address,
            request_sender,
            request_receiver,
            thread,
        }));
        workers.push(worker.clone());
        Self { worker }
    }

    fn request(&self, request: worker::Command) -> Result<String> {
        let worker = self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = worker.request_sender.send(request);

        if let Ok(reply) = worker.request_receiver.recv() {
            reply
        } else {
            // The clipboard thread is dead, however we shouldn't crash downstream, so
            // propogating an error.
            Err(std::io::Error::other("clipboard is dead."))
        }
    }

    fn send(&self, request: worker::Command) {
        let worker = self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = worker.request_sender.send(request);
    }

    /// Load clipboard data.
    ///
    /// Loads content from a clipboard on a last observed seat.
    pub fn load(&self) -> Result<String> {
        self.request(worker::Command::Load)
    }

    /// Store to a clipboard.
    ///
    /// Stores to a clipboard on a last observed seat.
    pub fn store<T: Into<String>>(&self, text: T) {
        self.send(worker::Command::Store(text.into()));
    }

    /// Load primary clipboard data.
    ///
    /// Loads content from a  primary clipboard on a last observed seat.
    pub fn load_primary(&self) -> Result<String> {
        self.request(worker::Command::LoadPrimary)
    }

    /// Store to a primary clipboard.
    ///
    /// Stores to a primary clipboard on a last observed seat.
    pub fn store_primary<T: Into<String>>(&self, text: T) {
        self.send(worker::Command::StorePrimary(text.into()));
    }
}
