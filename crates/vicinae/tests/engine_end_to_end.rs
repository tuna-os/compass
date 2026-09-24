//! The engine, end to end: a real process, a real socket, the real client.
//!
//! Every other test in this workspace exercises a library. These spawn the
//! `vicinae` binary as an actual daemon, talk to it over a Unix socket with the
//! same client code a user's shell runs, and assert on what comes back. That is
//! the only level at which "the port runs" is a claim rather than a hope.
//!
//! Each test gets its own socket under a `TempDir` and its own `XDG_DATA_DIRS`
//! pointing at a fixture tree, so nothing here can see — or be perturbed by —
//! the invoking user's real session or real applications.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// A session bus address with nothing behind it, for every engine this file
/// starts. The engine opens clipboard history through the login keyring on
/// the session bus; pointed at a real one, running these tests would create a
/// Compass key in the developer's own keyring.
const NO_SESSION_BUS: &str = "unix:path=/nonexistent/compass-test-no-session-bus";

/// How long to wait for the daemon to bind before calling it a failure.
///
/// Generous because CI runners are slow and a flaky timeout here would be
/// indistinguishable from a real bind failure, which is the thing worth
/// catching.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

fn binary() -> PathBuf {
    // The integration-test binary lives next to the crate's binaries.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("vicinae")
}

#[test]
fn graphical_session_starts_an_engine_and_reaps_only_its_own_child() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let mut command = Command::new(binary());
    command
        .arg("--socket")
        .arg(socket.as_path())
        .args(["serve", "--no-hotkey"])
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", dir.path().join("empty"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("HOME", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = vicinae::session::ensure(&socket, &mut command, STARTUP_TIMEOUT)
            .await
            .unwrap();
        assert!(vicinae::ipc::ping(&socket).await.is_ok());
        let mut must_not_spawn = Command::new("/does/not/exist");
        let guest = vicinae::session::ensure(&socket, &mut must_not_spawn, STARTUP_TIMEOUT)
            .await
            .unwrap();
        drop(guest);
        assert!(
            vicinae::ipc::ping(&socket).await.is_ok(),
            "reusing must not stop the engine"
        );
        drop(owner);
        assert!(
            vicinae::ipc::ping(&socket).await.is_err(),
            "owned engine must be reaped"
        );
    });
}

#[test]
fn graphical_session_reports_engine_startup_failure() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime
        .block_on(vicinae::session::ensure(
            &socket,
            &mut Command::new("/bin/false"),
            STARTUP_TIMEOUT,
        ))
        .expect_err("failed child must not count as ready");
    assert!(error.to_string().contains("exited before becoming ready"));
}

