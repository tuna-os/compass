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
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
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
            // The file indexer's database, which a real cache home must never
            // receive from a test.
            .env("XDG_CACHE_HOME", dirs.path().join(".cache"))
            .env("HOME", dirs.path())
            // Nor its compositor: on a wlroots session the engine would answer
            // window requests over Wayland (`tests/wlroots_engine.rs`).
            .env_remove("WAYLAND_DISPLAY")
            // Nor its compositor's own socket (Hyprland, niri), nor its
            // desktop: the wallpaper backends are chosen by name.
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("NIRI_SOCKET")
            .env_remove("XDG_CURRENT_DESKTOP")
            .env_remove("GDMSESSION")
            // Nor the network: a test's HTTP goes to its own local fake,
            // never through a proxy the invoking shell set.
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env("NO_PROXY", "127.0.0.1,localhost")
            // Nor the machine's keyboards: a helper that exits at once
            // stands in for vicinae-input-server unless a test brings its
            // own fake.
            .env("VICINAE_INPUT_SERVER_BIN", "/bin/true")
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
              {"name": "auth", "title": "Sign In", "mode": "view"},
              {"name": "link", "title": "Open Link", "mode": "no-view"},
              {"name": "term", "title": "In Terminal", "mode": "no-view"},
              {"name": "tiles", "title": "Tiles", "mode": "view"},
              {"name": "toaster", "title": "Toaster", "mode": "view"},
              {"name": "heap", "title": "Heap", "mode": "no-view"},
              {"name": "machine", "title": "Machine", "mode": "view"},
              {"name": "probe", "title": "Probe", "mode": "no-view",
               "preferences": [{"name": "limit", "title": "Limit", "type": "textfield",
                                "required": false, "default": "20"}]},
              {"name": "issue", "title": "New Issue", "mode": "view"},
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
    let replaced = root.join("data-home/vicinae/support/hello/replaced.txt");
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
               }}),
               React.createElement(List.Item, {{ title: 'twice', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Ask twice', onAction: async () => {{
                     const first = confirmAlert({{ title: 'First?' }});
                     confirmAlert({{ title: 'Second?' }});
                     require('node:fs').writeFileSync({replaced:?}, String(await first));
                   }} }}))
               }}));",
            answered = answered.to_string_lossy(),
            replaced = replaced.to_string_lossy()
        ),
    )
    .unwrap();
    let authorized = root.join("data-home/vicinae/support/hello/authorized.txt");
    std::fs::write(
        ext.join("auth.js"),
        format!(
            "const React = require('react');
             const {{ List, OAuth }} = require('@vicinae/api');
             module.exports.default = function Auth() {{
               React.useEffect(() => {{
                 const client = new OAuth.PKCEClient({{
                   redirectMethod: OAuth.RedirectMethod.Web,
                   providerName: 'Example', description: 'Connect your Example account' }});
                 client.authorizationRequest({{
                   endpoint: 'https://example.com/authorize', clientId: 'id', scope: 'read' }})
                   .then((request) => client.authorize(request))
                   .then(({{ authorizationCode }}) => authorizationCode, (e) => 'refused:' + e.message)
                   .then((out) => require('node:fs').writeFileSync({authorized:?}, out));
               }}, []);
               return React.createElement(List, null, React.createElement(List.Item, {{ title: 'waiting' }}));
             }};",
            authorized = authorized.to_string_lossy()
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
    let submitted = root.join("data-home/vicinae/support/hello/submitted.json");
    std::fs::write(
        ext.join("issue.js"),
        format!(
            "const React = require('react');
             const {{ Form, ActionPanel, Action }} = require('@vicinae/api');
             module.exports.default = function Issue() {{
               const [title, setTitle] = React.useState('');
               return React.createElement(Form, {{ actions:
                   React.createElement(ActionPanel, null,
                     React.createElement(Action.SubmitForm, {{ title: 'Create', onSubmit: (values) =>
                       require('node:fs').writeFileSync({submitted:?},
                         JSON.stringify({{ values, title }})) }}))
                 }},
                 React.createElement(Form.TextField, {{ id: 'title', title: 'Title',
                   value: title, onChange: setTitle }}),
                 React.createElement(Form.Checkbox, {{ id: 'urgent', label: 'Urgent',
                   defaultValue: false }}));
             }};",
            submitted = submitted.to_string_lossy()
        ),
    )
    .unwrap();
    let probed = root.join("data-home/vicinae/support/hello/probe.json");
    std::fs::write(
        ext.join("probe.js"),
        format!(
            "module.exports.default = async () => {{
               const {{ environment, getPreferenceValues }} = require('@vicinae/api');
               const {{ spawnSync }} = require('node:child_process');
               const fs = require('node:fs');
               const status = (cmd, args) => {{
                 const r = spawnSync(cmd, args);
                 return r.error ? String(r.error.code) : r.status;
               }};
               // A program the command writes itself, into the one place it
               // may write, and then tries to run.
               const dropped = environment.supportPath + '/dropped';
               const original = ['/usr/bin/true', '/bin/true'].find((p) => fs.existsSync(p));
               fs.copyFileSync(original, dropped);
               fs.chmodSync(dropped, 0o755);
               // Outside the JavaScript heap, which the heap cap cannot see.
               let bigBuffer;
               try {{ bigBuffer = Buffer.alloc(512 * 1024 * 1024).length; }}
               catch (e) {{ bigBuffer = e.name; }}
               fs.writeFileSync({probed:?}, JSON.stringify({{
                 execDropped: status(dropped, []),
                 bigBuffer,
                 assetsPath: environment.assetsPath,
                 supportPath: environment.supportPath,
                 isDevelopment: environment.isDevelopment,
                 extensionName: environment.extensionName,
                 commandName: environment.commandName,
                 preferences: getPreferenceValues(),
                 execTrue: status('true', []),
                 execUnshare: status('unshare', ['-U', 'true']),
               }}));
             }};",
            probed = probed.to_string_lossy()
        ),
    )
    .unwrap();
    // `heap_size_limit` would be the obvious probe, and it lies in a worker
    // thread (it reports the process figure), so this allocates instead.
    let heap = root.join("data-home/vicinae/support/hello/heap");
    std::fs::write(
        ext.join("heap.js"),
        format!(
            "module.exports.default = async () => {{
               const fs = require('node:fs');
               fs.writeFileSync({started:?}, 'started');
               const keep = [];
               for (let i = 0; i < 400; i++) keep.push(new Array(128 * 1024).fill(i + 0.5));
               fs.writeFileSync({survived:?}, String(keep.length));
             }};",
            started = heap.with_extension("started").to_string_lossy(),
            survived = heap.with_extension("survived").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("toaster.js"),
        "const React = require('react');
         const { List, ActionPanel, Action, showToast, Toast } = require('@vicinae/api');
         let toast;
         module.exports.default = function Toaster() {
           React.useEffect(() => {
             showToast({ style: Toast.Style.Failure, title: 'Offline', message: 'retrying' })
               .then((t) => { toast = t; });
           }, []);
           return React.createElement(List, null,
             React.createElement(List.Item, { title: 'hide', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Hide', onAction: () => toast.hide() }))
             }));
         };",
    )
    .unwrap();
    // Asks the host what a command can learn about the desktop, and shows the
    // answers (or the rejections) in one line.
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
               said(WindowManagement.getWindows()),
               said(WindowManagement.getScreens()),
               said(WindowManagement.getActiveWorkspace()),
             ]).then((answers) => setText(answers.join(' | ')));
           }, []);
           return React.createElement(Detail, { markdown: text || 'asking' });
         };",
    )
    .unwrap();
    std::fs::write(
        ext.join("tiles.js"),
        "const React = require('react');
         const { Grid, ActionPanel, Action } = require('@vicinae/api');
         module.exports.default = () => React.createElement(Grid, { columns: 4 },
           React.createElement(Grid.Section, { title: 'Weather' },
             React.createElement(Grid.Item, { title: 'sun', content: 'sun-16', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Pick', onAction: () => {} }))
             }),
             React.createElement(Grid.Item, { title: 'red', content: { color: '#ff0000' } })));",
    )
    .unwrap();
    std::fs::write(
        ext.join("term.js"),
        "const { runInTerminal } = require('@vicinae/api');
         module.exports.default = async () => {
           await runInTerminal(['htop', '-d', '5'], { title: 'Top' });
         };",
    )
    .unwrap();
    std::fs::write(
        ext.join("link.js"),
        "const { open } = require('@vicinae/api');
         module.exports.default = async () => { await open('https://example.com/a b'); };",
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

/// Waits up to 20 s for `path` to exist with something in it: an extension's
/// `writeFileSync` creates the file before it writes, so existing is not enough.
fn wait_for_content(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !std::fs::metadata(path).is_ok_and(|m| m.len() > 0) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
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
    wait_for_content(&out);
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
    wait_for_content(&acted);
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
fn without_a_desktop_the_selection_and_window_apis_answer_as_the_cpp_does() {
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
        id: "@someone/hello:machine".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    // No session bus and no Wayland display: no Shell extension, no
    // compositor. Every call is answered, none of them "not implemented".
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Detail(detail) if detail.markdown.as_deref() != Some("asking")),
    );
    let View::Detail(detail) = view else {
        unreachable!()
    };
    assert_eq!(
        detail.markdown.as_deref(),
        Some(
            "error: Unable to get selected text | error: No active window | [] | [] \
             | error: No active workspace"
        )
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
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
    wait_for_content(&answered);
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

/// Waits for `session` to show an alert titled `title`.
fn wait_for_alert(daemon: &Daemon, session: u64, title: &str) {
    use compass_ipc::{Request, Response};
    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "no alert {title:?} arrived");
        let Response::ExtensionView { version, alert, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        if alert.is_some_and(|alert| alert.title == title) {
            return;
        }
        after = version;
    }
}

#[test]
fn a_replaced_alert_and_one_navigated_away_from_both_answer_no() {
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
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:ask".into(),
        arguments_json: None,
    }) else {
        panic!("no session");
    };
    let (view, _) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    let View::List(list) = view else {
        unreachable!()
    };
    let handler = |row: usize| {
        list.sections[0].items[row]
            .actions
            .as_ref()
            .expect("actions")
            .actions()[0]
            .handler
            .0
            .clone()
    };
    let support = root.join("data-home/vicinae/support/hello");

    // A second alert while the first is open: the first is cancelled, and
    // the extension is told so rather than left waiting.
    daemon.request(Request::ExtensionEvent {
        session,
        handler: handler(1),
        args_json: "[]".into(),
    });
    wait_for_alert(&daemon, session, "Second?");
    let replaced = support.join("replaced.txt");
    wait_for_content(&replaced);
    assert_eq!(
        std::fs::read_to_string(&replaced).ok().as_deref(),
        Some("false"),
        "the replaced alert's promise settled with no"
    );
    assert_eq!(
        daemon.request(Request::ExtensionAlertAnswer {
            session,
            confirmed: false
        }),
        Response::Ack
    );

    // Leaving the view while an alert is open answers it no.
    daemon.request(Request::ExtensionEvent {
        session,
        handler: handler(0),
        args_json: "[]".into(),
    });
    wait_for_alert(&daemon, session, "Delete it?");
    assert_eq!(
        daemon.request(Request::ExtensionPop { session }),
        Response::Ack
    );
    let answered = support.join("answered.txt");
    wait_for_content(&answered);
    assert_eq!(
        std::fs::read_to_string(&answered).ok().as_deref(),
        Some("false"),
        "navigating away is a way out of the dialog, and it is not the confirm button"
    );
    daemon.request(Request::CloseExtension { session });
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
    wait_for_content(&greeted);
    assert_eq!(
        std::fs::read_to_string(&greeted).expect("the command ran"),
        "hi Ada"
    );
}

