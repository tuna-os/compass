//! A real wlroots compositor for tests: headless Sway in a private runtime
//! directory.
//!
//! Every test starts its own Sway, so tests neither share a selection nor a
//! window list, and connects to it by socket path rather than through
//! `WAYLAND_DISPLAY` — the environment is process-wide and the tests run in
//! parallel. The one thing that has to go through the environment
//! (`wl-clipboard-rs` connects with `connect_to_env`) is run in a child copy of
//! the test binary, see [`Sway::run_child`].
//!
//! # Skipping, loudly
//!
//! Without `sway` on `PATH` a test prints why and returns, because most
//! machines this workspace is built on have no compositor. CI sets
//! `COMPASS_REQUIRE_COMPOSITOR=1`, and then a missing Sway is a failure: a
//! compositor job that silently skipped everything would be the green that
//! proves nothing.

#![allow(dead_code)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Set by CI: a missing compositor fails instead of skipping.
pub const REQUIRE_ENV: &str = "COMPASS_REQUIRE_COMPOSITOR";

/// Set in a child copy of the test binary to say which child role it plays.
pub const CHILD_ENV: &str = "COMPASS_WLR_CHILD";

/// A running headless Sway.
pub struct Sway {
    child: Child,
    runtime: tempfile::TempDir,
    socket: PathBuf,
    display: String,
}

impl Sway {
    /// Starts Sway, or returns `None` (having said why) when there is none.
    ///
    /// # Panics
    ///
    /// When Sway is missing and [`REQUIRE_ENV`] is set, or when it starts and
    /// never opens a socket.
    pub fn start(test: &str) -> Option<Self> {
        let found = Command::new("sway")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            assert!(
                std::env::var_os(REQUIRE_ENV).is_none(),
                "{REQUIRE_ENV} is set and `sway` is not on PATH"
            );
            eprintln!(
                "SKIPPED {test}: no `sway` on PATH. This test needs a real wlroots \
                 compositor; install sway (the `wlroots` CI job does) to run it."
            );
            return None;
        }

        let runtime = tempfile::Builder::new()
            .prefix("compass-sway-")
            .tempdir()
            .expect("a private runtime directory");
        let config = runtime.path().join("sway.cfg");
        std::fs::write(
            &config,
            "output HEADLESS-1 resolution 1280x800\nxwayland disable\n",
        )
        .expect("writing the sway config");
        let log = std::fs::File::create(runtime.path().join("sway.log")).expect("a log file");

        let child = Command::new("sway")
            .arg("-c")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("SWAYSOCK")
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawning sway");

