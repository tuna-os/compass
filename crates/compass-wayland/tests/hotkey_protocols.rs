//! `compass_wayland::hotkey` against a fake compositor.
//!
//! No released compositor carries `xx-hotkey-v1` or `vicinae-hotkey-v1`, so
//! the client is driven here against the compositor side of both protocols,
//! stood up in-process with `wayland-server` over a socket pair: the bytes
//! are real Wayland wire traffic, and nothing reaches a real display.
//!
//! The fake binds anything but the `x` key, which it refuses as taken, and
//! logs every request so a test can see what the client asked for and when
//! it let go.

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use compass_wayland::hotkey::{
    HotkeyClient, HotkeyError, HotkeyEvent, HotkeyRequest, Protocol, modifiers,
};
use compass_wayland_protocols::server::vicinae_hotkey_v1::{
    vicinae_hotkey_manager_v1::{self, VicinaeHotkeyManagerV1},
    vicinae_hotkey_v1::{self, VicinaeHotkeyV1},
};
use compass_wayland_protocols::server::xx_hotkey_v1::{
    xx_hotkey_manager_v1::{self, XxHotkeyManagerV1},
    xx_hotkey_v1::{self, XxHotkeyV1},
};
use tokio::sync::mpsc;
use wayland_server::backend::ClientData;
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, WEnum,
};

/// `XKB_KEY_x`, which the fake refuses.
const TAKEN: u32 = 0x78;

/// `XKB_KEY_b`.
const KEY_B: u32 = 0x62;

#[derive(Default)]
struct Compositor {
    log: Arc<Mutex<Vec<String>>>,
    xx: Vec<(XxHotkeyV1, u32)>,
    vicinae: Vec<VicinaeHotkeyV1>,
}

impl Compositor {
    fn log(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }
}

struct NoData;
impl ClientData for NoData {}

impl GlobalDispatch<VicinaeHotkeyManagerV1, ()> for Compositor {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<VicinaeHotkeyManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<VicinaeHotkeyManagerV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &VicinaeHotkeyManagerV1,
        request: vicinae_hotkey_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let vicinae_hotkey_manager_v1::Request::Bind {
            id,
            keysym,
            modifiers,
            app_id,
            description,
            ..
        } = request
        {
            let hotkey = data_init.init(id, ());
            let modifiers = match modifiers {
                WEnum::Value(value) => value.bits(),
                WEnum::Unknown(raw) => raw,
            };
            state.log(format!(
                "bind {keysym:#x} {modifiers} {app_id} {description}"
            ));
            if keysym == TAKEN {
                hotkey.denied(
                    vicinae_hotkey_v1::DenyReason::AlreadyBound,
                    "taken".to_owned(),
                );
            } else {
                hotkey.bound();
                state.vicinae.push(hotkey);
            }
        }
    }
}

impl Dispatch<VicinaeHotkeyV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &VicinaeHotkeyV1,
        request: vicinae_hotkey_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let vicinae_hotkey_v1::Request::Destroy = request {
            state.log("destroy".to_owned());
            state.vicinae.retain(|hotkey| hotkey != resource);
        }
    }
}

impl GlobalDispatch<XxHotkeyManagerV1, ()> for Compositor {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<XxHotkeyManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<XxHotkeyManagerV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XxHotkeyManagerV1,
        request: xx_hotkey_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xx_hotkey_manager_v1::Request::SetAppId { app_id } => {
                state.log(format!("app_id {app_id}"));
            }
            xx_hotkey_manager_v1::Request::CreateHotkey { id } => {
                state.xx.push((data_init.init(id, ()), 0));
            }
            _ => {}
        }
    }
}

impl Dispatch<XxHotkeyV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &XxHotkeyV1,
        request: xx_hotkey_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            xx_hotkey_v1::Request::SetKeyTrigger { keysym, modifiers } => {
                let modifiers = match modifiers {
                    WEnum::Value(value) => value.bits(),
                    WEnum::Unknown(raw) => raw,
                };
                state.log(format!("trigger {keysym:#x} {modifiers}"));
                if let Some(entry) = state.xx.iter_mut().find(|(h, _)| h == resource) {
                    entry.1 = keysym;
                }
            }
            xx_hotkey_v1::Request::Commit => {
                let keysym = state
                    .xx
                    .iter()
                    .find(|(h, _)| h == resource)
                    .map_or(0, |(_, keysym)| *keysym);
                if keysym == TAKEN {
                    resource.denied(xx_hotkey_v1::DenyReason::AlreadyBound, String::new());
                } else {
                    resource.bound();
                }
            }
            xx_hotkey_v1::Request::Destroy => {
                state.log("destroy".to_owned());
                state.xx.retain(|(h, _)| h != resource);
            }
            _ => {}
        }
    }
}

enum Command {
    PressAll,
    RevokeAll,
}

