//! The host against a real Node worker.
//!
//! Every other test here drives the host with `cat` or with staged bytes.
//! This one runs Node, speaking the framing copied out of
//! `src/typescript/extension-manager/src/index.ts`, through a whole session:
//! load, ready, an extension call, the host's reply, a second call that reads
//! back what the first one wrote.
//!
//! What it proves is narrow and worth having: the host's framing, its manager
//! protocol, its tsapi routing and its storage service all agree with an
//! independent implementation of the same wire format, and they agree through
//! a pipe rather than through a function call.
//!
//! What it does not prove is that the real `vicinae-worker-ts` works: that one
//! is built by a toolchain this test does not run, and running it is Phase 4's
//! gate, not this test.

use std::path::{Path, PathBuf};

use compass_local_storage::{LocalStorage, namespace_for};
use compass_sqlcipher_sys::Database;
use compass_worker_host::extension_manager::ManagerClient;
use compass_worker_host::session::{Router, Session, Turn};
use compass_worker_host::storage_service::StorageService;
use compass_worker_host::{Worker, extension_manager};

fn node() -> PathBuf {
    for candidate in ["/usr/bin/node", "/usr/local/bin/node", "/bin/node"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    // Last resort: whatever is on PATH. `Command` will report it if missing.
    PathBuf::from("node")
}

fn worker_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock-worker.js")
}

/// A `LoadOptions` with the shape the C++ engine sends.
fn load_options() -> extension_manager::LoadOptions {
    extension_manager::LoadOptions {
        mode: extension_manager::CommandMode::View,
        env: extension_manager::CommandEnv::Production,
        vicinae_path: "/tmp/vicinae".to_owned(),
        entrypoint: "index.js".to_owned(),
        is_raycast: false,
        command_name: "search".to_owned(),
        extension_id: "hn".to_owned(),
        extension_name: "Hacker News".to_owned(),
        owner_or_author_name: "someone".to_owned(),
        arguments: serde_json::json!({}),
        preferences: serde_json::json!({}),
        launch_context: serde_json::Value::Null,
        launch_type: extension_manager::LaunchType::User,
        capabilities: extension_manager::Capabilities::default(),
        fallback_text: None,
        cwd: None,
    }
}

#[test]
fn a_node_worker_loads_stores_and_reads_back_through_the_host() {
    let script = worker_script();
    assert!(script.exists(), "{} is missing", script.display());

    let dir = tempfile::tempdir().expect("a temporary directory");
    let out = dir.path().join("observed.json");

    let mut command = std::process::Command::new(node());
    command.arg(&script).arg(&out);

    let mut worker = match Worker::spawn(command) {
        Ok(worker) => worker,
        Err(error) => panic!(
            "could not start node ({error}); this test needs it and will not pretend otherwise"
        ),
    };

    // Load, exactly as `ExtensionCommandRuntime::load` does.
    let load_id = ManagerClient::new(&mut worker)
        .load(&load_options())
        .expect("sending load");

    let response = worker
        .next_message()
        .expect("reading the load response")
        .expect("the worker answered");
    assert_eq!(response.id, Some(load_id));
    let session_id = response
        .result
        .as_ref()
        .and_then(|r| r.get("session_id"))
        .and_then(serde_json::Value::as_str)
        .expect("the load response carries a session id")
        .to_owned();

    // Then `ready`, which is what starts the extension.
    ManagerClient::new(&mut worker)
        .ready(&session_id)
        .expect("sending ready");

    let db = Database::open(&dir.path().join("vicinae.db"), &[]).expect("an unencrypted db");
    compass_db::vicinae::run(&db).expect("the migrations apply");
    let storage = LocalStorage::new(&db);
    let service = StorageService::new(storage.scoped(&namespace_for("hn")));

    let mut session = Session::new(worker, &session_id, Router::new().with(&service));

    // Pump until the worker has what it wanted. The bound is a guard against
    // a hang, not a tuning knob: the conversation is five messages long.
    let mut answered = Vec::new();
    for _ in 0..20 {
        match session.pump_once().expect("a turn") {
            Turn::Answered { method } => answered.push(method),
            Turn::Closed => break,
            Turn::Crashed { reason } => panic!("the worker crashed: {reason}"),
            Turn::Nothing | Turn::OtherSession { .. } => {}
        }
        if out.exists() {
            break;
        }
    }

    assert_eq!(
        answered,
        vec!["Storage/set".to_owned(), "Storage/get".to_owned()],
        "the host did not answer the two calls the worker made"
    );

    let observed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out).expect("the worker wrote what it saw"))
            .expect("it wrote JSON");
    assert_eq!(
        observed["result"], "hello",
        "the value the worker read back is not the one it stored: {observed}"
    );
    assert_eq!(observed["id"], 2);

    // And the host's own view agrees.
    assert_eq!(
        storage
            .scoped(&namespace_for("hn"))
            .get("greeting")
            .expect("read")
            .expect("a row")
            .to_json(),
        serde_json::json!("hello")
    );
}