#[test]
fn graphical_session_does_not_replace_an_unresponsive_socket_owner() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(socket.as_path()).unwrap();
        let server = tokio::spawn(async move {
            let (_connection, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let error = vicinae::session::ensure(
            &socket,
            &mut Command::new("/does/not/exist"),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(
            !server.is_finished(),
            "existing socket owner must be left alone"
        );
        server.abort();
    });
}

/// A `.desktop` fixture tree, laid out the way XDG expects.
fn write_apps(root: &Path, entries: &[(&str, &str)]) {
    let dir = root.join("applications");
    std::fs::create_dir_all(&dir).expect("create applications dir");
    for (name, body) in entries {
        std::fs::write(dir.join(name), body).expect("write desktop entry");
    }
}

fn entry(name: &str, extra: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{extra}")
}

/// A daemon running on its own socket, killed on drop.
struct Daemon {
    child: Child,
    socket: PathBuf,
    _dirs: TempDir,
}

impl Daemon {
    fn start(entries: &[(&str, &str)]) -> Daemon {
        Self::start_with_config(entries, "{}")
    }

    fn start_with_config(entries: &[(&str, &str)], config: &str) -> Daemon {
        Self::start_prepared(entries, config, |_| Vec::new())
    }

    /// As [`Self::start_with_config`], with `prepare` given the tempdir root
    /// before the engine starts, returning extra environment for it.
    fn start_prepared(
        entries: &[(&str, &str)],
        config: &str,
        prepare: impl FnOnce(&std::path::Path) -> Vec<(&'static str, std::ffi::OsString)>,
    ) -> Daemon {
        let dirs = TempDir::new().expect("tempdir");
        let extra_env = prepare(dirs.path());
        let data = dirs.path().join("data");
        write_apps(&data, entries);
        let config_dir = dirs.path().join("config/vicinae");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("vicinae.json"), config).unwrap();

        let socket = dirs.path().join("ipc.sock");
        let child = Command::new(binary())
            .arg("--socket")
            .arg(&socket)
            .arg("serve")
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .env("XDG_DATA_DIRS", &data)
            // Keep the daemon out of the invoking user's home entirely: its
            // config, its launch history and its data all land in the tempdir.
            .env("XDG_DATA_HOME", dirs.path().join("data-home"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            .env("HOME", dirs.path())
            .envs(extra_env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the engine");

        let daemon = Daemon {
            child,
            socket,
            _dirs: dirs,
        };
        daemon.wait_until_listening();
        daemon
    }

    /// Blocks until the socket answers, or panics with why it never did.
    fn wait_until_listening(&self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if self.try_client(&["ping"]).status.success() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "the engine never started listening on {} within {STARTUP_TIMEOUT:?}",
            self.socket.display()
        );
    }

    fn try_client(&self, args: &[&str]) -> std::process::Output {
        Command::new(binary())
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .expect("run the client")
    }

    /// Runs a client command and asserts it succeeded, returning stdout.
    fn client(&self, args: &[&str]) -> String {
        let out = self.try_client(args);
        assert!(
            out.status.success(),
            "`vicinae {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    fn request(&self, request: compass_ipc::Request) -> compass_ipc::Response {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                compass_ipc::Client::connect(&self.socket)
                    .await
                    .unwrap()
                    .request(request)
                    .await
                    .unwrap()
            })
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------

#[test]
fn daemon_search_reads_application_aliases_and_enabled_precedence_from_config() {
    use compass_ipc::{Request, Response};
    let entries = [
        ("alpha.desktop", entry("Alpha Editor", "")),
        ("beta.desktop", entry("Beta Editor", "")),
    ];
    let entries = entries
        .iter()
        .map(|(id, body)| (*id, body.as_str()))
        .collect::<Vec<_>>();
    // `applications:beta`, not `beta.desktop` — the same `provider:entrypoint`
    // form the config below addresses these entries by.
    for (enabled, expected) in [(true, vec!["applications:beta"]), (false, vec![])] {
        let config = serde_json::json!({"providers": {"applications": {
            "enabled": enabled,
            "entrypoints": {
                "alpha": {"enabled": false},
                "beta": {"enabled": true, "alias": "uniquealias"}
            }
        }}});
        let daemon = Daemon::start_with_config(&entries, &config.to_string());
        for query in ["", "Editor", "uniquealias"] {
            let Response::QueryResults { hits } =
                daemon.request(Request::Query { text: query.into() })
            else {
                panic!("expected query result");
            };
            // Applications only: this is about their config. Builtin
            // commands rank in the same list and have their own test.
            assert_eq!(
                hits.iter()
                    .map(|hit| hit.id.as_str())
                    .filter(|id| id.starts_with("applications:"))
                    .collect::<Vec<_>>(),
                expected,
                "{query}"
            );
        }
    }
}

#[test]
fn the_engine_starts_and_answers_a_ping() {
    let daemon = Daemon::start(&[("true.desktop", &entry("True", ""))]);
    let out = daemon.client(&["ping"]);
    assert!(out.contains("engine alive"), "{out}");
    assert!(out.contains("protocol v"), "{out}");
}

#[test]
fn a_query_finds_an_application_from_the_indexed_directory() {
    let daemon = Daemon::start(&[
        ("editor.desktop", &entry("Text Editor", "Comment=Edit text")),
        ("browser.desktop", &entry("Web Browser", "")),
        ("calc.desktop", &entry("Calculator", "")),
    ]);

    let out = daemon.client(&["query", "text"]);
    assert!(out.contains("Text Editor"), "{out}");
    assert!(
        !out.contains("Calculator"),
        "a non-match was returned: {out}"
    );
}

#[test]
fn a_multi_word_query_is_joined_before_it_reaches_the_engine() {
    let daemon = Daemon::start(&[("editor.desktop", &entry("Text Editor", ""))]);
    let joined = daemon.client(&["query", "text", "editor"]);
    let quoted = daemon.client(&["query", "text editor"]);
    assert_eq!(joined, quoted);
    assert!(joined.contains("Text Editor"), "{joined}");
}

#[test]
fn reporting_a_launch_persists_history_and_changes_root_order() {
    use compass_core::FrecencyStore;
    use compass_ipc::{Request, Response};
    let daemon = Daemon::start(&[
        ("alpha.desktop", &entry("Alpha Editor", "")),
        ("beta.desktop", &entry("Beta Editor", "")),
    ]);
    let query = || {
        let Response::QueryResults { hits } = daemon.request(Request::Query {
            text: "Editor".to_owned(),
        }) else {
            panic!("query failed")
        };
        hits
    };
    let before = query();
    assert_eq!(before[0].id, "applications:alpha");
    assert_eq!(
        daemon.request(Request::RecordLaunch {
            key: "beta.desktop".to_owned()
        }),
        Response::Ack
    );
    let after = query();
    assert_eq!(after[0].id, "applications:beta");
    assert_eq!(
        after[0].score, before[1].score,
        "wire score excludes history"
    );
    let reopened = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/vicinae/frecency.json"),
    )
    .unwrap();
    let record = reopened.record("beta.desktop").unwrap();
    assert_eq!(record.launch_count, 1);
    assert!(record.last_launched_at.is_some());
    assert!(reopened.record("alpha.desktop").is_none());
}

#[test]
fn unknown_launch_keys_do_not_create_history_and_persistence_errors_are_reported() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let path = daemon._dirs.path().join("data-home/vicinae/frecency.json");
    for key in ["", "unknown.desktop", "../alpha.desktop"] {
        assert!(matches!(
            daemon.request(Request::RecordLaunch { key: key.to_owned() }),
            Response::Error(error) if error.kind == ErrorKind::BadRequest
        ));
    }
    assert!(!path.exists());
    std::fs::create_dir_all(&path).unwrap();
    assert!(matches!(
        daemon.request(Request::RecordLaunch { key: "alpha.desktop".to_owned() }),
        Response::Error(error) if error.kind == ErrorKind::Internal
    ));
    assert!(matches!(
        daemon.request(Request::Ping),
        Response::Pong { .. }
    ));
}

#[test]
fn concurrent_launch_reports_do_not_lose_visits() {
    use compass_core::FrecencyStore;
    use compass_ipc::{Client, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut requests = tokio::task::JoinSet::new();
            for _ in 0..16 {
                let socket = daemon.socket.clone();
                requests.spawn(async move {
                    Client::connect(socket)
                        .await
                        .unwrap()
                        .request(Request::RecordLaunch {
                            key: "alpha.desktop".to_owned(),
                        })
                        .await
                        .unwrap()
                });
            }
            while let Some(result) = requests.join_next().await {
                assert_eq!(result.unwrap(), Response::Ack);
            }
        });
    let reopened = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/vicinae/frecency.json"),
    )
    .unwrap();
    assert_eq!(reopened.record("alpha.desktop").unwrap().launch_count, 16);
}

