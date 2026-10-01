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
    path.join("compass")
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
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env("XDG_DATA_DIRS", dirs.path().join("empty"))
            .env("XDG_DATA_HOME", dirs.path().join("data"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            .env("HOME", dirs.path())
            .env_remove("XDG_STATE_HOME")
            .env("XDG_RUNTIME_DIR", sway.runtime_dir())
            .env("WAYLAND_DISPLAY", sway.display())
            .env("XDG_CURRENT_DESKTOP", desktop)
            // Never the invoking session's compositor socket.
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("NIRI_SOCKET")
            // Nor GitHub: the launcher asks for the update status when it
            // opens, and a closed local port answers it.
            .env(
                "COMPASS_UPDATE_FEED_URL",
                "http://127.0.0.1:9/releases/latest",
            )
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
fn on_sway_a_shortcut_expands_the_selected_text() {
    use std::io::BufRead;
    let Some(sway) = Sway::start("on_sway_a_shortcut_expands_the_selection") else {
        return;
    };
    let engine = Engine::start(&sway, "sway");
    let Response::Shortcuts { shortcuts } = engine.request(Request::SaveShortcut {
        id: None,
        name: "Define".into(),
        icon: "icon://omnicast/link".into(),
        url: "https://example.com/?q={selection}&also={selected}".into(),
        app: "default".into(),
    }) else {
        panic!("the shortcut was not saved");
    };
    let id = shortcuts[0].id.clone();
    let expand = || {
        engine.request(Request::ExpandShortcut {
            id: id.clone(),
            arguments: Vec::new(),
        })
    };

    // Nothing selected: the placeholders expand to nothing, as the C++'s do.
    assert_eq!(
        expand(),
        Response::Text {
            text: "https://example.com/?q=&also=".into()
        }
    );

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
    // The holder has handed its source to the compositor, but the selection
    // is only offered once the compositor has processed it: read until it is.
    let want = Response::Text {
        text: "https://example.com/?q=selected words&also=selected words".into(),
    };
    let deadline = std::time::Instant::now() + WAIT;
    let mut expanded = expand();
    while expanded != want && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        expanded = expand();
    }
    let _ = holder.kill();
    let _ = holder.wait();
    assert_eq!(expanded, want);
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
        let ext = root.join("data/compass/extensions/hello");
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

#[test]
fn on_sway_with_a_hyprland_socket_windows_learn_their_pid_and_workspace() {
    use compass_testkit::fake_compositor::{FakeSocket, Framing};
    let Some(sway) = Sway::start("on_sway_with_a_hyprland_socket") else {
        return;
    };
    let _alpha = TestWindow::open(&sway, "Alpha document", "test.Alpha");
    let _beta = TestWindow::open(&sway, "Beta document", "test.Beta");
    // The toplevel list is Sway's; the socket answers as Hyprland would
    // for the same two windows, and a third the toplevels do not have.
    let fake = FakeSocket::replaying(
        &FakeSocket::hyprland_path(sway.runtime_dir(), "compass-test"),
        Framing::Hyprland,
        vec![
            (
                "-j/clients".into(),
                r#"[{"address":"0xa","title":"Alpha document","class":"test.Alpha","pid":4242,
                     "workspace":{"id":7,"name":"7"},"at":[0,0],"size":[10,10],"focusHistoryID":1},
                    {"address":"0xb","title":"Beta document","class":"test.Beta","pid":4343,
                     "workspace":{"id":2,"name":"web"},"at":[0,0],"size":[10,10],"focusHistoryID":0},
                    {"address":"0xc","title":"Elsewhere","class":"test.Gamma","pid":1,
                     "workspace":{"id":9,"name":"9"},"focusHistoryID":2}]"#
                    .into(),
            ),
            (
                "-j/workspaces".into(),
                r#"[{"id":7,"name":"7","monitor":"HEADLESS-1"},{"id":2,"name":"web","monitor":"HEADLESS-1"}]"#
                    .into(),
            ),
            ("-j/activewindow".into(), "{}".into()),
        ],
    );
    let engine = Engine::start_with(&sway, "Hyprland", |_| {
        vec![("HYPRLAND_INSTANCE_SIGNATURE", "compass-test".into())]
    });

    let mut windows = Vec::new();
    assert!(
        eventually(WAIT, || {
            windows = engine.windows();
            windows.len() == 2
        }),
        "{windows:?}"
    );
    let found = |class: &str| windows.iter().find(|w| w.wm_class == class).unwrap();
    assert_eq!(
        (found("test.Alpha").pid, found("test.Alpha").workspace),
        (Some(4242), Some(7))
    );
    assert_eq!(
        (found("test.Beta").pid, found("test.Beta").workspace),
        (Some(4343), Some(2))
    );
    assert!(fake.seen().iter().any(|request| request == "-j/clients"));
}