        let deadline = Instant::now() + Duration::from_secs(20);
        let display = loop {
            if let Some(name) = wayland_socket(runtime.path()) {
                break name;
            }
            assert!(
                Instant::now() < deadline,
                "sway opened no Wayland socket within 20s; its log:\n{}",
                std::fs::read_to_string(runtime.path().join("sway.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        let socket = runtime.path().join(&display);
        Some(Self {
            child,
            runtime,
            socket,
            display,
        })
    }

    /// A fresh client connection to this compositor.
    pub fn connect(&self) -> Connection {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match UnixStream::connect(&self.socket) {
                Ok(stream) => {
                    return Connection::from_socket(stream).expect("a Wayland connection");
                }
                Err(err) => {
                    assert!(Instant::now() < deadline, "connecting to sway: {err}");
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    /// The runtime directory, for `XDG_RUNTIME_DIR`.
    pub fn runtime_dir(&self) -> &Path {
        self.runtime.path()
    }

    /// The socket name, for `WAYLAND_DISPLAY`.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Runs the test named `test` in a child copy of this test binary with the
    /// compositor in its environment and [`CHILD_ENV`] set to `role`.
    ///
    /// The child must check [`child_role`] and return immediately when it is
    /// not set, so the same function is a no-op in the ordinary run.
    pub fn run_child(&self, test: &str, role: &str, extra: &[(&str, &str)]) -> Child {
        let mut command = Command::new(std::env::current_exe().expect("the test binary"));
        command
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, role)
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("WAYLAND_DISPLAY", self.display())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in extra {
            command.env(key, value);
        }
        command.spawn().expect("spawning the child test")
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The role this process plays, when it is a child started by
/// [`Sway::run_child`].
pub fn child_role() -> Option<String> {
    std::env::var(CHILD_ENV).ok()
}

fn wayland_socket(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().into_string().ok()?;
        (name.starts_with("wayland-") && !name.ends_with(".lock")).then_some(name)
    })
}

/// Polls `condition` until it holds or `timeout` passes.
pub fn eventually(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A plain `xdg_toplevel` window from a test client, standing in for an
/// application. It closes itself when the compositor asks it to, as a
/// well-behaved application does.
pub struct TestWindow {
    closed: Arc<Mutex<bool>>,
}

struct WindowState {
    wm_base: xdg_wm_base::XdgWmBase,
    surface: wl_surface::WlSurface,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    buffer: wl_buffer::WlBuffer,
    configured: bool,
    closed: Arc<Mutex<bool>>,
}

impl TestWindow {
    /// Maps a 64×64 window with `title` and `app_id`.
    pub fn open(sway: &Sway, title: &str, app_id: &str) -> Self {
        let connection = sway.connect();
        let (globals, mut queue): (_, EventQueue<WindowState>) =
            registry_queue_init(&connection).expect("registry");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor =
            globals.bind(&qh, 1..=4, ()).expect("wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
        let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=2, ()).expect("xdg_wm_base");

        let (width, height) = (64_i32, 64_i32);
        let stride = width * 4;
        let file = tempfile::tempfile().expect("a shm file");
        file.set_len(u64::try_from(stride * height).unwrap())
            .expect("sizing the shm file");
        let pool = shm.create_pool(std::os::fd::AsFd::as_fd(&file), stride * height, &qh, ());
        let buffer =
            pool.create_buffer(0, width, height, stride, wl_shm::Format::Xrgb8888, &qh, ());

        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title(title.to_owned());
        toplevel.set_app_id(app_id.to_owned());
        surface.commit();

        let closed = Arc::new(Mutex::new(false));
        let mut state = WindowState {
            wm_base,
            surface,
            toplevel: Some(toplevel),
            buffer,
            configured: false,
            closed: Arc::clone(&closed),
        };
        while !state.configured {
            queue
                .blocking_dispatch(&mut state)
                .expect("configuring the window");
        }
        // The buffer commit queued in the configure handler must reach Sway
        // before this returns, or two windows opened in a row can map in
        // either order — and Sway focuses the one mapped last.
        queue.roundtrip(&mut state).expect("mapping the window");
        std::thread::spawn(move || {
            let _keep = (file, pool, xdg);
            while queue.blocking_dispatch(&mut state).is_ok() {}
        });
        Self { closed }
    }

    /// Whether the compositor asked it to close, and it did.
    pub fn is_closed(&self) -> bool {
        *self.closed.lock().unwrap()
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WindowState {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WindowState {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for WindowState {
    fn event(
        state: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            if state.toplevel.is_none() {
                return;
            }
            xdg.ack_configure(serial);
            if !state.configured && state.toplevel.is_some() {
                state.surface.attach(Some(&state.buffer), 0, 0);
                state.surface.commit();
            }
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WindowState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event
            && let Some(toplevel) = state.toplevel.take()
        {
            // Destroying the role unmaps the window. No commit afterwards: a
            // roleless `xdg_surface` may not commit, and Sway enforces it.
            toplevel.destroy();
            *state.closed.lock().unwrap() = true;
        }
    }
}

macro_rules! ignore_events {
    ($($ty:ty),*) => {$(
        impl Dispatch<$ty, ()> for WindowState {
            fn event(
                _: &mut Self,
                _: &$ty,
                _: <$ty as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface
);
