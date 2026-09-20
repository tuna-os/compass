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
        let dirs = TempDir::new().expect("tempdir");
        let data = dirs.path().join("data");
        write_apps(&data, entries);

        let socket = dirs.path().join("ipc.sock");
        let child = Command::new(binary())
            .arg("--socket")
            .arg(&socket)
            .arg("serve")
            .env("XDG_DATA_DIRS", &data)
            // Keep the daemon out of the invoking user's home entirely: its
            // config, its launch history and its data all land in the tempdir.
            .env("XDG_DATA_HOME", dirs.path().join("data-home"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            .env("HOME", dirs.path())
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
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------

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
    let all: serde_json::Value =
        serde_json::from_str(&daemon.client(&["query", "--json", ""])).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["id"], "browser.desktop");
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
