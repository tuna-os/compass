//! The kernel side: which `/dev/input` nodes to read, reading them, and the
//! uinput device injection writes to.
//!
//! # Classification without libudev
//!
//! The C++ asks libudev for nodes tagged `ID_INPUT_KEYBOARD` and
//! `ID_INPUT_MOUSE`. Those tags are computed by udev's `input_id` builtin
//! from the device's capability bits, so this reads the same bits through
//! `evdev` and applies the same tests ([`is_keyboard`], [`is_pointer`])
//! rather than linking libudev for a lookup. Hot-plugging is an inotify watch
//! on `/dev/input` (`notify`) instead of a udev monitor: a node appearing is
//! all the C++ acts on too.
//!
//! # Nothing is grabbed
//!
//! Devices are opened read-only and never `EVIOCGRAB`bed: the server
//! watches typing, it never takes it away from the session.

use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use compass_platform_linux::keyboard::{self as vk, EventSink, KeyEvent};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsoluteAxisCode, AttributeSet, BusType, Device, EventType, InputEvent, InputId, KeyCode,
    PropType, RelativeAxisCode,
};

use crate::recording::RawEvent;

/// Where event nodes live.
pub const INPUT_DIR: &str = "/dev/input";

/// What a node is watched for. A keyboard with a pointing stick is both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Kind {
    /// Typing is read from it.
    pub keyboard: bool,
    /// Any event from it interrupts an injection.
    pub pointer: bool,
}

impl Kind {
    /// Whether it is watched at all.
    #[must_use]
    pub const fn any(self) -> bool {
        self.keyboard || self.pointer
    }
}

/// udev's keyboard test: every key from `KEY_ESC` to `KEY_D` (codes 1–31,
/// the first word of the key bitmask bar bit 0).
#[must_use]
pub fn is_keyboard(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| (1..32).all(|code| keys.contains(KeyCode::new(code))))
}

/// udev's mouse test: relative X and Y with a left button, or absolute X and
/// Y with a left button on something that is neither a pen nor a touchpad
/// (a virtual machine's tablet).
#[must_use]
pub fn is_pointer(device: &Device) -> bool {
    let has_key = |key: KeyCode| {
        device
            .supported_keys()
            .is_some_and(|keys| keys.contains(key))
    };
    if !has_key(KeyCode::BTN_LEFT) {
        return false;
    }
    let relative = device.supported_relative_axes().is_some_and(|axes| {
        axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y)
    });
    if relative {
        return true;
    }
    let absolute = device.supported_absolute_axes().is_some_and(|axes| {
        axes.contains(AbsoluteAxisCode::ABS_X) && axes.contains(AbsoluteAxisCode::ABS_Y)
    });
    let pen = has_key(KeyCode::BTN_TOOL_PEN) || has_key(KeyCode::BTN_STYLUS);
    let touchpad =
        has_key(KeyCode::BTN_TOOL_FINGER) && !device.properties().contains(PropType::DIRECT);
    absolute && !pen && !touchpad
}

/// Whether `path` names an event node (`/dev/input/eventN`).
#[must_use]
pub fn is_event_node(path: &Path) -> bool {
    path.parent() == Some(Path::new(INPUT_DIR))
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"))
}

/// Opens `path` read-only and says what it is, skipping the server's own
/// virtual keyboard (its name is [`vk::device::NAME`]).
///
/// # Errors
///
/// When the node cannot be opened, or is not an evdev device.
pub fn open(path: &Path) -> std::io::Result<Option<(Device, Kind)>> {
    let file = File::open(path)?;
    let device = Device::from_fd(OwnedFd::from(file))?;
    if device.name() == Some(vk::device::NAME) {
        return Ok(None);
    }
    let kind = Kind {
        keyboard: is_keyboard(&device),
        pointer: is_pointer(&device),
    };
    Ok(kind.any().then_some((device, kind)))
}

/// The event nodes present now, sorted, as the C++ enumerates them at start.
#[must_use]
pub fn event_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir(INPUT_DIR)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| is_event_node(path))
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// What the reader threads and the hot-plug watch tell the main loop.
#[derive(Debug)]
pub enum DeviceMessage {
    /// An event from a watched node.
    Input {
        /// The node.
        node: PathBuf,
        /// What it is watched for.
        kind: Kind,
        /// The event.
        event: RawEvent,
    },
    /// The node stopped answering (unplugged).
    Gone(PathBuf),
    /// A node appeared under `/dev/input`.
    Appeared(PathBuf),
}