#[test]
fn a_confined_command_opens_a_link_in_the_application_that_claims_its_scheme() {
    use compass_ipc::{Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // The opener is spawned by the engine, not the extension, so it may write
    // where the sandbox would never let the extension.
    let mut opened = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        opened = dir.join("opened.txt");
        let script = dir.join("browser.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > {:?}\n",
                opened.to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("browser.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Browser\nExec={} %u\n\
                 MimeType=x-scheme-handler/https;\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:link".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&opened);
    assert_eq!(
        std::fs::read_to_string(&opened).expect("the link was opened"),
        "https://example.com/a b"
    );
}

#[test]
fn a_grid_command_renders_typed_cells() {
    use compass_extension_api::View;
    use compass_extension_api::view::{Color, GridContent};
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
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:tiles".into(),
        arguments_json: None,
    }) else {
        panic!("the grid command did not start a view");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Grid(grid) if !grid.sections.is_empty()),
    );
    let View::Grid(grid) = view else {
        unreachable!()
    };
    assert_eq!(grid.columns, Some(4));
    let section = &grid.sections[0];
    assert_eq!(section.title.as_deref(), Some("Weather"));
    let titles: Vec<&str> = section.items.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["sun", "red"]);
    assert!(section.items[0].actions.is_some());
    assert_eq!(
        section.items[1].content,
        GridContent::Color(Color::Literal("#ff0000".into()))
    );
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn a_form_command_takes_edits_and_its_submit_gets_the_values() {
    use compass_extension_api::View;
    use compass_extension_api::view::{FieldValue, FormItem};
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut submitted = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        submitted = dir.join("data-home/vicinae/support/hello/submitted.json");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:issue".into(),
        arguments_json: None,
    }) else {
        panic!("the form command did not start a view");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Form(form) if !form.items.is_empty()),
    );
    let View::Form(form) = view else {
        unreachable!()
    };
    let FormItem::Field(title) = &form.items[0] else {
        panic!("no title field");
    };
    let on_change = title.on_change.clone().expect("a controlled field");
    let submit = form.actions.as_ref().expect("actions").actions()[0]
        .handler
        .clone();

    // The person types; the extension's state takes it and echoes it back.
    let answer = daemon.request(Request::ExtensionEvent {
        session,
        handler: on_change.0,
        args_json: r#"["Crash on paste", 1]"#.into(),
    });
    assert_eq!(answer, Response::Ack);
    let (echoed, _) = wait_for_view(&daemon, session, |view, _| match view {
        View::Form(form) => matches!(&form.items[0],
            FormItem::Field(f) if f.value == Some(FieldValue::Text("Crash on paste".into()))),
        _ => false,
    });
    let View::Form(echoed) = echoed else {
        unreachable!()
    };
    let FormItem::Field(title) = &echoed.items[0] else {
        unreachable!()
    };
    assert_eq!(
        title.echo.map(|seq| seq.raw()),
        Some(1),
        "an echo of edit 1"
    );

    daemon.request(Request::ExtensionEvent {
        session,
        handler: submit.0,
        args_json: r#"[{"title": "Crash on paste", "urgent": true}]"#.into(),
    });
    wait_for_content(&submitted);
    let got: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&submitted).expect("submitted")).unwrap();
    assert_eq!(
        got,
        serde_json::json!({
            "values": {"title": "Crash on paste", "urgent": true},
            "title": "Crash on paste"
        })
    );
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn an_extension_that_allocates_past_the_heap_cap_is_stopped() {
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut heap = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        heap = dir.join("data-home/vicinae/support/hello/heap");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:heap".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    let (started, survived) = (
        heap.with_extension("started"),
        heap.with_extension("survived"),
    );
    wait_for_content(&started);
    assert!(started.exists(), "the command never ran");
    // 400 MiB takes well under a second to allocate; give it five.
    std::thread::sleep(Duration::from_secs(5));
    assert!(
        !survived.exists(),
        "an extension allocated 400 MiB of heap under a 160 MiB cap"
    );
}

/// Suite 1: what a command sees of its environment, and what the sandbox
/// lets it run.
#[test]
fn a_command_sees_its_own_paths_and_preferences_and_may_exec_but_not_unshare() {
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
        root = dir.to_owned();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let probed = root.join("data-home/vicinae/support/hello/probe.json");
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:probe".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&probed);
    let seen: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&probed).expect("the command ran")).unwrap();
    let data = root.join("data-home/vicinae");
    assert_eq!(
        seen["supportPath"],
        data.join("support/hello").to_string_lossy().as_ref()
    );
    assert_eq!(
        seen["assetsPath"],
        data.join("extensions/hello/assets")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(seen["isDevelopment"], false);
    assert_eq!(
        (&seen["extensionName"], &seen["commandName"]),
        (&"hello".into(), &"probe".into())
    );
    assert_eq!(
        seen["preferences"],
        serde_json::json!({"limit": "20"}),
        "a preference's default is resolved"
    );
    // Shelling out is allowed on purpose: `useExec` and every extension that
    // wraps a CLI depend on it.
    assert_eq!(seen["execTrue"], 0);
    // `unshare` is on the denylist. The control: where this machine lets an
    // unconfined process make a user namespace, the sandbox must not.
    let outside = std::process::Command::new("unshare")
        .args(["-U", "true"])
        .status()
        .is_ok_and(|status| status.success());
    if outside {
        assert_ne!(
            seen["execUnshare"], 0,
            "unshare -U works outside the sandbox and must not inside it"
        );
    } else {
        eprintln!("unshare -U fails here even unconfined; the denial is not tested");
    }
    // Suite 1's `fork` case (§8.2): running what it wrote itself is the
    // escape, and it fails closed, while `true` from the system ran above.
    assert_eq!(
        seen["execDropped"], "EACCES",
        "a program the command wrote into its support directory ran"
    );
    // The 512 MB case: a Buffer lives outside the heap cap, and the data
    // limit refuses it as an error the command can catch rather than a crash.
    assert_eq!(
        seen["bigBuffer"], "RangeError",
        "a 512 MiB Buffer was allocated inside the sandbox"
    );
}

#[test]
fn a_confined_command_runs_a_command_in_the_terminal() {
    use compass_ipc::{Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut launched = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        launched = dir.join("terminal-argv.txt");
        let script = dir.join("term.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" > {:?}\n",
                launched.to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("term.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Term\nExec={}\n\
                 Categories=System;TerminalEmulator;\n\
                 X-TerminalArgExec=--\nX-TerminalArgTitle=--title=\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:term".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&launched);
    assert_eq!(
        std::fs::read_to_string(&launched).expect("the terminal was launched"),
        "--title=Top|--|htop|-d|5|"
    );
}

#[test]
fn a_views_toast_reaches_the_launcher_and_hiding_it_clears_it() {
    use compass_extension_api::View;
    use compass_ipc::{ExtensionToastStyle, Request, Response};
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
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:toaster".into(),
        arguments_json: None,
    }) else {
        panic!("the view did not start");
    };
    // Follow the raw answers: the toast is not part of the view.
    let mut after = 0;
    let mut hide = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    let toast = loop {
        assert!(Instant::now() < deadline, "no toast arrived");
        let Response::ExtensionView {
            version,
            view_json,
            toast,
            ..
        } = daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        after = version;
        if let Some(Ok(View::List(list))) =
            view_json.map(|json| serde_json::from_str::<View>(&json))
        {
            hide = list.sections[0].items[0]
                .actions
                .as_ref()
                .map(|panel| panel.actions()[0].handler.clone());
        }
        if let (Some(toast), Some(_)) = (toast, &hide) {
            break toast;
        }
    };
    assert_eq!(
        (toast.title.as_str(), toast.message.as_str(), toast.style),
        ("Offline", "retrying", ExtensionToastStyle::Failure)
    );

    let hide = hide.expect("the list's action");
    daemon.request(Request::ExtensionEvent {
        session,
        handler: hide.0,
        args_json: "[]".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "the toast was never hidden");
        let Response::ExtensionView { version, toast, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        after = version;
        if toast.is_none() {
            break;
        }
    }
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn a_power_command_answers_with_its_own_sentences_and_never_touches_this_machine() {
    use compass_ipc::{ErrorKind, Request, Response};
    // A system bus that does not exist: whatever this engine tries, the
    // machine running the test cannot be rebooted by it.
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![(
            "DBUS_SYSTEM_BUS_ADDRESS",
            "unix:path=/nonexistent/compass-test-system-bus".into(),
        )]
    });
    let Response::Error(err) = daemon.request(Request::RunPowerCommand {
        id: "reboot".into(),
    }) else {
        panic!("a reboot with no logind was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to reboot")
    );
    let Response::Error(err) = daemon.request(Request::RunPowerCommand {
        id: "self-destruct".into(),
    }) else {
        panic!("an unknown power command was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn a_media_command_says_why_it_did_nothing() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::io::{BufRead, BufReader};
    let play_pause = || Request::RunMediaCommand {
        id: "play-pause".into(),
    };

    // No session bus at all: the player could not be reached.
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let Response::Error(err) = daemon.request(play_pause()) else {
        panic!("play/pause with no session bus was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to toggle playback")
    );
    let Response::Error(err) = daemon.request(Request::RunMediaCommand {
        id: "rewind-time".into(),
    }) else {
        panic!("an unknown media command was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    drop(daemon);

    // A bus with no player on it: the C++'s own sentence.
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED the empty-bus half: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });
    let answer = daemon.request(play_pause());
    let _ = bus.kill();
    let _ = bus.wait();
    let Response::Error(err) = answer else {
        panic!("play/pause with no player was not refused: {answer:?}");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Unsupported, "No media player is running")
    );
}

#[test]
fn a_volume_command_runs_pactl_with_the_cpp_arguments() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let bin = TempDir::new().expect("tempdir");
    let log = bin.path().join("pactl.log");
    let fake = bin.path().join("pactl");
    std::fs::write(
        &fake,
        format!(
            r#"#!/bin/sh
echo "$*" >> '{log}'
[ -e '{fail}' ] && exit 1
case "$*" in
  get-default-sink) echo sink0 ;;
  "--format=json list sinks") echo '[{{"name":"sink0","mute":false,"volume":{{"mono":{{"value_percent":"45%"}}}}}}]' ;;
esac
exit 0
"#,
            log = log.display(),
            fail = bin.path().join("fail").display(),
        ),
    )
    .expect("fake pactl");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH");
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("PATH", path)]
    });
    let run = |id: &str| daemon.request(Request::RunMediaCommand { id: id.to_owned() });

    assert!(matches!(run("volume-50"), Response::Ack));
    assert!(matches!(run("volume-up"), Response::Ack));
    assert!(matches!(run("toggle-mute"), Response::Ack));
    let calls = std::fs::read_to_string(&log).expect("pactl ran");
    let calls: Vec<&str> = calls.lines().collect();
    assert_eq!(
        calls,
        [
            "set-sink-volume @DEFAULT_SINK@ 50%",
            "set-sink-volume @DEFAULT_SINK@ +5%",
            "get-default-sink",
            "--format=json list sinks",
            "set-sink-mute @DEFAULT_SINK@ toggle",
            "get-default-sink",
            "--format=json list sinks",
            "get-default-sink",
            "--format=json list sinks",
        ]
    );

    std::fs::write(bin.path().join("fail"), "").expect("make pactl fail");
    let Response::Error(err) = run("volume-0") else {
        panic!("a failing pactl was not reported");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to set volume")
    );

    // The step argument: a number moves by that much, anything else is
    // refused before pactl runs.
    std::fs::remove_file(bin.path().join("fail")).expect("let pactl succeed");
    std::fs::write(&log, "").expect("clear the log");
    let with = |id: &str, argument: &str| {
        daemon.request(Request::RunMediaCommandWith {
            id: id.to_owned(),
            argument: Some(argument.to_owned()),
        })
    };
    assert!(matches!(with("volume-down", "-12"), Response::Ack));
    let Response::Error(err) = with("volume-up", "five") else {
        panic!("a step that is not a number was not refused");
    };
    assert_eq!(err.message, "Invalid step value");
    let calls = std::fs::read_to_string(&log).expect("pactl ran");
    assert_eq!(
        calls.lines().next(),
        Some("set-sink-volume @DEFAULT_SINK@ -12%")
    );
}