#[test]
fn ui_search_and_successful_launch_share_the_real_daemons_history() {
    use compass_core::FrecencyStore;
    use compass_platform::{AppLauncher, LaunchFuture, LaunchMethod};
    use compass_ui::{LauncherApp, Message};
    use futures_util::StreamExt;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct Launcher(Mutex<Vec<String>>);
    impl AppLauncher for Launcher {
        fn launch<'a>(
            &'a self,
            entry: &'a compass_xdg::DesktopEntry,
            _uris: &'a [&'a str],
        ) -> LaunchFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(entry.name().to_owned());
                Ok(LaunchMethod::Direct)
            })
        }
    }

    let daemon = Daemon::start(&[
        ("alpha.desktop", &entry("Alpha Editor", "")),
        ("beta.desktop", &entry("Beta Editor", "")),
    ]);
    let launcher = Arc::new(Launcher::default());
    let build_ui = || {
        LauncherApp::with_index(
            compass_core::AppIndex::builder()
                .dir(daemon._dirs.path().join("data/applications"))
                .build(),
        )
        .with_launcher(launcher.clone())
        .with_backend(Arc::new(vicinae::ui_backend::DaemonBackend::new(
            compass_ipc::SocketPath::exact(&daemon.socket),
        )))
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let drive = |app: &mut LauncherApp, message| {
        let task = app.update(message);
        runtime.block_on(async {
            if let Some(mut stream) = iced_winit::runtime::task::into_stream(task) {
                while let Some(action) = stream.next().await {
                    if let iced_winit::runtime::Action::Output(message) = action {
                        let _ = app.update(message);
                    }
                }
            }
        });
    };
    let mut app = build_ui();
    drive(&mut app, Message::QueryChanged("Editor".to_owned()));
    assert_eq!(app.selected_item().unwrap().key(), "alpha.desktop");
    drive(&mut app, Message::ResultSelected(1));
    drive(&mut app, Message::LaunchSelected);
    assert_eq!(*launcher.0.lock().unwrap(), ["Beta Editor"]);
    let mut reopened_ui = build_ui();
    drive(
        &mut reopened_ui,
        Message::Opened(iced::window::Id::unique()),
    );
    assert_eq!(reopened_ui.selected_item().unwrap().key(), "beta.desktop");
    let history = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/vicinae/frecency.json"),
    )
    .unwrap();
    assert_eq!(history.record("beta.desktop").unwrap().launch_count, 1);
}

#[test]
fn a_desktop_link_without_exec_is_returned_by_root_search() {
    let daemon = Daemon::start(&[(
        "manual.desktop",
        "[Desktop Entry]\nType=Link\nName=Reference Manual\nURL=file:///usr/share/doc/manual.html\n",
    )]);
    for query in ["", "Reference"] {
        let out = daemon.client(&["query", query, "--json", "--provider", "applications"]);
        let rows: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        // The ENTRYPOINT id, which is what the protocol documents this field
        // as and what the C++ engine answers. It read `manual.desktop` until
        // Suite 0's first differential put the two engines side by side.
        assert_eq!(rows[0]["id"], "applications:manual");
        assert_eq!(rows[0]["title"], "Reference Manual");
    }
}

