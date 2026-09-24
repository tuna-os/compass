//! The engine on a real wlroots compositor: window switching over
//! `zwlr_foreign_toplevel_manager_v1`, through the same socket requests the
//! launcher sends.
//!
//! The compositor harness is `compass-wayland`'s (one headless Sway per test,
//! skipped loudly without `sway`, required when `COMPASS_REQUIRE_COMPOSITOR`
//! is set). What this adds over that crate's own tests is the engine: its
//! registry probe, the GNOME-first family decision and the request routing in
//! `serve`, which only a real process on a real socket exercises.

#[path = "../../compass-wayland/tests/support/mod.rs"]
mod support;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use compass_ipc::{Request, Response, WindowInfo};
use support::{Sway, TestWindow, eventually};

const NO_SESSION_BUS: &str = "unix:path=/nonexistent/compass-test-no-session-bus";
const WAIT: Duration = Duration::from_secs(20);

fn binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("vicinae")
}

struct Engine {
    child: Child,
    socket: PathBuf,
    _dirs: tempfile::TempDir,
}

impl Engine {
    fn start(sway: &Sway, desktop: &str) -> Self {
        let dirs = tempfile::tempdir().unwrap();
        let socket = dirs.path().join("ipc.sock");
        let child = Command::new(binary())
            .arg("--socket")
            .arg(&socket)
            .args(["serve", "--no-hotkey"])
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .env("XDG_DATA_DIRS", dirs.path().join("empty"))
            .env("XDG_DATA_HOME", dirs.path().join("data"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            .env("HOME", dirs.path())
            .env("XDG_RUNTIME_DIR", sway.runtime_dir())
            .env("WAYLAND_DISPLAY", sway.display())
            .env("XDG_CURRENT_DESKTOP", desktop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the engine");
        let engine = Self {
            child,
            socket,
            _dirs: dirs,
        };
        assert!(
            eventually(WAIT, || engine.try_request(Request::Ping).is_some()),
            "the engine never listened"
        );
        engine
    }

    fn try_request(&self, request: Request) -> Option<Response> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut client = compass_ipc::Client::connect(&self.socket).await.ok()?;
                client.request(request).await.ok()
            })
    }

    fn request(&self, request: Request) -> Response {
        self.try_request(request).expect("the engine answered")
    }

    fn windows(&self) -> Vec<WindowInfo> {
        match self.request(Request::ListWindows) {
            Response::Windows { windows } => windows,
            other => panic!("ListWindows answered {other:?}"),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn on_sway_the_engine_lists_focuses_and_closes_windows_without_the_shell_extension() {
    let Some(sway) = Sway::start("on_sway_the_engine_lists_windows") else {
        return;
    };
    let _alpha = TestWindow::open(&sway, "Alpha document", "test.Alpha");
    let beta = TestWindow::open(&sway, "Beta document", "test.Beta");
    let engine = Engine::start(&sway, "sway");

    let mut windows = Vec::new();
    assert!(
        eventually(WAIT, || {
            windows = engine.windows();
            windows.len() == 2
        }),
        "{windows:?}"
    );
    // The focused window goes last, as on GNOME: it is the one the user is
    // already looking at.
    assert_eq!(windows[0].wm_class, "test.Alpha", "{windows:?}");
    assert_eq!(windows[1].wm_class, "test.Beta");
    assert!(windows[1].focused && !windows[0].focused);
    assert!(windows.iter().all(|w| w.can_close && w.pid.is_none()));

    let alpha = windows[0].id;
    assert_eq!(
        engine.request(Request::ActivateWindow { id: alpha }),
        Response::Ack
    );
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        windows = engine.windows();
        if windows.last().is_some_and(|w| w.id == alpha && w.focused) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        windows.last().is_some_and(|w| w.id == alpha && w.focused),
        "activating alpha did not focus it: {windows:?}"
    );

    let beta_id = windows
        .iter()
        .find(|w| w.wm_class == "test.Beta")
        .unwrap()
        .id;
    assert_eq!(
        engine.request(Request::CloseWindow { id: beta_id }),
        Response::Ack
    );
    assert!(eventually(WAIT, || beta.is_closed()));
    assert!(eventually(WAIT, || engine.windows().len() == 1));
}

#[test]
fn on_sway_a_gnome_desktop_name_keeps_the_gnome_path() {
    // The family decision puts GNOME first by name, so even a compositor with
    // every wlroots protocol is not driven over Wayland when the session says
    // it is GNOME — the request falls through to the Shell extension path,
    // which with no session bus refuses by naming it.
    let Some(sway) = Sway::start("a_gnome_desktop_name_keeps_the_gnome_path") else {
        return;
    };
    let _window = TestWindow::open(&sway, "Alpha", "test.Alpha");
    let engine = Engine::start(&sway, "GNOME");
    let Response::Error(err) = engine.request(Request::ListWindows) else {
        panic!("GNOME must not list windows over wlroots protocols");
    };
    assert!(err.message.contains("session bus"), "{}", err.message);
}