/// A mock MPRIS player for the engine's media tests.
struct FakePlayer {
    status: &'static str,
    title: &'static str,
    can_go_next: bool,
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl FakePlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status.to_owned()
    }

    #[zbus(property)]
    fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "xesam:title".to_owned(),
            zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from(self.title))
                .expect("a value"),
        );
        metadata
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(name = "PlayPause")]
    fn play_pause(&self) {
        self.calls.lock().expect("log").push("PlayPause".to_owned());
    }

    #[zbus(name = "Next")]
    fn next(&self) {
        self.calls.lock().expect("log").push("Next".to_owned());
    }

    #[zbus(name = "Previous")]
    fn previous(&self) {
        self.calls.lock().expect("log").push("Previous".to_owned());
    }
}

struct FakePlayerRoot {
    identity: &'static str,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl FakePlayerRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.to_owned()
    }
}

#[test]
fn a_player_argument_picks_the_player_and_now_playing_lists_and_drives_them() {
    use compass_ipc::{ErrorKind, MediaPlayerAction, Request, Response};
    use std::io::{BufRead, BufReader};
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();

    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the players");
    let spotify = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let firefox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _players: Vec<zbus::Connection> = runtime.block_on(async {
        let mut connections = Vec::new();
        for (name, identity, status, title, can_go_next, calls) in [
            (
                "org.mpris.MediaPlayer2.spotify",
                "Spotify",
                "Playing",
                "Blue Monday",
                true,
                &spotify,
            ),
            (
                "org.mpris.MediaPlayer2.firefox",
                "Firefox",
                "Paused",
                "A lecture",
                false,
                &firefox,
            ),
        ] {
            let connection = zbus::connection::Builder::address(address.as_str())
                .expect("the private bus")
                .name(name)
                .expect("a player name")
                .serve_at(
                    "/org/mpris/MediaPlayer2",
                    FakePlayer {
                        status,
                        title,
                        can_go_next,
                        calls: std::sync::Arc::clone(calls),
                    },
                )
                .expect("the player interface")
                .serve_at("/org/mpris/MediaPlayer2", FakePlayerRoot { identity })
                .expect("the root interface")
                .build()
                .await
                .expect("the player connects");
            connections.push(connection);
        }
        connections
    });

    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });
    let Response::MediaPlayers { mut players } = daemon.request(Request::ListMediaPlayers) else {
        panic!("the players were not listed");
    };
    players.sort_by(|a, b| a.identity.cmp(&b.identity));
    let summary: Vec<(&str, &str, bool, bool)> = players
        .iter()
        .map(|p| (p.identity.as_str(), p.title.as_str(), p.playing, p.paused))
        .collect();
    assert_eq!(
        summary,
        [
            ("Firefox", "A lecture", false, true),
            ("Spotify", "Blue Monday", true, false)
        ]
    );

    let with = |id: &str, argument: &str| {
        daemon.request(Request::RunMediaCommandWith {
            id: id.to_owned(),
            argument: Some(argument.to_owned()),
        })
    };
    assert!(matches!(with("play-pause", "lecture"), Response::Ack));
    let Response::Error(err) = with("next-track", "firefox") else {
        panic!("a skip the player cannot make was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (
            ErrorKind::Unsupported,
            "Firefox cannot skip to the next track"
        )
    );
    let Response::Error(err) = with("play-pause", "vlc") else {
        panic!("a player nothing matches was not refused");
    };
    assert_eq!(err.message, "No media player matches \"vlc\"");
    assert!(matches!(
        daemon.request(Request::ControlMediaPlayer {
            player: "org.mpris.MediaPlayer2.spotify".into(),
            action: MediaPlayerAction::Next,
        }),
        Response::Ack
    ));
    let Response::Error(err) = daemon.request(Request::ControlMediaPlayer {
        player: "com.example.nope".into(),
        action: MediaPlayerAction::Next,
    }) else {
        panic!("a name that is not a player's was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    drop(daemon);
    let _ = bus.kill();
    let _ = bus.wait();
    assert_eq!(firefox.lock().unwrap().as_slice(), ["PlayPause"]);
    assert_eq!(spotify.lock().unwrap().as_slice(), ["Next"]);
}

/// Asks Search Files until `found` accepts the answer, or panics with the
/// last one: the indexer scans in the background, so the first queries can
/// come before the file is in the index — or before the helper has started.
fn search_files_until(
    daemon: &Daemon,
    query: &str,
    category: Option<&str>,
    found: impl Fn(&str, &[compass_ipc::FileHit]) -> bool,
) -> (String, Vec<compass_ipc::FileHit>) {
    use compass_ipc::{Request, Response};
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut last = None;
    while Instant::now() < deadline {
        match daemon.request(Request::SearchFiles {
            query: query.into(),
            category: category.map(str::to_owned),
        }) {
            Response::Files { heading, files } => {
                if found(&heading, &files) {
                    return (heading, files);
                }
                last = Some(format!("{heading}: {files:?}"));
            }
            other => last = Some(format!("{other:?}")),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("Search Files never answered {query:?} as expected; last answer: {last:?}");
}

#[test]
fn search_files_indexes_the_home_directory_and_finds_a_file_by_a_misspelled_query() {
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let documents = root.join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join("quarterly-report.pdf"), "%PDF").unwrap();
        std::fs::write(documents.join("holiday.png"), "png").unwrap();
        Vec::new()
    });
    let is_report = |files: &[compass_ipc::FileHit]| {
        files
            .first()
            .is_some_and(|file| file.name == "quarterly-report.pdf")
    };

    let (heading, files) = search_files_until(&daemon, "quartely report", None, |_, files| {
        is_report(files)
    });
    assert_eq!(heading, "Results");
    assert!(files[0].path.ends_with("/Documents/quarterly-report.pdf"));
    assert_eq!(files[0].category, "Documents");

    // The category filter reaches the index: images only, so no report.
    let (_, images) = search_files_until(&daemon, "holiday", Some("Images"), |_, files| {
        files.iter().any(|file| file.name == "holiday.png")
    });
    assert!(images.iter().all(|file| file.category == "Images"));
    let (_, none) = search_files_until(&daemon, "quarterly report", Some("Images"), |_, _| true);
    assert!(
        none.iter().all(|file| file.name != "quarterly-report.pdf"),
        "{none:?}"
    );
}

#[test]
fn search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let notes = root.join("notes.md");
        std::fs::write(&notes, "# notes").unwrap();
        let data_home = root.join("data-home");
        std::fs::create_dir_all(&data_home).unwrap();
        std::fs::write(
            data_home.join("recently-used.xbel"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file://{}" added="2026-01-01T00:00:00Z" modified="2026-01-02T00:00:00Z" visited="2026-01-01T00:00:00Z"/>
  <bookmark href="file:///nonexistent/gone.txt" added="2026-01-01T00:00:00Z" modified="2026-01-03T00:00:00Z" visited="2026-01-01T00:00:00Z"/>
</xbel>
"#,
                notes.display()
            ),
        )
        .unwrap();
        Vec::new()
    });

    let Response::Files { heading, files } = daemon.request(Request::SearchFiles {
        query: String::new(),
        category: None,
    }) else {
        panic!("the empty query was not answered with files");
    };
    assert_eq!(heading, "Recently Accessed");
    assert_eq!(
        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["notes.md"],
        "the missing file is skipped"
    );

    let Response::Files { heading, files } = daemon.request(Request::SearchFiles {
        query: "~/notes.md".into(),
        category: None,
    }) else {
        panic!("a typed path was not answered with files");
    };
    assert_eq!(heading, "Direct file path");
    assert_eq!(files.len(), 1);
    assert!(files[0].path.ends_with("/notes.md"), "{files:?}");

    // Opening refuses a path that is not there, and a file no installed
    // application opens -- the fixture tree has none that opens Markdown --
    // rather than launching anything on the machine running the tests.
    let Response::Error(err) = daemon.request(Request::OpenFile {
        path: "/nonexistent/gone.txt".into(),
        reveal: false,
    }) else {
        panic!("opening a missing file was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    let Response::Error(err) = daemon.request(Request::OpenFile {
        path: files[0].path.clone(),
        reveal: false,
    }) else {
        panic!("opening a file nothing opens was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

#[test]
fn search_files_without_indexing_says_the_index_is_unavailable() {
    use compass_ipc::{ErrorKind, Request, Response};
    let config = r#"{"providers": {"files": {"preferences": {"autoIndexing": false}}}}"#;
    let daemon = Daemon::start_with_config(&[("a.desktop", &entry("Alpha", ""))], config);
    let Response::Error(err) = daemon.request(Request::SearchFiles {
        query: "report".into(),
        category: None,
    }) else {
        panic!("an index search with indexing off was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(err.message.contains("file indexer"), "{}", err.message);
}

#[test]
fn an_oauth_authorization_opens_the_browser_and_the_redirect_answers_it() {
    use compass_extension_api::View;
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // The browser is a script that writes down the URL it was given, which
    // is how the test learns the state the extension generated.
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        let script = dir.join("browser.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > {:?}\n",
                dir.join("opened.txt").to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("browser.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Browser\nExec={} %u\n\
                 MimeType=x-scheme-handler/https;\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:auth".into(),
        arguments_json: None,
    }) else {
        panic!("no session");
    };
    wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));

    let opened = root.join("opened.txt");
    wait_for_content(&opened);
    let url = std::fs::read_to_string(&opened).expect("the browser was opened");
    assert!(
        url.starts_with("https://example.com/authorize?"),
        "the extension's own authorization URL, as built: {url}"
    );
    let state = compass_worker_host::oauth_service::query_value(&url, "state").expect("a state");

    // A redirect for some other request is refused, and does not settle this one.
    let Response::Error(err) = daemon.request(Request::OAuthRedirect {
        url: "raycast://oauth?code=nope&state=someone-else".into(),
    }) else {
        panic!("a redirect nobody waits on was accepted");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    let redirect = format!(
        "raycast://oauth?package_name=Extension&code=the-code&state={}",
        percent_encode(&state)
    );
    assert_eq!(
        daemon.request(Request::OAuthRedirect { url: redirect }),
        Response::Ack
    );
    let authorized = root.join("data-home/vicinae/support/hello/authorized.txt");
    wait_for_content(&authorized);
    assert_eq!(
        std::fs::read_to_string(&authorized).ok().as_deref(),
        Some("the-code"),
        "the extension's authorize() resolved with the code from the redirect"
    );
    daemon.request(Request::CloseExtension { session });
}

/// Percent-encodes everything but the unreserved characters.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A fake application in its own directory that appends each argument it is
/// launched with to `<dir>/<name>.log`, and the desktop entry that runs it
/// for `mime_types`.
fn recording_app(dir: &Path, name: &str, mime_types: &str) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join(format!("{name}.log"));
    let program = dir.join(name);
    std::fs::write(
        &program,
        format!(
            // All at once, then renamed into place: a reader polling for a
            // non-empty log must not see the first argument without the rest.
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}.part' && mv '{log}.part' '{log}'\n",
            log = log.display()
        ),
    )
    .expect("fake application");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={} %u\nMimeType={mime_types}\n",
        program.display()
    );
    (entry, log)
}