#[test]
fn an_empty_query_lists_everything_rather_than_nothing() {
    // The pre-typing state of a launcher. Returning nothing here would make the
    // window open blank, which reads as broken.
    let daemon = Daemon::start(&[
        ("a.desktop", &entry("Alpha", "")),
        ("b.desktop", &entry("Beta", "")),
    ]);
    let out = daemon.client(&["query", ""]);
    assert!(out.contains("Alpha"), "{out}");
    assert!(out.contains("Beta"), "{out}");
}

#[test]
fn a_query_that_matches_nothing_says_so() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.client(&["query", "zzzzzzqqq"]);
    assert_eq!(out.trim(), "no matches");
}

#[test]
fn query_json_is_machine_readable_and_carries_the_documented_fields() {
    let daemon = Daemon::start(&[(
        "editor.desktop",
        &entry("Text Editor", "GenericName=Editor"),
    )]);
    let out = daemon.client(&["query", "--json", "text"]);
    let hits: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let first = &hits[0];

    assert_eq!(first["title"], "Text Editor");
    assert!(
        first["subtitle"].is_null(),
        "application descriptions are not root subtitles"
    );
    assert!(first["id"].is_string(), "{first}");

    let score = first["score"].as_u64().expect("a numeric score");
    assert!(
        score <= 100,
        "score {score} is outside the documented 0..=100"
    );
}

#[test]
fn queries_use_root_provider_fields_and_do_not_return_desktop_actions() {
    let daemon = Daemon::start(&[(
        "browser.desktop",
        &entry(
            "Browser",
            "TryExec=compass-unresolved-sentinel\nComment=DescriptionSentinel\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=browser --private\n",
        ),
    )]);
    let all: serde_json::Value = serde_json::from_str(&daemon.client(&[
        "query",
        "--json",
        "--provider",
        "applications",
        "",
    ]))
    .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["id"], "applications:browser");
    for query in ["DescriptionSentinel", "Private"] {
        let hits: serde_json::Value =
            serde_json::from_str(&daemon.client(&["query", "--json", query])).unwrap();
        assert!(hits.as_array().unwrap().is_empty(), "{query}: {hits}");
    }
}

#[test]
fn the_window_commands_refuse_rather_than_pretend() {
    // The point of the design: a client must be able to tell "there is no
    // window" from "the window was shown". If these ever start answering Ack
    // without a UI behind them, first bring-up debugs a lie.
    //
    // Asserted on the *properties* of the refusal rather than on its wording.
    // An earlier version checked `stderr.contains("headless")`, which broke the
    // moment ADR-0015 made the refusal name a missing connection instead of a
    // headless build -- a more accurate message failing a test is the test
    // being wrong, not the message.
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    for command in ["show", "hide", "toggle"] {
        let out = daemon.try_client(&[command]);
        assert!(
            !out.status.success(),
            "`{command}` reported success with no window"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        assert!(
            stderr.contains("window"),
            "`{command}` refused without naming what is missing: {stderr}"
        );
        // Actionable: a refusal that does not say what to do about it sends
        // the reader to the source.
        assert!(
            stderr.contains("vicinae ui"),
            "`{command}` refused without saying how to fix it: {stderr}"
        );
    }
}

#[test]
fn doctor_runs_against_the_live_engine() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.client(&["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(report["checks"].as_array().is_some_and(|c| !c.is_empty()));
}

#[test]
fn shutdown_is_answered_before_the_engine_goes_away() {
    // The response has to be written before the serve loop stops, or the client
    // sees its connection drop and reports a crash instead of a clean stop.
    let mut daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.try_client(&["shutdown"]);
    assert!(
        out.status.success(),
        "shutdown did not report success: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match daemon.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "the engine exited with {status}");
                break;
            }
            None if Instant::now() >= deadline => panic!("the engine did not exit after shutdown"),
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }

    // And the socket file is gone, so the next start does not have to reclaim it.
    assert!(
        !daemon.socket.exists(),
        "the socket file outlived the engine"
    );
}

#[test]
fn a_second_engine_on_the_same_socket_refuses_to_start() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("serve")
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .output()
        .expect("run a second engine");

    assert!(!out.status.success(), "two engines bound the same socket");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already running"),
        "the second engine did not say why: {stderr}"
    );
}

#[test]
fn a_client_with_no_engine_explains_itself() {
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("absent.sock");
    let out = Command::new(binary())
        .arg("--socket")
        .arg(&socket)
        .arg("ping")
        .output()
        .expect("run the client");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&socket.display().to_string()),
        "the error did not name the path it tried: {stderr}"
    );
}

// --- the resident window, against a real daemon -----------------------------
//
// ADR-0015: `serve` forwards `show`/`hide`/`toggle` to a window that attached
// itself over the same socket. These tests *are* that window: the test process
// attaches a `WindowClient` to a real spawned daemon and answers what it is
// asked, which exercises the handover, the push and the reply across a process
// boundary rather than inside one.

