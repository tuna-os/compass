//! The settings IPC and config watcher against one real engine and a recording
//! helper. All configuration, home, runtime and cache paths are temporary.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use compass_ipc::{ErrorKind, Request, Response};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(30);

struct Daemon {
    child: Child,
    runtime: tokio::runtime::Runtime,
    socket: PathBuf,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        Self::start_with_config(&configuration(false, json!(["~/first"]), json!([])).to_string())
    }

    fn start_with_config(config: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let binary = bin.join("compass");
        // current_exe must resolve beside our fake, never the installed helper.
        std::fs::hard_link(env!("CARGO_BIN_EXE_compass"), &binary)
            .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_compass"), &binary).map(|_| ()))
            .unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let helper = bin.join("compass-file-indexer");
        std::fs::copy(fixtures.join("recording_indexer.py"), &helper).unwrap();
        std::fs::copy(
            fixtures.join("fake_indexer.py"),
            bin.join("fake_indexer.py"),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(root.join("config/compass")).unwrap();
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        std::fs::write(root.join("config/compass/compass.json"), config).unwrap();
        let socket = root.join("ipc.sock");
        let child = Command::new(binary)
            .args(["--socket", socket.to_str().unwrap(), "serve", "--no-hotkey"])
            .current_dir(root)
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .env("HOME", root)
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_DATA_DIRS", root.join("empty"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CONFIG_DIRS", root.join("empty"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/compass-test-no-bus",
            )
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env(
                "COMPASS_UPDATE_FEED_URL",
                "http://127.0.0.1:9/releases/latest",
            )
            .env("COMPASS_INPUT_SERVER_BIN", "/bin/true")
            .env("COMPASS_TEST_INDEXER_LOG", root.join("indexer.jsonl"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Self {
            child,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            socket,
            dir,
        };
        wait_for(
            || daemon.try_request(Request::Ping).is_some(),
            "the engine socket",
        );
        daemon
    }

    fn try_request(&self, request: Request) -> Option<Response> {
        self.runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, async {
                compass_ipc::Client::connect(&self.socket)
                    .await?
                    .request(request)
                    .await
            })
            .await
            .ok()
            .and_then(Result::ok)
        })
    }

    fn request(&self, request: Request) -> Response {
        self.try_request(request).expect("the engine answers")
    }

    fn set(&self, name: &str, value: Value) -> Response {
        self.request(Request::SetSetting {
            key: format!("providers.files.preferences.{name}"),
            value_json: value.to_string(),
        })
    }

    fn records(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("indexer.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn configurations(&self) -> Vec<Value> {
        self.records()
            .into_iter()
            .filter(|row| row["method"] == "FileIndexer/configure")
            .collect()
    }

    fn wait_config(&self, paths: &[PathBuf], excluded: &[PathBuf]) -> Value {
        let expected = json!({"paths": paths, "excluded_paths": excluded});
        wait_for(
            || {
                self.configurations()
                    .last()
                    .is_some_and(|row| row["params"]["config"] == expected)
            },
            "the helper configuration",
        );
        let expected_files: Vec<PathBuf> =
            paths.iter().map(|root| root.join("probe.txt")).collect();
        wait_for(
            || match self.search("probe") {
                Response::Files { files, .. } => {
                    files
                        .into_iter()
                        .map(|file| PathBuf::from(file.path))
                        .collect::<Vec<_>>()
                        == expected_files
                }
                _ => false,
            },
            "the configured query result",
        );
        self.configurations().pop().unwrap()
    }

    fn search(&self, query: &str) -> Response {
        self.request(Request::SearchFiles {
            query: query.to_owned(),
            category: None,
        })
    }

    fn assert_query_root(&self, root: &Path) {
        let Response::Files { files, .. } = self.search("probe") else {
            panic!("the current shared file-search client must answer");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(Path::new(&files[0].path), root.join("probe.txt"));
    }

    fn write_config(&self, text: &str) {
        let directory = self.dir.path().join("config/compass");
        let staged = directory.join(".compass.json.tmp");
        std::fs::write(&staged, text).unwrap();
        std::fs::rename(staged, directory.join("compass.json")).unwrap();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn configuration(enabled: bool, paths: Value, excluded: Value) -> Value {
    json!({
        "tray": {"enabled": false},
        "input_server": {"enabled": false},
        "providers": {"files": {"preferences": {
            "autoIndexing": enabled,
            "indexingPaths": paths,
            "excludedIndexingPaths": excluded,
        }}},
    })
}

fn wait_for(mut ready: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn settle_watcher() {
    std::thread::sleep(compass::config_watch::DEBOUNCE * 5);
}

#[test]
fn file_settings_reconfigure_one_helper_and_survive_disable_and_reenable() {
    let daemon = Daemon::start();
    let root = daemon.dir.path();
    assert!(
        daemon.records().is_empty(),
        "disabled at startup means no helper"
    );
    let Response::Error(error) = daemon.search("probe") else {
        panic!("disabled indexing must be unavailable");
    };
    assert_eq!(error.kind, ErrorKind::Unsupported);

    assert_eq!(daemon.set("autoIndexing", json!(true)), Response::Ack);
    let first = daemon.wait_config(&[root.join("first")], &[]);
    daemon.assert_query_root(&root.join("first"));

    assert_eq!(
        daemon.set("indexingPaths", json!(["~/second/./", "~/second"])),
        Response::Ack
    );
    let changed = daemon.wait_config(&[root.join("second")], &[]);
    assert_eq!(
        changed["pid"], first["pid"],
        "roots change in the existing helper"
    );
    daemon.assert_query_root(&root.join("second"));
    assert_eq!(
        daemon.set("excludedIndexingPaths", json!(["~/second/private"])),
        Response::Ack
    );
    let changed = daemon.wait_config(&[root.join("second")], &[root.join("second/private")]);
    assert_eq!(changed["pid"], first["pid"]);
    let count = daemon.configurations().len();
    assert_eq!(
        daemon.set("excludedIndexingPaths", json!(["~/second/./private"])),
        Response::Ack
    );
    settle_watcher();
    daemon.assert_query_root(&root.join("second"));
    assert_eq!(
        daemon.configurations().len(),
        count,
        "same snapshot sends no configure"
    );

    let edited = configuration(true, json!(["third/./"]), json!(["~/third/private"]));
    daemon.write_config(&edited.to_string());
    let changed = daemon.wait_config(&[root.join("third")], &[root.join("third/private")]);
    assert_eq!(
        changed["pid"], first["pid"],
        "hand edits use the same live path"
    );
    daemon.assert_query_root(&root.join("third"));

    let count = daemon.configurations().len();
    let Response::Error(error) = daemon.set("indexingPaths", json!([7])) else {
        panic!("invalid settings IPC must be refused");
    };
    assert_eq!(error.kind, ErrorKind::BadRequest);
    for broken in [
        "{broken JSON",
        r#"{"providers":{"files":{"preferences":false}}}"#,
        r#"{"providers":{"files":{"preferences":{"indexingPaths":7}}}}"#,
    ] {
        daemon.write_config(broken);
        settle_watcher();
        daemon.assert_query_root(&root.join("third"));
        assert_eq!(
            daemon.configurations().len(),
            count,
            "invalid edits keep the good snapshot"
        );
    }
    daemon.write_config(&edited.to_string());
    settle_watcher();

    assert_eq!(daemon.set("autoIndexing", json!(false)), Response::Ack);
    assert_eq!(
        daemon
            .records()
            .iter()
            .filter(|row| row["event"] == "stop")
            .count(),
        1
    );
    assert!(matches!(daemon.search("probe"), Response::Error(_)));
    let direct = root.join("direct.txt");
    std::fs::write(&direct, "temporary fixture").unwrap();
    assert!(matches!(
        daemon.search(direct.to_str().unwrap()),
        Response::Files { .. }
    ));

    assert_eq!(
        daemon.set("indexingPaths", json!(["~/fourth"])),
        Response::Ack
    );
    assert_eq!(
        daemon.set("excludedIndexingPaths", json!(["~/fourth/private"])),
        Response::Ack
    );
    settle_watcher();
    assert_eq!(
        daemon.configurations().len(),
        count,
        "disabled edits are staged without a helper"
    );
    assert_eq!(daemon.set("autoIndexing", json!(true)), Response::Ack);
    let restarted = daemon.wait_config(&[root.join("fourth")], &[root.join("fourth/private")]);
    assert_ne!(restarted["pid"], first["pid"]);
    daemon.assert_query_root(&root.join("fourth"));
    assert_eq!(daemon.set("autoIndexing", json!(true)), Response::Ack);
    settle_watcher();
    assert_eq!(
        daemon
            .records()
            .iter()
            .filter(|row| row["event"] == "start")
            .count(),
        2
    );
    assert!(
        !daemon
            .records()
            .iter()
            .any(|row| row["method"] == "FileIndexer/rebuildIndex")
    );
    assert_eq!(daemon.set("autoIndexing", json!(false)), Response::Ack);
}

#[test]
fn invalid_startup_configuration_never_defaults_to_home_wide_indexing() {
    for broken in [
        "{broken JSON",
        r#"{"providers":{"files":{"preferences":false}}}"#,
        r#"{"providers":{"files":{"preferences":{"indexingPaths":7}}}}"#,
    ] {
        let daemon = Daemon::start_with_config(broken);
        assert!(matches!(daemon.search("probe"), Response::Error(_)));
        assert!(daemon.records().is_empty(), "no helper starts for {broken}");
        daemon.write_config(&configuration(true, json!(["~/recovered"]), json!([])).to_string());
        daemon.wait_config(&[daemon.dir.path().join("recovered")], &[]);
        daemon.assert_query_root(&daemon.dir.path().join("recovered"));
        assert_eq!(daemon.set("autoIndexing", json!(false)), Response::Ack);
    }
}