/// Reads `log` until it has a line, or panics: a launch is spawned, so the
/// answer can arrive before the program has run.
fn launched_with(log: &Path) -> Vec<String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(log)
            && !text.is_empty()
        {
            return text.lines().map(str::to_owned).collect();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{} was never launched", log.display());
}

#[test]
fn shortcuts_are_imported_created_searched_opened_edited_and_removed() {
    use compass_ipc::{ErrorKind, Request, Response, ShortcutEntry};
    let bin = TempDir::new().expect("tempdir");
    let (browser, log) = recording_app(bin.path(), "browser", "x-scheme-handler/https;");
    let vicinae_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("browser.desktop", &browser)], "{}", |root| {
        let file = root.join("data-home/vicinae/shortcuts/shortcuts.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[{"id":"sct-aaaaaaaaaaaa","name":"Crate Docs","icon":"icon://omnicast/link",
                "url":"https://docs.rs/{crate}","app":"default","openCount":4,
                "createdAt":1700000000,"updatedAt":1700000000}]"#,
        )
        .unwrap();
        vicinae_file.set(file).unwrap();
        Vec::new()
    });
    let list = |response: Response| -> Vec<ShortcutEntry> {
        match response {
            Response::Shortcuts { shortcuts } => shortcuts,
            other => panic!("not a shortcut list: {other:?}"),
        }
    };

    let imported = list(daemon.request(Request::ListShortcuts));
    assert_eq!(imported.len(), 1, "Vicinae's shortcut came across");
    assert_eq!(imported[0].name, "Crate Docs");
    assert_eq!(imported[0].open_count, 4);

    let saved = list(daemon.request(Request::SaveShortcut {
        id: None,
        name: "Search Rust".into(),
        icon: "default".into(),
        url: "https://docs.rs/releases/search?query={query}".into(),
        app: "default".into(),
    }));
    assert_eq!(saved.len(), 2);
    let created = saved.iter().find(|s| s.name == "Search Rust").unwrap();
    assert!(created.id.starts_with("sct-"), "{}", created.id);
    assert_eq!(
        created.icon, "icon://favicon/docs.rs?fallback=icon://omnicast/image",
        "the default icon is resolved to the site's favicon when saved"
    );

    // Root search ranks it with everything else, by its name.
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "search rust".into(),
    }) else {
        panic!("no query results");
    };
    assert_eq!(
        hits.first().map(|hit| hit.id.as_str()),
        Some(format!("shortcuts:{}", created.id).as_str()),
        "{hits:?}"
    );

    let Response::Text { text } = daemon.request(Request::ExpandShortcut {
        id: created.id.clone(),
        arguments: vec!["serde json".into()],
    }) else {
        panic!("not expanded");
    };
    assert_eq!(text, "https://docs.rs/releases/search?query=serde json");

    assert_eq!(
        daemon.request(Request::OpenShortcut {
            id: created.id.clone(),
            arguments: vec!["serde".into()],
        }),
        Response::Ack
    );
    assert_eq!(
        launched_with(&log),
        ["https://docs.rs/releases/search?query=serde"],
        "the browser, which claims https, opened the expanded link"
    );
    let after = list(daemon.request(Request::ListShortcuts));
    let opened = after.iter().find(|s| s.id == created.id).unwrap();
    assert_eq!(opened.open_count, 1);
    assert!(opened.last_used_at.is_some());

    let edited = list(daemon.request(Request::SaveShortcut {
        id: Some(created.id.clone()),
        name: "Rust Search".into(),
        icon: "icon://omnicast/bolt".into(),
        url: created.url.clone(),
        app: "default".into(),
    }));
    let renamed = edited.iter().find(|s| s.id == created.id).unwrap();
    assert_eq!(renamed.name, "Rust Search");
    assert_eq!(renamed.icon, "icon://omnicast/bolt");
    assert_eq!(renamed.open_count, 1, "editing is not opening");

    let remaining = list(daemon.request(Request::RemoveShortcut {
        id: "sct-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(
        remaining.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [created.id.as_str()]
    );
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "crate docs".into(),
    }) else {
        panic!("no query results");
    };
    assert!(
        hits.iter()
            .all(|hit| hit.id != "shortcuts:sct-aaaaaaaaaaaa"),
        "a removed shortcut leaves root search: {hits:?}"
    );

    // Compass wrote its own file; Vicinae's is as it was.
    let vicinae = std::fs::read_to_string(vicinae_file.get().unwrap()).unwrap();
    assert!(vicinae.contains("Crate Docs"));
    let compass = std::fs::read_to_string(
        vicinae_file
            .get()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("compass-shortcuts.json"),
    )
    .unwrap();
    assert!(compass.contains("Rust Search") && !compass.contains("Crate Docs"));

    let Response::Error(err) = daemon.request(Request::OpenShortcut {
        id: "sct-gone".into(),
        arguments: vec![],
    }) else {
        panic!("opening a missing shortcut was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    let Response::Error(err) = daemon.request(Request::SaveShortcut {
        id: None,
        name: "No link".into(),
        icon: "default".into(),
        url: String::new(),
        app: "default".into(),
    }) else {
        panic!("a shortcut with no link was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn snippets_are_imported_created_expanded_edited_and_removed() {
    use compass_ipc::{ErrorKind, Request, Response, SnippetEntry};
    let vicinae_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let file = root.join("data-home/vicinae/snippets/snippets.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[{"id":"snp-aaaaaaaaaaaa","name":"Signature","data":{"text":"Best,\nMe"},
                "createdAt":1700000000,"expansion":{"keyword":";sig","apps":[],"word":true}}]"#,
        )
        .unwrap();
        vicinae_file.set(file).unwrap();
        Vec::new()
    });
    let list = |response: Response| -> Vec<SnippetEntry> {
        match response {
            Response::Snippets { snippets } => snippets,
            other => panic!("not a snippet list: {other:?}"),
        }
    };
    let refused = |response: Response| -> (ErrorKind, String) {
        match response {
            Response::Error(err) => (err.kind, err.message),
            other => panic!("not refused: {other:?}"),
        }
    };

    let imported = list(daemon.request(Request::ListSnippets));
    assert_eq!(imported.len(), 1, "Vicinae's snippet came across");
    assert_eq!(imported[0].keyword.as_deref(), Some(";sig"));
    assert_eq!(imported[0].text.as_deref(), Some("Best,\nMe"));

    let saved =
        list(
            daemon.request(Request::SaveSnippet {
                id: None,
                name: "Greeting".into(),
                text:
                    "Hello {name}, {date format=\"yyyy\"} {shell code=\"echo shell-ran\"}{cursor}"
                        .into(),
                keyword: Some(";hi".into()),
                word: false,
                apps: vec![],
            }),
        );
    assert_eq!(saved.len(), 2);
    let greeting = saved.iter().find(|s| s.name == "Greeting").unwrap().clone();
    assert!(greeting.id.starts_with("snp-"));
    assert!(!greeting.word);

    let (kind, message) = refused(daemon.request(Request::SaveSnippet {
        id: None,
        name: "Other".into(),
        text: "x".into(),
        keyword: Some(";sig".into()),
        word: true,
        apps: vec![],
    }));
    assert_eq!(kind, ErrorKind::BadRequest);
    assert_eq!(message, "keyword already assigned to \"Signature\"");
    let (kind, message) = refused(daemon.request(Request::SaveSnippet {
        id: None,
        name: "x".into(),
        text: "{cursor}{cursor}".into(),
        keyword: Some("has space".into()),
        word: true,
        apps: vec![],
    }));
    assert_eq!(kind, ErrorKind::BadRequest);
    assert!(
        message.contains("2 chars min.")
            && message.contains("Only one {cursor}")
            && message.contains("printable ASCII"),
        "{message}"
    );

    let Response::Text { text } = daemon.request(Request::ExpandSnippet {
        id: greeting.id.clone(),
        arguments: vec![("name".into(), "Zoë".into())],
    }) else {
        panic!("not expanded");
    };
    let year: i32 = text
        .split(", ")
        .nth(1)
        .and_then(|rest| rest.get(..4))
        .and_then(|year| year.parse().ok())
        .unwrap_or_else(|| panic!("no year in {text:?}"));
    assert!(year >= 2024, "{text}");
    assert_eq!(text, format!("Hello Zoë, {year} shell-ran"));

    // Pasting needs the Shell extension, and this engine has no session bus.
    let (kind, _) = refused(daemon.request(Request::PasteSnippet {
        id: greeting.id.clone(),
        arguments: vec![],
    }));
    assert_eq!(kind, ErrorKind::Unsupported);

    let edited = list(daemon.request(Request::SaveSnippet {
        id: Some(greeting.id.clone()),
        name: "Hello".into(),
        text: "Hi".into(),
        keyword: Some(";hi".into()),
        word: true,
        apps: vec![],
    }));
    let hello = edited.iter().find(|s| s.id == greeting.id).unwrap();
    assert_eq!(hello.name, "Hello");
    assert!(hello.updated_at.is_some());
    assert_eq!(hello.created_at, greeting.created_at);

    let remaining = list(daemon.request(Request::RemoveSnippet {
        id: "snp-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(remaining.len(), 1);
    let (kind, _) = refused(daemon.request(Request::RemoveSnippet {
        id: "snp-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(kind, ErrorKind::BadRequest);

    let vicinae = std::fs::read_to_string(vicinae_file.get().unwrap()).unwrap();
    assert!(vicinae.contains("Signature"), "Vicinae's file is untouched");
    let compass = std::fs::read_to_string(
        vicinae_file
            .get()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("compass-snippets.json"),
    )
    .unwrap();
    assert!(compass.contains("\"name\":\"Hello\"") && !compass.contains("Signature"));
}

/// Asks for a script run's output until it has finished, or panics.
fn script_output_until_finished(daemon: &Daemon, session: u64) -> (String, Option<i32>) {
    use compass_ipc::{Request, Response};
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        match daemon.request(Request::ScriptOutput { session }) {
            Response::ScriptOutput {
                output,
                finished: true,
                exit_code,
                ..
            } => return (output, exit_code),
            Response::ScriptOutput { .. } => {}
            other => panic!("not a script output: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("script run {session} never finished");
}

#[test]
fn script_commands_are_scanned_searched_and_run_in_their_modes() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let marker = std::sync::OnceLock::new();
    let custom = TempDir::new().expect("tempdir");
    let write = |dir: &Path, name: &str, mode: &str, title: &str, body: &str| {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title {title}\n\
                 # @raycast.mode {mode}\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write(
        custom.path(),
        "compact.sh",
        "compact",
        "Custom Compact",
        "echo custom; exit 1",
    );
    let config = format!(
        r#"{{"providers": {{"scripts": {{"preferences": {{"customDirs": ["{}"]}}}}}}}}"#,
        custom.path().display()
    );
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], &config, |root| {
        let dir = root.join("data-home/vicinae/scripts");
        write(
            &dir,
            "full.sh",
            "fullOutput",
            "Full Report",
            "# @raycast.argument1 {\"type\":\"text\",\"placeholder\":\"who\"}\n\
             printf '\\033[32mgreen\\033[0m %s https://x.test\\n' \"$1\"; echo err >&2",
        );
        write(&dir, "inline.sh", "inline", "Queue Size", "echo '42 items'");
        write(
            &dir,
            "compact.sh",
            "compact",
            "Packaged Compact",
            "echo packaged",
        );
        let flag = root.join("silent-ran");
        write(
            &dir.join("tools"),
            "silent.sh",
            "silent",
            "Touch Marker",
            &format!("touch '{}'", flag.display()),
        );
        marker.set(flag).unwrap();
        Vec::new()
    });

    let Response::Scripts { scripts } = daemon.request(Request::ListScripts) else {
        panic!("no script list");
    };
    let mut listed: Vec<(&str, &str, &str)> = scripts
        .iter()
        .map(|s| (s.id.as_str(), s.title.as_str(), s.mode.as_str()))
        .collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            ("compact.sh", "Custom Compact", "compact"),
            ("full.sh", "Full Report", "fullOutput"),
            ("inline.sh", "Queue Size", "inline"),
            ("tools.silent.sh", "Touch Marker", "silent"),
        ],
        "a custom directory's script shadows the packaged one with the same id"
    );
    let full = scripts.iter().find(|s| s.id == "full.sh").unwrap();
    assert_eq!(full.arguments.len(), 1);
    assert_eq!(full.arguments[0].placeholder.as_deref(), Some("who"));

    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "full report".into(),
    }) else {
        panic!("no query results");
    };
    assert_eq!(hits.first().map(|h| h.id.as_str()), Some("scripts:full.sh"));

    let started = |id: &str, arguments: Vec<String>| match daemon.request(Request::RunScript {
        id: id.into(),
        arguments,
    }) {
        Response::ScriptStarted { session } => session,
        other => panic!("{id} did not start: {other:?}"),
    };

    let session = started("full.sh", vec!["Zoë".into()]).expect("full output is followed");
    let (output, exit) = script_output_until_finished(&daemon, session);
    assert_eq!(exit, Some(0));
    assert!(
        output.contains("\u{1b}[32mgreen\u{1b}[0m Zoë https://x.test"),
        "{output:?}"
    );
    assert!(output.contains("err"), "stderr is part of full output");

    let session = started("inline.sh", vec![]).expect("inline is followed");
    let (output, _) = script_output_until_finished(&daemon, session);
    assert_eq!(output.trim(), "42 items");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let Response::Scripts { scripts } = daemon.request(Request::ListScripts) else {
            panic!("no script list");
        };
        let inline = scripts.iter().find(|s| s.id == "inline.sh").unwrap();
        if inline.subtitle == "42 items" {
            break;
        }
        assert_eq!(inline.subtitle, "No data");
        assert!(
            Instant::now() < deadline,
            "the inline line never became the subtitle"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let session = started("compact.sh", vec![]).expect("compact is followed");
    assert_eq!(
        script_output_until_finished(&daemon, session),
        ("custom\n".to_owned(), Some(1))
    );

    assert_eq!(started("tools.silent.sh", vec![]), None);
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while !marker.get().unwrap().exists() {
        assert!(Instant::now() < deadline, "the silent script never ran");
        std::thread::sleep(Duration::from_millis(25));
    }

    let Response::Error(err) = daemon.request(Request::RunScript {
        id: "nothing.sh".into(),
        arguments: vec![],
    }) else {
        panic!("an unknown script was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn run_terminal_program_lists_path_and_runs_directly_or_refuses() {
    use compass_ipc::{ErrorKind, Request, Response};
    let bin = TempDir::new().expect("tempdir");
    let (_, log) = recording_app(bin.path(), "fake-tool", "");
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH");
    let config = r#"{"providers": {"commands": {"entrypoints": {"run-program":
        {"preferences": {"default-action": "run"}}}}}}"#;
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], config, |_| {
        vec![("PATH", path)]
    });

    let Response::Programs {
        programs,
        terminal,
        default_action,
    } = daemon.request(Request::ListPrograms)
    else {
        panic!("no program list");
    };
    let tool = bin.path().join("fake-tool").to_string_lossy().into_owned();
    assert!(programs.contains(&tool), "{programs:?}");
    assert_eq!(terminal, None, "the fixture installs no terminal");
    assert_eq!(default_action, "run", "read from the command's preferences");

    assert_eq!(
        daemon.request(Request::RunProgram {
            argv: vec!["fake-tool".into(), "--flag".into(), "two words".into()],
            terminal: false,
            hold: false,
        }),
        Response::Ack
    );
    assert_eq!(launched_with(&log), ["--flag", "two words"]);

    let Response::Error(err) = daemon.request(Request::RunProgram {
        argv: vec!["no-such-tool-anywhere".into()],
        terminal: false,
        hold: false,
    }) else {
        panic!("a missing program was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::BadRequest, "Not a valid executable")
    );
    let Response::Error(err) = daemon.request(Request::RunProgram {
        argv: vec!["fake-tool".into()],
        terminal: true,
        hold: true,
    }) else {
        panic!("a terminal run with no terminal was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

#[test]
fn create_extension_writes_the_boilerplate_under_home() {
    use compass_ipc::{ErrorKind, Request, Response};
    let home = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        std::fs::create_dir_all(root.join("code")).unwrap();
        home.set(root.to_path_buf()).unwrap();
        Vec::new()
    });
    let request = |location: &str, author: &str| Request::CreateExtension {
        author: author.into(),
        title: "Hello World".into(),
        description: "Says hello to the whole world".into(),
        location: location.into(),
        command_title: "Say Hello".into(),
        command_description: "Says hello".into(),
        template: ":boilerplate/tmpl-no-view".into(),
    };
    let Response::ExtensionCreated { path } = daemon.request(request("~/code", "zoe")) else {
        panic!("not created");
    };
    let root = std::path::Path::new(&path);
    assert!(root.starts_with(home.get().unwrap().join("code")), "{path}");
    let manifest = std::fs::read_to_string(root.join("package.json")).unwrap();
    assert!(manifest.contains("\"mode\": \"no-view\""), "{manifest}");

    let Response::Error(err) = daemon.request(request("~/nowhere", "z")) else {
        panic!("an invalid form was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(
        err.message.contains("location: Must exist"),
        "{}",
        err.message
    );
}

#[test]
fn set_theme_keeps_the_theme_in_the_configuration() {
    use compass_ipc::{ErrorKind, Request, Response};
    let config_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        config_file
            .set(root.join("config/vicinae/vicinae.json"))
            .unwrap();
        Vec::new()
    });
    assert_eq!(
        daemon.request(Request::SetTheme {
            theme: "Tokyo-Night".into()
        }),
        Response::Ack
    );
    let saved = std::fs::read_to_string(config_file.get().unwrap()).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert!(
        saved.to_string().contains("\"tokyo-night\""),
        "the persisted spelling is written: {saved}"
    );
    let Response::Error(err) = daemon.request(Request::SetTheme {
        theme: "no-such-theme".into(),
    }) else {
        panic!("an unknown theme was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn browse_fonts_lists_families_and_previews_one() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let Response::Fonts { fonts, categories } = daemon.request(Request::ListFonts) else {
        panic!("no font list");
    };
    assert!(categories.iter().any(|category| category == "Latin"));
    for font in &fonts {
        assert!(
            font.categories.contains(&font.primary),
            "{font:?} is listed under a category it cannot be filtered by"
        );
    }
    if let Some(font) = fonts.first() {
        let Response::Text { text } = daemon.request(Request::FontSpecimen {
            name: font.name.clone(),
        }) else {
            panic!("no specimen for {}", font.name);
        };
        assert!(!text.is_empty());
    }
    let Response::Error(err) = daemon.request(Request::FontSpecimen {
        name: "No Such Family 123".into(),
    }) else {
        panic!("an unknown family was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

/// Runs `vicinae dmenu` against `daemon` with `stdin`, returning its output.
fn run_dmenu(daemon: &Daemon, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("dmenu")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run vicinae dmenu");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("vicinae dmenu finished")
}

#[test]
fn dmenu_shows_stdin_in_the_attached_window_and_prints_the_choice() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    // No window yet: refused, and nothing printed.
    let refused = run_dmenu(&daemon, &[], "a\nb\n");
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());

    // A fake window that picks the second entry for an index list, and
    // dismisses anything else.
    let socket = daemon.socket.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<compass_ipc::DmenuSpec>::new()));
    let window = {
        let seen = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(async move {
                let mut window = compass_ipc::WindowClient::attach(&socket)
                    .await
                    .expect("attach");
                ready_tx.send(()).expect("ready");
                for _ in 0..2 {
                    let Some(compass_ipc::WindowCommand::Dmenu(token)) =
                        window.next_command().await.expect("a command")
                    else {
                        panic!("expected a dmenu command");
                    };
                    window
                        .reply(compass_ipc::WindowOutcome::Shown)
                        .await
                        .expect("reply");
                    let mut client = compass_ipc::Client::connect(&socket).await.expect("client");
                    let compass_ipc::Response::DmenuList { spec } = client
                        .request(compass_ipc::Request::DmenuFetch { token })
                        .await
                        .expect("fetch")
                    else {
                        panic!("no dmenu list");
                    };
                    let output = spec.output_index.then(|| "1".to_owned());
                    seen.lock().expect("lock").push(spec);
                    let answered = client
                        .request(compass_ipc::Request::DmenuChoose { token, output })
                        .await
                        .expect("choose");
                    assert_eq!(answered, compass_ipc::Response::Ack);
                }
            });
        })
    };
    ready_rx.recv_timeout(STARTUP_TIMEOUT).expect("attached");

    let chosen = run_dmenu(
        &daemon,
        &["--format", "index", "-p", "Pick one", "-W", "300"],
        "alpha\nbeta\n\ngamma\n",
    );
    assert!(
        chosen.status.success(),
        "{}",
        String::from_utf8_lossy(&chosen.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&chosen.stdout), "1\n");

    let dismissed = run_dmenu(&daemon, &[], "alpha\n");
    assert_eq!(
        dismissed.status.code(),
        Some(1),
        "dismissed: exit 1, as the C++"
    );
    assert!(dismissed.stdout.is_empty());

    window.join().expect("the fake window finished");
    let seen = seen.lock().expect("lock");
    assert_eq!(seen[0].content, "alpha\nbeta\n\ngamma\n");
    assert_eq!(seen[0].placeholder.as_deref(), Some("Pick one"));
    assert!(
        seen[0].no_quick_look && seen[0].no_footer,
        "a list narrower than 500 px drops quick look and the footer"
    );
    assert!(!seen[1].output_index);
}