#[test]
fn on_sway_doctor_reports_the_wlroots_protocols_it_found() {
    let Some(sway) = Sway::start("on_sway_doctor_reports") else {
        return;
    };
    let dirs = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .arg("--socket")
        .arg(dirs.path().join("ipc.sock"))
        .args(["doctor", "--json"])
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("HOME", dirs.path())
        .env("XDG_RUNTIME_DIR", sway.runtime_dir())
        .env("WAYLAND_DISPLAY", sway.display())
        .env("XDG_CURRENT_DESKTOP", "sway")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("NIRI_SOCKET")
        .output()
        .expect("run doctor");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "doctor did not print JSON ({err}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    let check = report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|check| check["name"] == "wlroots.capabilities")
        .unwrap_or_else(|| panic!("no wlroots check: {report}"))
        .clone();
    let detail = check["detail"].as_str().unwrap_or_default();
    for part in [
        "layer-shell: yes",
        "zwlr_foreign_toplevel_manager_v1",
        "data-control: yes",
        "hotkey protocol: no",
        "virtual-keyboard: yes",
        "shortcuts-inhibit: yes",
        "portal GlobalShortcuts: no",
        "compositor IPC: none",
        "no global hotkey",
    ] {
        assert!(detail.contains(part), "{part:?} not in {detail}");
    }
}

/// Child role: print the regular selection's text, as a paste would read it.
#[test]
fn child_prints_the_clipboard() {
    if support::child_role().is_none() {
        return;
    }
    match compass_wayland::clipboard::read("text/plain;charset=utf-8") {
        Ok(bytes) => println!(
            "CLIPBOARD:{}",
            String::from_utf8_lossy(&bytes.unwrap_or_default())
        ),
        Err(err) => println!("CLIPBOARD-ERROR:{err}"),
    }
}

fn clipboard_text(sway: &Sway) -> String {
    let child = sway.run_child("child_prints_the_clipboard", "read", &[]);
    let output = child.wait_with_output().expect("the reader ran");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|line| {
            line.split_once("CLIPBOARD:")
                .map(|(_, text)| text.to_owned())
        })
        .unwrap_or_else(|| format!("<no clipboard: {stdout}>"))
}

/// The chord a window was sent: each key with the modifier mask at it.
fn chord(window: &TestWindow) -> Vec<(u32, bool, u32)> {
    let mut mask = 0;
    let mut chord = Vec::new();
    for event in window.presses() {
        match event {
            support::KeyEvent::Modifiers(now) => mask = now,
            support::KeyEvent::Key(code, pressed) => chord.push((code, pressed, mask)),
            _ => {}
        }
    }
    chord
}

const NO_HELPER: &str = "/nonexistent/compass-test-no-input-server";