/// A fake launcher window attached to a running daemon, on its own thread.
///
/// Answers every command with `reports`, and records what it was asked.
struct FakeWindow {
    seen: std::sync::Arc<std::sync::Mutex<Vec<compass_ipc::WindowCommand>>>,
    /// Told on drop, so the loop returns and the socket closes.
    ///
    /// Without it, dropping the window while the daemon still holds the link
    /// joins a thread parked forever in `next_command`. A test that wants to
    /// kill the window has to be able to do it while the engine is still
    /// alive, which is exactly what the death test needs.
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeWindow {
    fn attach(socket: &Path, reports: compass_ipc::WindowOutcome) -> Self {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

        let thread = {
            let seen = std::sync::Arc::clone(&seen);
            let socket = socket.to_path_buf();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let mut window = compass_ipc::WindowClient::attach(&socket)
                        .await
                        .expect("attach to the engine");
                    ready_tx.send(()).expect("signal attached");

                    loop {
                        tokio::select! {
                            // `biased` so a pending stop wins over a command
                            // that arrived in the same poll: a window asked to
                            // die should die rather than answer once more.
                            biased;
                            _ = &mut stop_rx => break,
                            command = window.next_command() => {
                                match command.expect("read a command") {
                                    Some(command) => {
                                        seen.lock().expect("lock").push(command);
                                        window.reply(reports.clone()).await.expect("reply");
                                    }
                                    // The engine closed the link.
                                    None => break,
                                }
                            }
                        }
                    }
                    drop(window);
                });
            })
        };

        // Attach before returning: a test that raced the attach would exercise
        // the refusal path and still look like it passed.
        ready_rx
            .recv_timeout(STARTUP_TIMEOUT)
            .expect("the fake window attached");

        Self {
            seen,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn seen(&self) -> Vec<compass_ipc::WindowCommand> {
        self.seen.lock().expect("lock").clone()
    }
}

impl Drop for FakeWindow {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            // Joining rather than detaching: the death test needs the socket
            // closed by the time this returns, and a detached runtime would
            // close it whenever it got around to it.
            let _ = thread.join();
        }
    }
}

#[test]
fn an_attached_window_turns_the_refusal_into_a_real_show() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    // Control: the same command against the same daemon is refused a moment
    // before the window attaches. Without this the test could pass against an
    // engine that answered `Ack` unconditionally.
    let before = daemon.try_client(&["show"]);
    assert!(
        !before.status.success(),
        "`show` must be refused before a window attaches"
    );

    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);

    let after = daemon.try_client(&["show"]);
    assert!(
        after.status.success(),
        "`show` must succeed with a window attached: {}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
}

#[test]
fn a_second_ui_invocation_shows_the_existing_window_without_starting_a_renderer() {
    let daemon = Daemon::start(&[]);
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(daemon.socket.with_extension("ui.lock"))
        .unwrap();
    lease.try_lock().unwrap();
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    let output = Command::new(binary())
        .args(["--socket", daemon.socket.to_str().unwrap(), "ui"])
        .env("DISPLAY", ":65534")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
    let hidden = Command::new(binary())
        .args([
            "--socket",
            daemon.socket.to_str().unwrap(),
            "start",
            "--hidden",
        ])
        .env("DISPLAY", ":65534")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(
        hidden.status.success(),
        "{}",
        String::from_utf8_lossy(&hidden.stderr)
    );
    assert_eq!(
        window.seen(),
        vec![compass_ipc::WindowCommand::Show],
        "a duplicate hidden start must not change visibility"
    );
}

#[test]
fn each_command_reaches_the_window_as_itself() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);

    // Three different commands, so a daemon that forwarded a constant would
    // produce a different vector rather than an identical-looking one.
    for command in ["show", "hide", "toggle"] {
        let out = daemon.try_client(&[command]);
        assert!(
            out.status.success(),
            "`{command}` failed with a window attached: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    assert_eq!(
        window.seen(),
        vec![
            compass_ipc::WindowCommand::Show,
            compass_ipc::WindowCommand::Hide,
            compass_ipc::WindowCommand::Toggle,
        ]
    );
}

#[test]
fn a_window_that_refuses_is_reported_rather_than_acked() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let reason = "no compositor to present on";
    let _window = FakeWindow::attach(
        &daemon.socket,
        compass_ipc::WindowOutcome::Failed(reason.to_owned()),
    );

    let out = daemon.try_client(&["show"]);
    assert!(
        !out.status.success(),
        "a window that refused must not be reported as a success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(reason),
        "the window's own reason should reach the user: {stderr}"
    );
}