// ---------------------------------------------------------------------------
// Rhai scripts. The in-process tests in `rhai_scripts.rs` cover actions,
// consent and effects against a fake clipboard; these hold the real process
// to the XDG layout: a packaged script under `$XDG_DATA_DIRS`, the user's
// directory created under `$XDG_DATA_HOME`, and hot reload.

#[test]
fn a_packaged_rhai_script_is_found_in_root_search_and_its_view_renders() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/rhai-examples");
    let mut root = PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        let target = dir.join("data/compass/scripts/web-search");
        std::fs::create_dir_all(&target).unwrap();
        for file in ["script.toml", "main.rhai"] {
            std::fs::copy(examples.join("web-search").join(file), target.join(file)).unwrap();
        }
        root = dir.to_path_buf();
        Vec::new()
    });
    let user_dir = root.join("data-home/compass/scripts");
    assert!(user_dir.is_dir(), "the engine creates the user's directory");

    let Response::RhaiScripts { scripts } = daemon.request(Request::ListRhaiScripts) else {
        panic!("no script list");
    };
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].title, "Web Search");
    assert_eq!(scripts[0].icon.as_deref(), Some("globe"));

    let json = daemon.client(&["query", "--json", "duckduckgo"]);
    assert!(json.contains("rhai:script.web-search"), "{json}");

    let started = daemon.request(Request::RunExtensionCommand {
        id: "rhai:script.web-search".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("the script did not open: {started:?}");
    };
    let (view, depth) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    assert_eq!(depth, 1);
    let View::List(list) = view else {
        unreachable!()
    };
    let titles: Vec<&str> = list.sections[0]
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(titles, ["DuckDuckGo", "Wikipedia", "GitHub", "crates.io"]);
    let handler = list.search.on_change.expect("the script takes the text");
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: handler.0,
            args_json: r#"["rust", 1]"#.into(),
        }),
        Response::Ack
    );
    let (view, _) = wait_for_view(&daemon, session, |view, _| {
        matches!(view, View::List(list)
            if list.sections[0].items[0].title.contains("rust"))
    });
    let View::List(list) = view else {
        unreachable!()
    };
    assert_eq!(
        list.sections[0].items[0].subtitle.as_deref(),
        Some("https://duckduckgo.com/?q=rust")
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );

    // A script saved into the user's directory while the engine runs is in
    // root search without asking for a rescan.
    let mine = user_dir.join("mine");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("main.rhai"), "fn search(q) { [] }").unwrap();
    std::fs::write(mine.join("script.toml"), "title = \"Zebra Notes\"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let json = daemon.client(&["query", "--json", "zebra notes"]);
        if json.contains("rhai:script.mine") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hot reload never listed it: {json}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- The extension stores, against a local fake ----

/// A local stand-in for `api.vicinae.com` (under `/v1`) and
/// `backend.raycast.com` (under `/raycast`), serving fixture listings and
/// bundles. The Raycast extension's commit can be moved on to publish an
/// update.
struct FakeStore {
    base: String,
    raycast_commit: std::sync::Arc<std::sync::Mutex<String>>,
    _server: std::sync::Arc<tiny_http::Server>,
}

fn bundle(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in entries {
        out.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("zip entry");
        out.write_all(content.as_bytes()).expect("zip write");
    }
    out.finish().expect("zip").into_inner()
}

impl FakeStore {
    fn start() -> FakeStore {
        let server =
            std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("fake store"));
        let port = server.server_addr().to_ip().expect("ip").port();
        let base = format!("http://127.0.0.1:{port}");
        let raycast_commit = std::sync::Arc::new(std::sync::Mutex::new("c1".to_owned()));

        let clock = bundle(&[
            (
                "clock/package.json",
                r#"{"name": "clock", "title": "Clock", "author": "zoe",
                    "commands": [{"name": "show-time", "title": "Show Time Now", "mode": "view"}]}"#,
            ),
            ("clock/show-time.js", "module.exports = {};"),
        ]);
        let hn = bundle(&[
            (
                "hn/package.json",
                r#"{"name": "hn", "title": "Hacker News", "author": "ray",
                    "dependencies": {"@raycast/api": "1.0.0"},
                    "commands": [{"name": "front", "title": "Front Page Stories", "mode": "view"}]}"#,
            ),
            ("hn/front.js", "module.exports = {};"),
        ]);
        let evil = bundle(&[
            ("evil/package.json", r#"{"name": "evil", "commands": []}"#),
            ("evil/../../escaped.txt", "gotcha"),
        ]);

        let vicinae_listing = serde_json::json!({
            "extensions": [
                {
                    "id": "x1", "name": "clock", "title": "Clock",
                    "description": "Shows the time",
                    "author": {"handle": "zoe", "name": "Zoë", "avatarUrl": "", "profileUrl": ""},
                    "downloadCount": 1001, "checksum": "v1",
                    "platforms": ["linux"], "categories": [{"id": "system", "name": "System"}],
                    "commands": [{"id": "c", "name": "show-time", "title": "Show Time Now",
                                  "subtitle": "", "description": "Says the time", "mode": "view"}],
                    "readmeUrl": format!("{base}/readme/clock.md"),
                    "downloadUrl": format!("{base}/dl/clock.zip"),
                    "updatedAt": "2026-07-02T11:50:22.441Z"
                },
                {
                    "id": "x2", "name": "evil", "title": "Evil", "description": "Climbs out",
                    "author": {"handle": "mallory", "name": "Mallory", "avatarUrl": "", "profileUrl": ""},
                    "downloadUrl": format!("{base}/dl/evil.zip")
                },
                {
                    "id": "x3", "name": "mac-only", "title": "Mac Only", "description": "Not here",
                    "platforms": ["macos"]
                }
            ],
            "pagination": {"page": 1, "limit": 500, "total": 3, "totalPages": 1}
        })
        .to_string();

        let thread_server = std::sync::Arc::clone(&server);
        let commit = std::sync::Arc::clone(&raycast_commit);
        let base_for_thread = base.clone();
        std::thread::spawn(move || {
            for request in thread_server.incoming_requests() {
                let url = request.url().to_owned();
                let hn_json = || {
                    serde_json::json!({
                        "id": "u1", "name": "hn", "title": "Hacker News",
                        "description": "Read the front page",
                        "author": {"name": "Ray", "handle": "ray"},
                        "platforms": ["macOS"], "download_count": 2500,
                        "commit_sha": *commit.lock().unwrap(),
                        "metadata_count": 1,
                        "readme_assets_path": format!("{base_for_thread}/assets/"),
                        "store_url": "https://www.raycast.com/ray/hn",
                        "download_url": format!("{base_for_thread}/dl/hn.zip"),
                        "updated_at": 1_788_465_682,
                        "commands": [{"id": "c", "name": "front", "title": "Front Page Stories",
                                      "mode": "view"}]
                    })
                };
                let (status, body): (u16, Vec<u8>) = match url.as_str() {
                    "/v1/store/list?page=1&limit=500" => (200, vicinae_listing.clone().into_bytes()),
                    "/v1/raycast/get-compat" => (
                        200,
                        br#"{"hn": {"status": "partial", "confidence": "high", "notes": ["No menu bar"]}}"#
                            .to_vec(),
                    ),
                    "/readme/clock.md" => (200, b"## Usage\n\nPress it.".to_vec()),
                    "/dl/clock.zip" => (200, clock.clone()),
                    "/dl/hn.zip" => (200, hn.clone()),
                    "/dl/evil.zip" => (200, evil.clone()),
                    "/raycast/store_listings?page=1&per_page=50"
                    | "/raycast/store_listings/search?q=hacker%20news" => (
                        200,
                        serde_json::json!({"data": [hn_json()]})
                            .to_string()
                            .into_bytes(),
                    ),
                    "/raycast/extensions/ray/hn" => (200, hn_json().to_string().into_bytes()),
                    _ => (404, b"not found".to_vec()),
                };
                let _ =
                    request.respond(tiny_http::Response::from_data(body).with_status_code(status));
            }
        });
        FakeStore {
            base,
            raycast_commit,
            _server: server,
        }
    }

    fn start_engine(&self) -> Daemon {
        let vicinae = format!("{}/v1", self.base);
        let raycast = format!("{}/raycast", self.base);
        Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", move |_| {
            vec![
                ("VICINAE_API_URL", vicinae.into()),
                ("COMPASS_RAYCAST_API_URL", raycast.into()),
            ]
        })
    }
}