/// Reads `device` until it fails, forwarding every event.
///
/// A thread per device keeps each read blocking and simple; the channel is
/// the C++'s epoll.
pub fn spawn_reader<M: From<DeviceMessage> + Send + 'static>(
    node: PathBuf,
    mut device: Device,
    kind: Kind,
    tx: Sender<M>,
) {
    let spawned = std::thread::Builder::new()
        .name(format!("read {}", node.display()))
        .spawn(move || {
            loop {
                let events: Vec<InputEvent> = match device.fetch_events() {
                    Ok(events) => events.collect(),
                    Err(error) => {
                        eprintln!("stopped reading {}: {error}", node.display());
                        let _ = tx.send(DeviceMessage::Gone(node).into());
                        return;
                    }
                };
                for event in events {
                    let raw = RawEvent {
                        kind: event.event_type().0,
                        code: event.code(),
                        value: event.value(),
                    };
                    let message = DeviceMessage::Input {
                        node: node.clone(),
                        kind,
                        event: raw,
                    };
                    if tx.send(message.into()).is_err() {
                        return;
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("could not start a reader thread: {error}");
    }
}

/// Watches `/dev/input` for new nodes. The watcher must be kept alive.
///
/// # Errors
///
/// When inotify cannot watch the directory.
pub fn watch_hotplug<M: From<DeviceMessage> + Send + 'static>(
    tx: Sender<M>,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if !matches!(event.kind, EventKind::Create(_)) {
            return;
        }
        for path in event.paths {
            if is_event_node(&path) {
                let _ = tx.send(DeviceMessage::Appeared(path).into());
            }
        }
    })?;
    watcher.watch(Path::new(INPUT_DIR), RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// Opens a node that just appeared. udev may still be setting it up, so a
/// failed open is retried briefly before giving up.
#[must_use]
pub fn open_new(path: &Path) -> Option<(Device, Kind)> {
    for attempt in 0..10 {
        match open(path) {
            Ok(found) => return found,
            Err(error) if attempt == 9 => {
                eprintln!("could not open hot-plugged {}: {error}", path.display());
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    None
}

/// The virtual keyboard's sink: events buffered until a sync, then written.
///
/// `evdev`'s `VirtualDevice::emit` appends its own `SYN_REPORT` to whatever
/// it writes, so the model's [`KeyEvent::Sync`] is where a batch is handed
/// over: the reader sees exactly the C++'s batches, press–sync–release–sync.
pub struct UinputSink {
    device: VirtualDevice,
    batch: Vec<InputEvent>,
}

impl std::fmt::Debug for UinputSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UinputSink")
            .field("batch", &self.batch.len())
            .finish_non_exhaustive()
    }
}

impl UinputSink {
    /// Creates `compass-snippet-virtual-keyboard` with every key from
    /// `KEY_ESC` to 255, as `UInputKeyboard`'s constructor does.
    ///
    /// # Errors
    ///
    /// When `/dev/uinput` cannot be opened (no permission, no module) or the
    /// device cannot be created.
    pub fn create() -> std::io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in vk::FIRST_KEY..vk::KEY_LIMIT {
            keys.insert(KeyCode::new(code));
        }
        let device = VirtualDevice::builder()?
            .name(vk::device::NAME)
            .input_id(InputId::new(
                BusType(vk::device::BUSTYPE),
                vk::device::VENDOR,
                vk::device::PRODUCT,
                vk::device::VERSION,
            ))
            .with_keys(&keys)?
            .build()?;
        Ok(Self {
            device,
            batch: Vec::new(),
        })
    }

    /// The `/dev/input` nodes the kernel made for this device.
    ///
    /// # Errors
    ///
    /// When sysfs cannot be read.
    pub fn event_nodes(&mut self) -> std::io::Result<Vec<PathBuf>> {
        self.device
            .enumerate_dev_nodes_blocking()?
            .collect::<std::io::Result<Vec<_>>>()
    }
}

impl EventSink for UinputSink {
    fn emit(&mut self, event: KeyEvent) {
        let key = |code: u16, value: i32| InputEvent::new(EventType::KEY.0, code, value);
        match event {
            KeyEvent::Press(code) => self.batch.push(key(code, 1)),
            KeyEvent::Release(code) => self.batch.push(key(code, 0)),
            KeyEvent::Sync => {
                if let Err(error) = self.device.emit(&self.batch) {
                    eprintln!("uinput write failed: {error}");
                }
                self.batch.clear();
            }
        }
    }

    fn delay(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