#[test]
fn a_window_that_dies_puts_the_engine_back_to_refusing() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    {
        let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
        let out = daemon.try_client(&["show"]);
        assert!(out.status.success(), "control: `show` works while attached");
        assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
    }
    // The fake window's runtime is gone here, so its socket is closed.

    // The first request after the death may be the one that discovers it --
    // the engine notices on a push, not before -- so a single failed attempt
    // proves nothing on its own. What must hold is that it *settles* on
    // refusing rather than alternating.
    //
    // What this does *not* pin is the engine clearing its slot: a push to a
    // dead socket fails whether or not the link was dropped, so the refusal
    // looks identical. Confirmed by a control -- these tests still pass with
    // the clearing removed. See `forward` in `serve.rs`.
    for attempt in 0..3 {
        let out = daemon.try_client(&["show"]);
        assert!(
            !out.status.success(),
            "attempt {attempt}: `show` reported success after the window died"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        assert!(
            stderr.contains("vicinae ui"),
            "attempt {attempt}: the refusal should say how to fix it: {stderr}"
        );
    }
}

#[test]
fn clipboard_history_without_a_keyring_is_refused_by_name() {
    // Every engine here runs with no session bus, so no keyring: the history
    // cannot be opened, and the request must say so rather than answer with
    // an empty list a client would show as "nothing copied yet".
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    for request in [
        compass_ipc::Request::ClipboardHistory {
            query: String::new(),
            limit: 10,
        },
        compass_ipc::Request::ClipboardSetPinned {
            id: "1".into(),
            pinned: true,
        },
        compass_ipc::Request::ClipboardRemove { id: "1".into() },
    ] {
        let response = daemon.request(request.clone());
        let compass_ipc::Response::Error(err) = response else {
            panic!("expected {request:?} to be refused, got {response:?}");
        };
        assert_eq!(err.kind, compass_ipc::ErrorKind::Unsupported, "{request:?}");
        assert!(err.message.contains("keyring"), "{}", err.message);
    }
}

#[test]
fn a_clipboard_request_for_no_entries_is_a_bad_request() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let response = daemon.request(compass_ipc::Request::ClipboardHistory {
        query: String::new(),
        limit: 0,
    });
    let compass_ipc::Response::Error(err) = response else {
        panic!("expected a refusal, got {response:?}");
    };
    assert_eq!(err.kind, compass_ipc::ErrorKind::BadRequest);
}

#[test]
fn builtin_commands_rank_in_the_root_and_their_use_is_remembered() {
    use compass_ipc::{Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "clipboard".into(),
    }) else {
        panic!("expected query results");
    };
    assert_eq!(
        hits.first().map(|h| h.id.as_str()),
        Some("commands:clipboard-history")
    );
    assert_eq!(hits[0].title, "Clipboard History");

    // The provider flag narrows either way.
    let commands: serde_json::Value =
        serde_json::from_str(&daemon.client(&["query", "--json", "--provider", "commands", ""]))
            .unwrap();
    assert!(commands.as_array().unwrap().iter().all(|row| {
        row["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("commands:"))
    }));

    // Opening it counts, like launching an application.
    assert!(matches!(
        daemon.request(Request::RecordLaunch {
            key: "commands:clipboard-history".into()
        }),
        Response::Ack
    ));
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: String::new(),
    }) else {
        panic!("expected query results");
    };
    assert_eq!(
        hits.first().map(|h| h.id.as_str()),
        Some("commands:clipboard-history"),
        "the most-used row leads the empty query"
    );
}

#[test]
fn window_requests_without_a_session_bus_are_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    for request in [
        Request::ListWindows,
        Request::ActivateWindow { id: 1 },
        Request::CloseWindow { id: 1 },
        // Before the store is consulted: without a Shell there is nothing to
        // paste into, whether or not the id names an entry.
        Request::ClipboardPaste { id: "1".into() },
    ] {
        let Response::Error(err) = daemon.request(request.clone()) else {
            panic!("{request:?} was not refused");
        };
        assert_eq!(err.kind, ErrorKind::Unsupported, "{request:?}");
        assert!(err.message.contains("session bus"), "{}", err.message);
    }
}