fn root_ids(daemon: &Daemon, text: &str) -> Vec<String> {
    use compass_ipc::{Request, Response};
    let Response::QueryResults { hits } = daemon.request(Request::Query { text: text.into() })
    else {
        panic!("no query results");
    };
    hits.into_iter().map(|hit| hit.id).collect()
}

#[test]
fn the_vicinae_store_lists_installs_into_root_search_and_uninstalls() {
    use compass_ipc::{ErrorKind, Request, Response, StoreKind};
    let store = FakeStore::start();
    let daemon = store.start_engine();
    let data_home = daemon._dirs.path().join("data-home/vicinae");
    let extensions = data_home.join("extensions");

    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert_eq!(heading, "Extensions");
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["clock", "evil"], "the macOS-only one is dropped");
    assert_eq!(entries[0].id, "store.vicinae.clock");
    assert_eq!(entries[0].downloads, "1.1K");
    assert!(!entries[0].installed);

    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: "time".into(),
    }) else {
        panic!("no filtered listing");
    };
    assert_eq!(entries.len(), 1, "filtered by the description");

    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Vicinae,
        author: "zoe".into(),
        name: "clock".into(),
    }) else {
        panic!("no detail");
    };
    assert!(
        detail.markdown.starts_with("# Clock\n"),
        "{}",
        detail.markdown
    );
    assert!(detail.markdown.contains("**Updated** 2026-07-02"));
    assert!(
        detail
            .markdown
            .contains("- **Show Time Now** — Says the time")
    );
    assert!(
        detail.markdown.contains("Press it."),
        "the README is appended"
    );

    assert!(
        !root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id.contains("store.vicinae.clock")),
        "not in root search before it is installed"
    );
    let Response::StoreInstalled { id, title } = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        author: "zoe".into(),
        name: "clock".into(),
    }) else {
        panic!("not installed");
    };
    assert_eq!(
        (id.as_str(), title.as_str()),
        ("store.vicinae.clock", "Clock")
    );
    assert!(
        extensions
            .join("store.vicinae.clock/package.json")
            .is_file()
    );
    assert!(
        extensions
            .join("store.vicinae.clock/show-time.js")
            .is_file()
    );
    assert!(
        root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id == "@zoe/store.vicinae.clock:show-time"),
        "the installed command is in root search"
    );
    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert!(entries[0].installed && !entries[0].update_available);

    // A bundle whose entry climbs out is refused whole.
    let Response::Error(err) = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        author: "mallory".into(),
        name: "evil".into(),
    }) else {
        panic!("a zip-slip bundle was installed");
    };
    assert!(
        err.message.contains("outside its directory"),
        "{}",
        err.message
    );
    assert!(!extensions.join("store.vicinae.evil").exists());
    assert!(!extensions.join("escaped.txt").exists());
    assert!(!data_home.join("escaped.txt").exists());

    assert!(matches!(
        daemon.request(Request::StoreUninstall {
            id: "store.vicinae.clock".into()
        }),
        Response::Ack
    ));
    assert!(!extensions.join("store.vicinae.clock").exists());
    assert!(
        !root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id.contains("store.vicinae.clock")),
        "gone from root search"
    );
    let Response::Error(err) = daemon.request(Request::StoreUninstall {
        id: "store.vicinae.clock".into(),
    }) else {
        panic!("uninstalled twice");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    let Response::Error(err) = daemon.request(Request::OpenUrl {
        url: "file:///etc/passwd".into(),
    }) else {
        panic!("a file URL was opened");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn the_raycast_store_badges_compatibility_and_notices_an_update() {
    use compass_ipc::{ErrorKind, Request, Response, StoreKind};
    let store = FakeStore::start();
    let daemon = store.start_engine();

    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert_eq!(heading, "Extensions");
    assert_eq!(entries.len(), 1, "a macOS listing is offered on Linux");
    assert_eq!(entries[0].id, "store.raycast.hn");
    if cfg!(target_os = "linux") {
        assert_eq!(entries[0].compat, Some(1), "partial, from the sheet");
    }

    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        author: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("no detail");
    };
    assert_eq!(
        detail.screenshots,
        [format!("{}/assets/metadata/hn-1.png", store.base)]
    );
    assert_eq!(
        detail.store_url.as_deref(),
        Some("https://www.raycast.com/ray/hn")
    );
    if cfg!(target_os = "linux") {
        assert!(detail.markdown.contains("works but has a few quirks"));
        assert!(detail.markdown.contains("No menu bar"));
    }

    let Response::StoreInstalled { id, .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Raycast,
        author: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("not installed");
    };
    assert_eq!(id, "store.raycast.hn");
    assert!(
        root_ids(&daemon, "front page stories")
            .iter()
            .any(|id| id == "@ray/store.raycast.hn:front")
    );

    let search = || {
        let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
            store: StoreKind::Raycast,
            query: "hacker news".into(),
        }) else {
            panic!("no search");
        };
        assert_eq!(heading, "Results");
        entries.into_iter().next().expect("a result")
    };
    let current = search();
    assert!(current.installed && !current.update_available);
    *store.raycast_commit.lock().unwrap() = "c2".into();
    assert!(search().update_available, "a new commit is an update");

    let Response::StoreInstalled { .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Raycast,
        author: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("not updated");
    };
    assert!(
        !search().update_available,
        "reinstalling applies the update"
    );

    let Response::Error(err) = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        author: "ray".into(),
        name: "missing".into(),
    }) else {
        panic!("a missing extension had a page");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(err.message.contains("\"ray/missing\""), "{}", err.message);
}