/// A fake compositor on its own thread, and the client end of its socket.
struct Fake {
    log: Arc<Mutex<Vec<String>>>,
    commands: std_mpsc::Sender<Command>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(xx: bool) -> (Self, wayland_client::Connection) {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        let mut display: Display<Compositor> = Display::new().expect("a display");
        let handle = display.handle();
        if xx {
            handle.create_global::<Compositor, XxHotkeyManagerV1, ()>(1, ());
        }
        handle.create_global::<Compositor, VicinaeHotkeyManagerV1, ()>(1, ());
        display
            .handle()
            .insert_client(server, Arc::new(NoData))
            .expect("the client");
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (commands, received) = std_mpsc::channel();
        let thread = {
            let log = Arc::clone(&log);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut state = Compositor {
                    log,
                    ..Compositor::default()
                };
                let mut serial = 0;
                while !stop.load(Ordering::Acquire) {
                    display.dispatch_clients(&mut state).expect("dispatch");
                    while let Ok(command) = received.try_recv() {
                        serial += 1;
                        match command {
                            Command::PressAll => {
                                for (hotkey, _) in &state.xx {
                                    hotkey.triggered(serial, 0);
                                }
                                for hotkey in &state.vicinae {
                                    hotkey.pressed(serial, 0);
                                }
                            }
                            Command::RevokeAll => {
                                for (hotkey, _) in &state.xx {
                                    hotkey.revoked("gone".to_owned());
                                }
                                for hotkey in &state.vicinae {
                                    hotkey.revoked(
                                        vicinae_hotkey_v1::RevokeReason::Removed,
                                        "gone".to_owned(),
                                    );
                                }
                            }
                        }
                    }
                    let _ = display.flush_clients();
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let connection =
            wayland_client::Connection::from_socket(client).expect("a client connection");
        (
            Self {
                log,
                commands,
                stop,
                thread: Some(thread),
            },
            connection,
        )
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Waits for the log to reach `len` lines.
    fn log_until(&self, len: usize) -> Vec<String> {
        for _ in 0..2000 {
            let log = self.log();
            if log.len() >= len {
                return log;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the compositor saw {:?}", self.log());
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn request(id: &str, keysym: u32) -> HotkeyRequest {
    HotkeyRequest {
        id: id.to_owned(),
        description: format!("run {id}"),
        keysym,
        modifiers: modifiers::SUPER | modifiers::SHIFT,
    }
}

async fn next(events: &mut mpsc::UnboundedReceiver<HotkeyEvent>) -> HotkeyEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event in time")
        .expect("the channel open")
}

#[tokio::test]
async fn vicinae_hotkey_binds_presses_refuses_and_releases() {
    let (fake, connection) = Fake::start(false);
    let (tx, mut events) = mpsc::unbounded_channel();
    let client = HotkeyClient::connect_on(connection, "org.tunaos.compass", tx).expect("a client");
    assert_eq!(client.protocol(), Protocol::Vicinae);

    let launcher = client
        .bind(&request("toggle", 0x20))
        .await
        .expect("space is free");
    let command = client
        .bind(&request("clipboard:history", KEY_B))
        .await
        .expect("b is free");
    assert_eq!(
        fake.log(),
        [
            "bind 0x20 9 org.tunaos.compass run toggle",
            "bind 0x62 9 org.tunaos.compass run clipboard:history",
        ]
    );

    match client.bind(&request("taken", TAKEN)).await {
        Err(HotkeyError::Denied(message)) => assert_eq!(message, "taken"),
        other => panic!("the compositor refused x, got {other:?}"),
    }

    fake.commands.send(Command::PressAll).unwrap();
    let mut pressed = [next(&mut events).await, next(&mut events).await];
    pressed.sort_by_key(|event| format!("{event:?}"));
    assert!(matches!(&pressed[0], HotkeyEvent::Triggered { id, .. } if id == "clipboard:history"));
    assert!(matches!(&pressed[1], HotkeyEvent::Triggered { id, .. } if id == "toggle"));

    drop(command);
    let log = fake.log_until(5);
    assert_eq!(log[3..], ["destroy", "destroy"], "the refused one, then b");

    fake.commands.send(Command::RevokeAll).unwrap();
    assert!(matches!(
        next(&mut events).await,
        HotkeyEvent::Revoked { id, message } if id == "toggle" && message == "gone"
    ));
    drop(launcher);
}

#[tokio::test]
async fn xx_hotkey_is_preferred_and_speaks_its_own_requests() {
    let (fake, connection) = Fake::start(true);
    let (tx, mut events) = mpsc::unbounded_channel();
    let client = HotkeyClient::connect_on(connection, "org.tunaos.compass", tx).expect("a client");
    assert_eq!(client.protocol(), Protocol::Xx, "the C++ factory's order");

    let hotkey = client
        .bind(&request("toggle", 0x20))
        .await
        .expect("space is free");
    assert_eq!(fake.log(), ["app_id org.tunaos.compass", "trigger 0x20 9"]);
    match client.bind(&request("taken", TAKEN)).await {
        Err(HotkeyError::Denied(message)) => {
            assert_eq!(message, compass_wayland::hotkey::DENIED_WITHOUT_MESSAGE);
        }
        other => panic!("the compositor refused x, got {other:?}"),
    }

    fake.commands.send(Command::PressAll).unwrap();
    assert!(matches!(
        next(&mut events).await,
        HotkeyEvent::Triggered { id, .. } if id == "toggle"
    ));
    drop(hotkey);
    assert_eq!(fake.log_until(5)[3..], ["destroy", "destroy"]);
}

#[test]
fn a_compositor_with_neither_protocol_is_unsupported() {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let mut display: Display<Compositor> = Display::new().expect("a display");
    display
        .handle()
        .insert_client(server, Arc::new(NoData))
        .expect("the client");
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut state = Compositor::default();
            while !stop.load(Ordering::Acquire) {
                let _ = display.dispatch_clients(&mut state);
                let _ = display.flush_clients();
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    };
    let connection = wayland_client::Connection::from_socket(client).expect("a connection");
    let (tx, _rx) = mpsc::unbounded_channel();
    let result = HotkeyClient::connect_on(connection, "org.tunaos.compass", tx);
    stop.store(true, Ordering::Release);
    let _ = thread.join();
    assert!(
        matches!(result, Err(HotkeyError::Unsupported)),
        "{result:?}"
    );
}