#[test]
fn on_sway_a_paste_is_copied_and_pressed_into_the_focused_window() {
    use compass_wayland::virtual_keyboard::{CONTROL_MASK, KEY_LEFTCTRL, KEY_V};
    let Some(sway) = Sway::start("on_sway_a_paste_is_copied_and_pressed") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let editor = TestWindow::open(&sway, "Editor", "test.Editor");
    assert!(eventually(WAIT, || editor
        .keys()
        .contains(&support::KeyEvent::Enter)));
    let engine = Engine::start_with(&sway, "sway", |_| {
        vec![("COMPASS_INPUT_SERVER_BIN", NO_HELPER.into())]
    });

    // Before, a wlroots session only copied, and the window was told so.
    assert_eq!(
        engine.request(Request::PasteText {
            text: "pasted words".into()
        }),
        Response::Ack
    );
    let want = vec![
        (KEY_LEFTCTRL, true, 0),
        (KEY_V, true, CONTROL_MASK),
        (KEY_V, false, CONTROL_MASK),
        (KEY_LEFTCTRL, false, CONTROL_MASK),
    ];
    assert!(
        eventually(WAIT, || chord(&editor) == want),
        "{:?}",
        editor.keys()
    );
    assert_eq!(clipboard_text(&sway), "pasted words");

    // A file's panel offers the paste, and pasting a file presses it too.
    let file = sway.runtime_dir().join("note.txt");
    std::fs::write(&file, "x").unwrap();
    let path = file.to_string_lossy().into_owned();
    let Response::FileActions(info) = engine.request(Request::FileActions { path: path.clone() })
    else {
        panic!("no file actions");
    };
    assert!(info.can_paste, "{info:?}");
    assert_eq!(
        engine.request(Request::CopyFile { path, paste: true }),
        Response::Ack
    );
    assert!(
        eventually(WAIT, || chord(&editor).len() == 2 * want.len()),
        "{:?}",
        editor.keys()
    );
}

#[test]
fn on_sway_a_terminal_is_pasted_into_with_ctrl_shift_v() {
    use compass_wayland::virtual_keyboard::{CONTROL_MASK, KEY_LEFTSHIFT, KEY_V, SHIFT_MASK};
    let Some(sway) = Sway::start("on_sway_a_terminal_is_pasted_into") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let terminal = TestWindow::open(&sway, "Terminal", "test.Terminal");
    let engine = Engine::start_with(&sway, "sway", |root| {
        let apps = root.join("data/applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("test.Terminal.desktop"),
            "[Desktop Entry]\nType=Application\nName=Test Terminal\nExec=true\n\
             Categories=System;TerminalEmulator;\n",
        )
        .unwrap();
        vec![("COMPASS_INPUT_SERVER_BIN", NO_HELPER.into())]
    });

    assert_eq!(
        engine.request(Request::PasteText { text: "ls".into() }),
        Response::Ack
    );
    let both = CONTROL_MASK | SHIFT_MASK;
    assert!(
        eventually(WAIT, || chord(&terminal).contains(&(KEY_V, true, both))),
        "the terminal was not sent Ctrl+Shift+V: {:?}",
        terminal.keys()
    );
    assert!(chord(&terminal).contains(&(KEY_LEFTSHIFT, true, CONTROL_MASK)));
}