/// The real stores, end to end: browse both, open a detail page, install a
/// Vicinae store extension into a temp data home and uninstall it. Network,
/// so ignored by default; run with
/// `cargo test -p vicinae --test engine_end_to_end -- --ignored real_stores`.
#[test]
#[ignore = "talks to api.vicinae.com and backend.raycast.com"]
fn real_stores_smoke() {
    use compass_ipc::{Request, Response, StoreKind};
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        // The invoking shell's proxy, which the harness otherwise strips.
        [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "NO_PROXY",
        ]
        .into_iter()
        .filter_map(|name| Some((name, std::env::var_os(name)?)))
        .collect()
    });

    let entries = match daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) {
        Response::StoreListing { entries, .. } => entries,
        other => panic!("no Vicinae listing: {other:?}"),
    };
    assert!(entries.len() > 10, "{} extensions", entries.len());
    let first = entries[0].clone();
    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Vicinae,
        author: first.author.clone(),
        name: first.name.clone(),
    }) else {
        panic!("no Vicinae detail");
    };
    assert!(detail.markdown.contains(&first.title));

    let Response::StoreInstalled { id, .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        author: first.author.clone(),
        name: first.name.clone(),
    }) else {
        panic!("not installed");
    };
    let installed = daemon
        ._dirs
        .path()
        .join("data-home/vicinae/extensions")
        .join(&id);
    assert!(installed.join("package.json").is_file());
    assert!(matches!(
        daemon.request(Request::StoreUninstall { id }),
        Response::Ack
    ));
    assert!(!installed.exists());

    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: String::new(),
    }) else {
        panic!("no Raycast listing");
    };
    assert!(!entries.is_empty());
    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: "spotify".into(),
    }) else {
        panic!("no Raycast search");
    };
    assert_eq!(heading, "Results");
    let hit = entries.first().expect("a Raycast result").clone();
    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        author: hit.author,
        name: hit.name,
    }) else {
        panic!("no Raycast detail");
    };
    assert!(detail.markdown.starts_with("# "));
}