/// An installed extension with one no-view command that writes `out`, and one
/// view command.
fn install_extension(root: &std::path::Path) -> std::path::PathBuf {
    let ext = root.join("data-home/vicinae/extensions/hello");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "hello", "title": "Hello", "author": "someone",
            "commands": [
              {"name": "write", "title": "Write Greeting", "mode": "no-view"},
              {"name": "show", "title": "Show Greeting", "mode": "view"},
              {"name": "nav", "title": "Navigate", "mode": "view"},
              {"name": "ask", "title": "Ask First", "mode": "view"},
              {"name": "greet", "title": "Greet Someone", "mode": "no-view",
               "arguments": [{"name": "name", "type": "text", "placeholder": "Name",
                              "required": true}]},
              {"name": "needs", "title": "Needs Token", "mode": "no-view",
               "preferences": [{"name": "token", "title": "API Token", "type": "password",
                                "required": true}]}
            ]}"#,
    )
    .unwrap();
    // In the extension's support directory, one of the two places the sandbox
    // lets it write. Not the tempdir: that is under /tmp, which it may not.
    let out = root.join("data-home/vicinae/support/hello/greeting.txt");
    let answered = root.join("data-home/vicinae/support/hello/answered.txt");
    std::fs::write(
        ext.join("ask.js"),
        format!(
            "const React = require('react');
             const {{ List, ActionPanel, Action, confirmAlert }} = require('@vicinae/api');
             module.exports.default = () => React.createElement(List, null,
               React.createElement(List.Item, {{ title: 'risky', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Delete', onAction: async () => {{
                     const ok = await confirmAlert({{ title: 'Delete it?' }});
                     require('node:fs').writeFileSync({answered:?}, String(ok));
                   }} }}))
               }}));",
            answered = answered.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("nav.js"),
        "const React = require('react');
         const { List, Detail, ActionPanel, Action, useNavigation } = require('@vicinae/api');
         module.exports.default = function Root() {
           const { push } = useNavigation();
           return React.createElement(List, null,
             React.createElement(List.Item, { title: 'go', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Push', onAction: () =>
                   push(React.createElement(Detail, { markdown: 'pushed' })) }))
             }));
         };",
    )
    .unwrap();
    let acted = root.join("data-home/vicinae/support/hello/acted.txt");
    std::fs::write(
        ext.join("show.js"),
        format!(
            "const React = require('react');
             const {{ List, ActionPanel, Action }} = require('@vicinae/api');
             module.exports.default = () => React.createElement(List, {{ navigationTitle: 'Greetings' }},
               React.createElement(List.Item, {{ title: 'hello', id: 'h', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Act', onAction: () =>
                     require('node:fs').writeFileSync({acted:?}, 'acted') }}))
               }}));",
            acted = acted.to_string_lossy()
        ),
    )
    .unwrap();
    let greeted = root.join("data-home/vicinae/support/hello/greeted.txt");
    std::fs::write(
        ext.join("greet.js"),
        format!(
            "module.exports.default = async (props) => {{
               require('node:fs').writeFileSync({greeted:?}, 'hi ' + props.arguments.name);
             }};",
            greeted = greeted.to_string_lossy()
        ),
    )
    .unwrap();
    // Outside every path the sandbox grants: the data home itself, beside the
    // support directory the command may write.
    let escape = root.join("data-home/escape.txt");
    std::fs::write(
        ext.join("write.js"),
        format!(
            "module.exports.default = async () => {{
               const fs = require('node:fs');
               let escaped = 'wrote';
               try {{ fs.writeFileSync({escape:?}, 'x'); }} catch (e) {{ escaped = e.code; }}
               fs.writeFileSync({out:?}, 'hi:' + escaped);
             }};",
            escape = escape.to_string_lossy(),
            out = out.to_string_lossy()
        ),
    )
    .unwrap();
    out
}

fn extension_runtime() -> Option<std::path::PathBuf> {
    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/typescript/extension-manager/dist/runtime.js");
    std::env::var_os("COMPASS_EXTENSION_RUNTIME")
        .map(Into::into)
        .or_else(|| built.is_file().then_some(built))
}

#[test]
fn an_installed_extension_command_is_found_and_a_no_view_one_runs() {
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut out = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        out = install_extension(root);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });

    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "write greeting".into(),
    }) else {
        panic!("no results");
    };
    let hit = hits.first().expect("a hit");
    assert_eq!(hit.id, "@someone/hello:write");
    assert_eq!(hit.subtitle.as_deref(), Some("Hello"));

    let started = daemon.request(Request::RunExtensionCommand {
        id: hit.id.clone(),
        arguments_json: None,
    });
    assert_eq!(started, Response::Ack, "{started:?}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !out.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(&out).ok().as_deref(),
        Some("hi:EACCES"),
        "the command ran, confined: a write outside its directories is refused"
    );

    let Response::Error(err) = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:nothing".into(),
        arguments_json: None,
    }) else {
        panic!("an unknown id was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn a_view_command_renders_and_its_action_runs() {
    use compass_extension_api::View;
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });

    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:show".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("the view command did not start a session: {started:?}");
    };

    let mut after = 0;
    let view = loop {
        let answer = daemon.request(Request::ExtensionView { session, after });
        let Response::ExtensionView {
            version,
            view_json,
            problem,
            ended,
            ..
        } = answer
        else {
            panic!("no view answer: {answer:?}");
        };
        assert!(!ended, "the command ended: {problem:?}");
        assert_eq!(problem, None);
        if let Some(json) = view_json {
            break serde_json::from_str::<View>(&json).expect("a View");
        }
        after = version;
    };
    let View::List(list) = view else {
        panic!("not a list: {view:?}");
    };
    assert_eq!(list.navigation_title.as_deref(), Some("Greetings"));
    let item = &list.sections[0].items[0];
    assert_eq!(item.title, "hello");
    let action = item.actions.as_ref().expect("actions").actions()[0].clone();
    assert_eq!(action.title, "Act");

    let acted = root.join("data-home/vicinae/support/hello/acted.txt");
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: action.handler.0.clone(),
            args_json: "[]".into(),
        }),
        Response::Ack
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !acted.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(&acted).ok().as_deref(),
        Some("acted"),
        "the action's onAction ran in the extension"
    );

    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
    let Response::Error(err) = daemon.request(Request::ExtensionView { session, after: 0 }) else {
        panic!("a closed session still answers");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

/// Polls `session` until `done` accepts its view, returning it and the depth.
fn wait_for_view(
    daemon: &Daemon,
    session: u64,
    done: impl Fn(&compass_extension_api::View, u32) -> bool,
) -> (compass_extension_api::View, u32) {
    use compass_ipc::{Request, Response};
    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let answer = daemon.request(Request::ExtensionView { session, after });
        let Response::ExtensionView {
            version,
            view_json,
            problem,
            ended,
            depth,
            ..
        } = answer
        else {
            panic!("no view answer: {answer:?}");
        };
        assert!(!ended && problem.is_none(), "ended: {problem:?}");
        if let Some(view) = view_json.and_then(|json| serde_json::from_str(&json).ok())
            && done(&view, depth)
        {
            return (view, depth);
        }
        after = version;
    }
    panic!("the view never got there");
}