#[test]
fn on_sway_the_input_server_presses_the_paste_when_it_runs() {
    let Some(sway) = Sway::start("on_sway_the_input_server_presses_the_paste") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let editor = TestWindow::open(&sway, "Editor", "test.Editor");
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_input_server.py");
    let log = std::sync::Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let log_path = std::sync::Arc::clone(&log);
    let engine = Engine::start_with(&sway, "sway", move |root| {
        let file = root.join("input-server.log");
        *log_path.lock().unwrap() = file.clone();
        vec![
            ("COMPASS_INPUT_SERVER_BIN", script.into_os_string()),
            ("FAKE_INPUT_LOG", file.into_os_string()),
        ]
    });
    let log = log.lock().unwrap().clone();
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        eventually(WAIT, || matches!(
            engine.request(Request::InputServerStatus),
            Response::InputServerStatus(status) if status.running && status.injection
        )),
        "the helper never ran"
    );

    assert_eq!(
        engine.request(Request::PasteText {
            text: "through the helper".into()
        }),
        Response::Ack
    );
    assert!(
        eventually(WAIT, || calls().contains("Snippet/injectPaste")),
        "the helper was not asked to paste: {}",
        calls()
    );
    assert!(calls().contains(r#""terminal": false"#), "{}", calls());
    assert!(
        chord(&editor).is_empty(),
        "the virtual keyboard pressed it as well: {:?}",
        editor.keys()
    );
    assert_eq!(clipboard_text(&sway), "through the helper");
}

/// `compass start` on the compositor: an engine, and the launcher as its
/// child, both on a private socket, with everything they print kept.
struct Started {
    child: Child,
    socket: PathBuf,
    dirs: tempfile::TempDir,
}

impl Started {
    fn start(sway: &Sway) -> Self {
        let dirs = tempfile::tempdir().unwrap();
        let socket = dirs.path().join("ipc.sock");
        let log = std::fs::File::create(dirs.path().join("start.log")).unwrap();
        let child = Command::new(binary())
            .arg("--socket")
            .arg(&socket)
            .args(["start", "--hidden"])
            .env("RUST_LOG", "info")
            .env("COMPASS_NO_ONBOARDING", "1")
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env("XDG_DATA_DIRS", dirs.path().join("empty"))
            .env("XDG_DATA_HOME", dirs.path().join("data"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            .env("XDG_CACHE_HOME", dirs.path().join("cache"))
            .env("HOME", dirs.path())
            .env_remove("XDG_STATE_HOME")
            .env("XDG_RUNTIME_DIR", sway.runtime_dir())
            .env("WAYLAND_DISPLAY", sway.display())
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env_remove("COMPASS_LAYER_SHELL")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("NIRI_SOCKET")
            .env(
                "COMPASS_UPDATE_FEED_URL",
                "http://127.0.0.1:9/releases/latest",
            )
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn compass start");
        let started = Self {
            child,
            socket,
            dirs,
        };
        // A hidden launcher acknowledges `Hide` once it has attached.
        assert!(
            eventually(WAIT, || started.launcher().is_some()
                && started.acks(Request::Hide)),
            "compass start never brought up an engine and a launcher:\n{}",
            started.log()
        );
        started
    }

    /// What `start` and its children printed.
    fn log(&self) -> String {
        std::fs::read_to_string(self.dirs.path().join("start.log")).unwrap_or_default()
    }

    /// The launcher process `start` is running, by pid.
    fn launcher(&self) -> Option<u32> {
        let pid = self.child.id();
        let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
        children
            .split_whitespace()
            .filter_map(|child| child.parse::<u32>().ok())
            .find(|child| {
                std::fs::read(format!("/proc/{child}/cmdline")).is_ok_and(|cmdline| {
                    cmdline
                        .split(|byte| *byte == 0)
                        .any(|arg| arg == b"launcher-child")
                })
            })
    }

    /// The engine's answer to `request`, or `None` when there is none within
    /// `within`: a launcher that never answers must fail the test, not hang it.
    fn answers(&self, request: Request, within: Duration) -> Option<Response> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(within, async {
                    let mut client = compass_ipc::Client::connect(&self.socket).await.ok()?;
                    client.request(request).await.ok()
                })
                .await
                .ok()
                .flatten()
            })
    }

    /// Whether `request` was acknowledged within ten seconds.
    fn acks(&self, request: Request) -> bool {
        self.answers(request, Duration::from_secs(10)) == Some(Response::Ack)
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        // `start` stops its engine only on a clean exit; take it and the
        // launcher down here.
        if let Some(launcher) = self.launcher() {
            let _ = Command::new("kill")
                .args(["-9", &launcher.to_string()])
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("pkill")
            .args(["-9", "-f"])
            .arg(self.socket.as_os_str())
            .status();
    }
}

/// Child role: take the clipboard `COMPASS_WLR_COPIES` times through
/// data-control, as another client copying would.
#[test]
fn child_copies_again_and_again() {
    if support::child_role().is_none() {
        return;
    }
    let copies: usize = std::env::var("COMPASS_WLR_COPIES")
        .unwrap()
        .parse()
        .unwrap();
    for n in 0..copies {
        let mut options = wl_clipboard_rs::copy::Options::new();
        options.clipboard(wl_clipboard_rs::copy::ClipboardType::Regular);
        options
            .copy(
                wl_clipboard_rs::copy::Source::Bytes(format!("copy {n}").into_bytes().into()),
                wl_clipboard_rs::copy::MimeType::Text,
            )
            .expect("copying");
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("CHILD-OK");
}

#[test]
fn on_sway_the_launcher_survives_copies_while_keyboards_come_and_go() {
    // TIL-01. The toolkit's clipboard released its `wl_data_device` whenever
    // the seat lost its keyboard or the launcher hid. A `data_offer` the
    // compositor had already sent then landed on the dead device, libwayland
    // dropped the object id it carried, and the next offer was a fatal "not a
    // valid new object id" on the launcher's connection. A seat whose
    // keyboards come and go (each virtual keyboard here, `wtype`, a KVM
    // switch) while another client copies hits it within a few dozen tries.
    let Some(sway) = Sway::start("the_launcher_survives_copies") else {
        return;
    };
    let started = Started::start(&sway);
    let launcher = started.launcher().expect("a launcher");
    assert!(started.acks(Request::Show), "{}", started.log());

    let copier = sway.run_child(
        "child_copies_again_and_again",
        "copy",
        &[("COMPASS_WLR_COPIES", "150")],
    );
    for n in 0..60 {
        drop(support::seat_keyboard(&sway));
        if n % 15 == 14 {
            assert!(started.acks(Request::Toggle), "{}", started.log());
        }
    }
    let copied = copier.wait_with_output().expect("the copier ran");
    assert!(
        String::from_utf8_lossy(&copied.stdout).contains("CHILD-OK"),
        "{}",
        String::from_utf8_lossy(&copied.stderr)
    );

    assert!(started.acks(Request::Toggle), "{}", started.log());
    let log = started.log();
    assert!(!log.contains("not a valid new object id"), "{log}");
    assert!(!log.contains("the launcher stopped"), "{log}");
    assert_eq!(
        started.launcher(),
        Some(launcher),
        "the launcher was restarted:\n{log}"
    );
}

#[test]
fn on_sway_a_failed_launcher_is_started_again_and_the_engine_kept() {
    // The other half of TIL-01: whatever ends the launcher's process, `start`
    // keeps the engine and brings the launcher back for the next summon.
    let Some(sway) = Sway::start("a_failed_launcher_is_started_again") else {
        return;
    };
    let started = Started::start(&sway);
    let engine = match started.answers(Request::Ping, WAIT) {
        Some(Response::Pong { pid, .. }) => pid,
        other => panic!("Ping answered {other:?}"),
    };
    let first = started.launcher().expect("a launcher");
    assert!(
        Command::new("kill")
            .args(["-9", &first.to_string()])
            .status()
            .unwrap()
            .success()
    );

    assert!(
        eventually(WAIT, || started.launcher().is_some_and(|now| now != first)),
        "no new launcher:\n{}",
        started.log()
    );
    assert!(
        eventually(WAIT, || started.acks(Request::Show)),
        "the new launcher never showed:\n{}",
        started.log()
    );
    assert!(
        matches!(
            started.answers(Request::Ping, WAIT),
            Some(Response::Pong { pid, .. }) if pid == engine
        ),
        "the engine did not survive the launcher"
    );
    assert!(
        started.log().contains("starting it again"),
        "{}",
        started.log()
    );
}