/// The input server is started, told every snippet keyword, told again as
/// snippets change, and stopped and started by `SetInputServerEnabled` —
/// against a scripted helper that logs what it is asked and touches no
/// device.
#[test]
fn the_input_server_is_told_the_keywords_and_follows_the_setting() {
    use compass_ipc::{Request, Response};

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_input_server.py");
    let log = std::sync::Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let log_path = std::sync::Arc::clone(&log);
    let daemon = Daemon::start_prepared(&[], "{}", move |root| {
        let file = root.join("input-server.log");
        *log_path.lock().unwrap() = file.clone();
        vec![
            ("VICINAE_INPUT_SERVER_BIN", script.into_os_string()),
            ("FAKE_INPUT_LOG", file.into_os_string()),
        ]
    });
    let log = log.lock().unwrap().clone();
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    let wait_for = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if calls().contains(what) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the input server was never sent {what}; it got:\n{}",
            calls()
        );
    };
    let status = |daemon: &Daemon| match daemon.request(Request::InputServerStatus) {
        Response::InputServerStatus(status) => status,
        other => panic!("unexpected {other:?}"),
    };

    wait_for("Snippet/getCapabilities");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !status(&daemon).running {
        assert!(
            Instant::now() < deadline,
            "never running: {:?}",
            status(&daemon)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let now = status(&daemon);
    assert!(now.enabled && now.injection, "{now:?}");

    let Response::Snippets { snippets } = daemon.request(Request::SaveSnippet {
        id: None,
        name: "Sig".into(),
        text: "Best".into(),
        keyword: Some(";sig".into()),
        word: true,
        apps: vec![],
    }) else {
        panic!("not saved");
    };
    wait_for(r#""trigger": ";sig""#);
    assert!(
        calls().contains(r#""mode": "Word""#),
        "registered as a word snippet: {}",
        calls()
    );
    assert_eq!(status(&daemon).keywords, 1);

    daemon.request(Request::RemoveSnippet {
        id: snippets[0].id.clone(),
    });
    wait_for("Snippet/removeSnippet");

    let Response::InputServerStatus(off) =
        daemon.request(Request::SetInputServerEnabled { enabled: false })
    else {
        panic!("no status");
    };
    assert!(!off.enabled && !off.running, "{off:?}");
    let saved =
        std::fs::read_to_string(log.parent().unwrap().join("config/vicinae/vicinae.json")).unwrap();
    assert!(saved.contains("\"input_server\""), "{saved}");

    let before = calls().matches("Snippet/getCapabilities").count();
    let Response::InputServerStatus(on) =
        daemon.request(Request::SetInputServerEnabled { enabled: true })
    else {
        panic!("no status");
    };
    assert!(on.enabled, "{on:?}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while calls().matches("Snippet/getCapabilities").count() == before {
        assert!(Instant::now() < deadline, "not restarted");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A second extension, `hosts`, whose commands reach the host APIs the
/// engine routes since IPC v15: `FileSearch`, `Wallpaper`, `BrowserExtension`,
/// workspaces through `WindowManagement`, and `Command`. Each writes what it
/// was answered into its support directory (the one place the sandbox lets
/// it write) or shows it in a `Detail`.
fn install_host_extension(root: &Path) -> PathBuf {
    let ext = root.join("data-home/vicinae/extensions/hosts");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "hosts", "title": "Hosts", "author": "someone",
            "commands": [
              {"name": "probe", "title": "Probe Hosts", "mode": "view"},
              {"name": "launcher", "title": "Launch Sibling", "mode": "no-view"},
              {"name": "target", "title": "Launch Target", "mode": "no-view",
               "arguments": [{"name": "who", "type": "text", "placeholder": "Who",
                              "required": false}]},
              {"name": "badlaunch", "title": "Launch Nothing", "mode": "no-view"},
              {"name": "prefs", "title": "Open Preferences", "mode": "no-view"}
            ]}"#,
    )
    .unwrap();
    let support = root.join("data-home/vicinae/support/hosts");
    let wall = root.join("wall.png");
    std::fs::write(&wall, b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(
        ext.join("probe.js"),
        format!(
            "const React = require('react');
             const {{ Detail, FileSearch, Wallpaper, BrowserExtension, WindowManagement,
                      environment }} = require('@vicinae/api');
             const said = (p) => p.then((v) => v === undefined ? 'ok' : JSON.stringify(v),
                                        (e) => 'error: ' + (e && e.message ? e.message : String(e)));
             module.exports.default = () => {{
               const [text, setText] = React.useState('');
               React.useEffect(() => {{
                 Promise.all([
                   Promise.resolve(JSON.stringify([FileSearch, Wallpaper, BrowserExtension,
                     WindowManagement].map((api) => environment.canAccess(api)))),
                   said(FileSearch.search('quarterly', {{ limit: 5 }})),
                   said(Wallpaper.set({wall:?}, {{ fit: 'Contain' }})),
                   said(Wallpaper.set('/nonexistent/wall.png')),
                   said(BrowserExtension.getTabs()),
                   said(WindowManagement.getWorkspaces()),
                   said(WindowManagement.getActiveWorkspace()),
                   said(WindowManagement.getActiveWindow().then((w) => w.id)),
                 ]).then((answers) => setText(answers.join(' | ')));
               }}, []);
               return React.createElement(Detail, {{ markdown: text || 'asking' }});
             }};",
            wall = wall.to_string_lossy()
        ),
    )
    .unwrap();
    let launched = support.join("launched.txt");
    std::fs::write(
        ext.join("launcher.js"),
        format!(
            "const {{ launchCommand, updateCommandMetadata, LaunchType }} = require('@vicinae/api');
             module.exports.default = async () => {{
               await updateCommandMetadata({{ subtitle: '3 unread' }});
               let out = 'launched';
               try {{
                 await launchCommand({{ name: 'target', type: LaunchType.UserInitiated,
                   arguments: {{ who: 'ada' }}, context: {{ from: 'launcher' }} }});
               }} catch (e) {{ out = 'error: ' + ((e && e.message) || String(e)); }}
               require('node:fs').writeFileSync({launched:?}, out);
             }};",
            launched = launched.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("target.js"),
        format!(
            "module.exports.default = async (props) => {{
               require('node:fs').writeFileSync({target:?},
                 JSON.stringify({{ who: props.arguments.who, context: props.launchContext }}));
             }};",
            target = support.join("target.txt").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("badlaunch.js"),
        format!(
            "const {{ launchCommand, LaunchType }} = require('@vicinae/api');
             module.exports.default = async () => {{
               let out = 'launched';
               try {{
                 await launchCommand({{ name: 'nothing', type: LaunchType.UserInitiated }});
               }} catch (e) {{ out = 'error: ' + ((e && e.message) || String(e)); }}
               require('node:fs').writeFileSync({bad:?}, out);
             }};",
            bad = support.join("bad.txt").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("prefs.js"),
        format!(
            "const {{ openCommandPreferences }} = require('@vicinae/api');
             module.exports.default = async () => {{
               await openCommandPreferences();
               require('node:fs').writeFileSync({opened:?}, 'opened');
             }};",
            opened = support.join("prefs.txt").to_string_lossy()
        ),
    )
    .unwrap();
    support
}

/// A directory of stand-in programs for the wallpaper backends: `gsettings`
/// records its arguments, and `hyprctl`, `swww` and `awww` fail, so a real
/// one on the invoking machine's path is never reached.
fn fake_wallpaper_programs(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("gsettings.log");
    let scripts = [
        (
            "gsettings",
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
        ),
        ("hyprctl", "#!/bin/sh\nexit 1\n".to_owned()),
        ("swww", "#!/bin/sh\nexit 1\n".to_owned()),
        ("awww", "#!/bin/sh\nexit 1\n".to_owned()),
    ];
    for (name, body) in scripts {
        let path = bin.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn hyprland_fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../compass-platform-linux/tests/fixtures/hyprland")
            .join(name),
    )
    .unwrap()
}

#[test]
fn an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    use compass_testkit::fake_compositor::{FakeSocket, Framing};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = PathBuf::new();
    let mut hyprland = None;
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        root = dir.to_path_buf();
        install_host_extension(dir);
        let documents = dir.join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join("quarterly-report.pdf"), "%PDF").unwrap();
        let runtime_dir = dir.join("run");
        hyprland = Some(FakeSocket::replaying(
            &FakeSocket::hyprland_path(&runtime_dir, "v0.50_test"),
            Framing::Hyprland,
            vec![
                ("-j/clients".into(), hyprland_fixture("clients.json")),
                ("-j/workspaces".into(), hyprland_fixture("workspaces.json")),
                (
                    "-j/activeworkspace".into(),
                    hyprland_fixture("activeworkspace.json"),
                ),
                (
                    "-j/activewindow".into(),
                    hyprland_fixture("activewindow.json"),
                ),
            ],
        ));
        let bin = fake_wallpaper_programs(dir);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&path));
        vec![
            ("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string()),
            ("XDG_CURRENT_DESKTOP", "GNOME".into()),
            ("XDG_RUNTIME_DIR", runtime_dir.into_os_string()),
            ("HYPRLAND_INSTANCE_SIGNATURE", "v0.50_test".into()),
            ("PATH", std::env::join_paths(paths).unwrap()),
        ]
    });
    // The index scans in the background; the extension asks once it has.
    search_files_until(&daemon, "quarterly", None, |_, files| {
        files.iter().any(|file| file.name == "quarterly-report.pdf")
    });

    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hosts:probe".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Detail(detail) if detail.markdown.as_deref() != Some("asking")),
    );
    let View::Detail(detail) = view else {
        unreachable!()
    };
    let text = detail.markdown.unwrap_or_default();
    let answers: Vec<&str> = text.split(" | ").collect();
    assert_eq!(answers.len(), 8, "{text}");
    assert_eq!(
        answers[0], "[true,true,false,true]",
        "canAccess: files, wallpaper, no browser, windows"
    );
    let files: serde_json::Value = serde_json::from_str(answers[1]).expect(answers[1]);
    assert_eq!(files[0]["category"], "Document", "{files}");
    assert!(
        files[0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("/Documents/quarterly-report.pdf")),
        "{files}"
    );
    assert_eq!(answers[2], "ok", "the wallpaper was set");
    assert_eq!(answers[3], "error: No such file: /nonexistent/wall.png");
    assert_eq!(answers[4], "[]", "no browser is ever connected");
    let workspaces: serde_json::Value = serde_json::from_str(answers[5]).expect(answers[5]);
    assert_eq!(
        workspaces
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["id"].as_str().unwrap(), w["active"].as_bool().unwrap()))
            .collect::<Vec<_>>(),
        [("1", true), ("3", false)]
    );
    let active: serde_json::Value = serde_json::from_str(answers[6]).expect(answers[6]);
    assert_eq!(active["id"], "1");
    assert_eq!(active["monitorId"], "DP-1");
    assert_eq!(
        answers[7], "\"0x5581c8a4f310\"",
        "Hyprland's frontmost window"
    );

    let wall = root.join("wall.png");
    let log = std::fs::read_to_string(root.join("gsettings.log")).unwrap();
    let uri = format!("file://{}", wall.display());
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            format!("set org.gnome.desktop.background picture-uri {uri}"),
            format!("set org.gnome.desktop.background picture-uri-dark {uri}"),
            "set org.gnome.desktop.background picture-options scaled".to_owned(),
        ],
        "GNOME's three keys, Contain as `scaled`; the missing file set nothing"
    );
    assert!(
        hyprland
            .as_ref()
            .unwrap()
            .seen()
            .iter()
            .all(|request| request.starts_with("-j/")),
        "WindowManagement only asked"
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn an_extension_launches_a_sibling_relabels_itself_and_opens_its_preferences() {
    use compass_ipc::{ErrorKind, Request, Response, WindowCommand};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut support = PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        support = install_host_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let run = |id: &str| {
        daemon.request(Request::RunExtensionCommand {
            id: id.into(),
            arguments_json: None,
        })
    };

    // No window: a no-view sibling runs in the engine, with its arguments
    // and the launch context.
    assert_eq!(run("@someone/hosts:launcher"), Response::Ack);
    wait_for_content(&support.join("launched.txt"));
    assert_eq!(
        std::fs::read_to_string(support.join("launched.txt")).unwrap(),
        "launched"
    );
    wait_for_content(&support.join("target.txt"));
    let target: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(support.join("target.txt")).unwrap())
            .unwrap();
    assert_eq!(
        target,
        serde_json::json!({"who": "ada", "context": {"from": "launcher"}})
    );

    // The subtitle it set shows in root search and is served to the window.
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "launch sibling".into(),
    }) else {
        panic!("no results");
    };
    let hit = hits
        .iter()
        .find(|hit| hit.id == "@someone/hosts:launcher")
        .expect("the launcher command");
    assert_eq!(hit.subtitle.as_deref(), Some("3 unread"));
    assert_eq!(
        daemon.request(Request::ExtensionSubtitles),
        Response::ExtensionSubtitles {
            subtitles: vec![("@someone/hosts:launcher".into(), "3 unread".into())]
        }
    );

    // A command that is not installed is the C++'s own refusal.
    assert_eq!(run("@someone/hosts:badlaunch"), Response::Ack);
    wait_for_content(&support.join("bad.txt"));
    assert_eq!(
        std::fs::read_to_string(support.join("bad.txt")).unwrap(),
        "error: No such command"
    );

    // With a window: it is handed the launch, which it takes once.
    std::fs::remove_file(support.join("launched.txt")).unwrap();
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    assert_eq!(run("@someone/hosts:launcher"), Response::Ack);
    wait_for_content(&support.join("launched.txt"));
    let token = wait_for_launch(&window, 0);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::ExtensionLaunch {
            id: "@someone/hosts:target".into(),
            arguments_json: Some(r#"{"who":"ada"}"#.into()),
            preferences: false,
        }
    );
    let Response::Error(err) = daemon.request(Request::ExtensionLaunchFetch { token }) else {
        panic!("a launch was taken twice");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    assert_eq!(run("@someone/hosts:prefs"), Response::Ack);
    wait_for_content(&support.join("prefs.txt"));
    let token = wait_for_launch(&window, 1);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::ExtensionLaunch {
            id: "@someone/hosts:prefs".into(),
            arguments_json: None,
            preferences: true,
        }
    );
    assert!(window.seen().contains(&WindowCommand::Launch(token)));
    // Without a keyring there is nowhere to keep preferences, which is said.
    let Response::Error(err) = daemon.request(Request::ExtensionPreferences {
        id: "@someone/hosts:prefs".into(),
    }) else {
        panic!("preferences without a keyring were not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

/// Waits for the window to have been handed more than `after` launches, and
/// returns the token of the next one.
fn wait_for_launch(window: &FakeWindow, after: usize) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let launches: Vec<u64> = window
            .seen()
            .iter()
            .filter_map(|command| match command {
                compass_ipc::WindowCommand::Launch(token) => Some(*token),
                _ => None,
            })
            .collect();
        if let Some(token) = launches.get(after) {
            return *token;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the window was never handed a launch: {:?}", window.seen());
}