#[test]
fn a_pushed_view_shows_and_escape_pops_back_to_the_list() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:nav".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };

    let (root, depth) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    assert_eq!(depth, 1);
    let View::List(list) = root else {
        unreachable!()
    };
    let push = list.sections[0].items[0]
        .actions
        .as_ref()
        .expect("actions")
        .actions()[0]
        .handler
        .0
        .clone();

    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: push,
            args_json: "[]".into(),
        }),
        Response::Ack
    );
    let (pushed, depth) =
        wait_for_view(&daemon, session, |view, _| matches!(view, View::Detail(_)));
    assert_eq!(depth, 2, "the pushed view is on top of the list");
    let View::Detail(detail) = pushed else {
        unreachable!()
    };
    assert_eq!(detail.markdown.as_deref(), Some("pushed"));

    assert_eq!(
        daemon.request(Request::ExtensionPop { session }),
        Response::Ack
    );
    let (_, depth) = wait_for_view(&daemon, session, |view, depth| {
        matches!(view, View::List(_)) && depth == 1
    });
    assert_eq!(depth, 1, "popped back to the list");
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn an_alert_reaches_the_launcher_and_its_answer_reaches_the_extension() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:ask".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    let (view, _) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    let View::List(list) = view else {
        unreachable!()
    };
    let delete = list.sections[0].items[0]
        .actions
        .as_ref()
        .expect("actions")
        .actions()[0]
        .handler
        .0
        .clone();
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: delete,
            args_json: "[]".into(),
        }),
        Response::Ack
    );

    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    let alert = loop {
        assert!(Instant::now() < deadline, "no alert arrived");
        let Response::ExtensionView { version, alert, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        if let Some(alert) = alert {
            break alert;
        }
        after = version;
    };
    assert_eq!(alert.title, "Delete it?");

    assert_eq!(
        daemon.request(Request::ExtensionAlertAnswer {
            session,
            confirmed: true
        }),
        Response::Ack
    );
    let answered = root.join("data-home/vicinae/support/hello/answered.txt");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !answered.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(&answered).ok().as_deref(),
        Some("true"),
        "the extension's confirmAlert resolved with the person's answer"
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn a_required_preference_without_a_keyring_is_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // These engines run with no session bus, so no keyring: there is nowhere
    // safe to keep the token, and the engine says so rather than asking.
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let answer = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:needs".into(),
        arguments_json: None,
    });
    let Response::Error(err) = answer else {
        panic!("not refused: {answer:?}");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(
        err.message.contains("API Token") && err.message.contains("keyring"),
        "{}",
        err.message
    );
}

#[test]
fn a_command_with_arguments_is_asked_for_them_then_runs_with_them() {
    use compass_ipc::{PreferenceFieldKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut greeted = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        greeted = dir.join("data-home/vicinae/support/hello/greeted.txt");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let run = |arguments_json: Option<&str>| {
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:greet".into(),
            arguments_json: arguments_json.map(str::to_owned),
        })
    };

    let Response::ExtensionNeedsArguments { title, fields } = run(None) else {
        panic!("not asked for its arguments");
    };
    assert_eq!(title, "Greet Someone");
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].name.as_str(), fields[0].title.as_str()),
        ("name", "Name")
    );
    assert!(fields[0].required && fields[0].kind == PreferenceFieldKind::Text);

    assert!(
        matches!(
            run(Some(r#"{"name": ""}"#)),
            Response::ExtensionNeedsArguments { .. }
        ),
        "an empty required argument is asked for again"
    );
    assert!(!greeted.exists(), "and nothing ran");

    assert_eq!(run(Some(r#"{"name": "Ada"}"#)), Response::Ack);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !greeted.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(&greeted).expect("the command ran"),
        "hi Ada"
    );
}
