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
        Self::start_with(sway, desktop, |_| Vec::new())
    }

    /// [`Self::start`], after `prepare` has laid out the temporary tree and
    /// named any extra environment.
    fn start_with(
        sway: &Sway,
        desktop: &str,
        prepare: impl FnOnce(&std::path::Path) -> Vec<(&'static str, std::ffi::OsString)>,
    ) -> Self {
        let dirs = tempfile::tempdir().unwrap();
        let extra = prepare(dirs.path());
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
            .envs(extra)
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

/// Child role: hold `COMPASS_WLR_TEXT` as the primary selection until killed.
#[test]
fn child_holds_a_primary_selection() {
    if support::child_role().is_none() {
        return;
    }
    let text = std::env::var("COMPASS_WLR_TEXT").unwrap();
    let mut options = wl_clipboard_rs::copy::Options::new();
    options.clipboard(wl_clipboard_rs::copy::ClipboardType::Primary);
    options
        .copy(
            wl_clipboard_rs::copy::Source::Bytes(text.into_bytes().into_boxed_slice()),
            wl_clipboard_rs::copy::MimeType::Text,
        )
        .expect("selecting");
    println!("CHILD-OK");
    std::thread::sleep(WAIT * 2);
}

fn extension_runtime() -> Option<PathBuf> {
    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/typescript/extension-manager/dist/runtime.js");
    std::env::var_os("COMPASS_EXTENSION_RUNTIME")
        .map(Into::into)
        .or_else(|| built.is_file().then_some(built))
}

#[test]
fn on_sway_an_extension_reads_the_selection_the_windows_and_the_monitors() {
    use std::io::BufRead;
    let Some(sway) = Sway::start("on_sway_an_extension_reads_the_desktop") else {
        return;
    };
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let _window = TestWindow::open(&sway, "Alpha document", "test.Alpha");
    let mut holder = sway.run_child(
        "child_holds_a_primary_selection",
        "select",
        &[("COMPASS_WLR_TEXT", "selected words")],
    );
    let mut lines = std::io::BufReader::new(holder.stdout.take().unwrap()).lines();
    assert!(
        lines.any(|line| line.is_ok_and(|line| line.contains("CHILD-OK"))),
        "the selection holder never started"
    );

    let engine = Engine::start_with(&sway, "sway", |root| {
        let ext = root.join("data/vicinae/extensions/hello");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("package.json"),
            r#"{"name": "hello", "title": "Hello", "author": "someone",
                "commands": [{"name": "machine", "title": "Machine", "mode": "view"}]}"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("machine.js"),
            "const React = require('react');
             const { Detail, getSelectedText, WindowManagement } = require('@vicinae/api');
             const said = (p) => p.then((v) => JSON.stringify(v), (e) => 'error: ' + String(e));
             module.exports.default = () => {
               const [text, setText] = React.useState('');
               React.useEffect(() => {
                 Promise.all([
                   said(getSelectedText()),
                   said(WindowManagement.getActiveWindow()),
                   said(WindowManagement.getScreens()),
                 ]).then((answers) => setText(answers.join(' | ')));
               }, []);
               return React.createElement(Detail, { markdown: text || 'asking' });
             };",
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });

    let Response::ExtensionStarted { session } = engine.request(Request::RunExtensionCommand {
        id: "@someone/hello:machine".into(),
        arguments_json: None,
    }) else {
        panic!("the command did not start");
    };
    let mut markdown = String::new();
    let mut after = 0;
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let Response::ExtensionView {
            version,
            view_json,
            problem,
            ..
        } = engine.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        assert!(problem.is_none(), "{problem:?}");
        after = version;
        if let Some(compass_extension_api::View::Detail(detail)) =
            view_json.and_then(|json| serde_json::from_str(&json).ok())
            && let Some(text) = detail.markdown
            && text != "asking"
        {
            markdown = text;
            break;
        }
    }
    let _ = holder.kill();
    let _ = holder.wait();

    let answers: Vec<&str> = markdown.split(" | ").collect();
    assert_eq!(answers.len(), 3, "{markdown}");
    assert_eq!(answers[0], "\"selected words\"");
    let window: serde_json::Value = serde_json::from_str(answers[1]).expect(answers[1]);
    assert_eq!(window["title"], "Alpha document");
    assert_eq!(window["active"], true);
    let screens: serde_json::Value = serde_json::from_str(answers[2]).expect(answers[2]);
    assert_eq!(screens[0]["name"], "HEADLESS-1", "{screens}");
    assert_eq!(screens[0]["physicalResolution"]["width"], 1280);
    assert_eq!(screens[0]["active"], true);
}
