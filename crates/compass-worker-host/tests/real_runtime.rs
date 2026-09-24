//! The host against the **real** extension runtime.
//!
//! `node_worker.rs` drives a mock worker that speaks the same wire format,
//! which is a claim about the format, not about the runtime. This one runs
//! `src/typescript/extension-manager`'s own bundle — the thing the C++ engine
//! ships as `vicinae-worker-ts` — loads a real no-view command into it, and
//! serves the `Storage` calls that command makes.
//!
//! # Why this can skip
//!
//! The bundle is built by a toolchain that is not cargo: figura (C++) generates
//! the TypeScript protos from `figura/*.fig`, then esbuild bundles them with
//! the API package. `make extension-runtime` does both. When the bundle is not
//! there this test skips, because a missing build is not a failing host — but
//! `COMPASS_REQUIRE_RUNTIME=1` turns the skip into a failure, so CI can insist
//! once it builds it.

use std::path::{Path, PathBuf};

use compass_local_storage::{LocalStorage, namespace_for};
use compass_worker_host::extension_manager::ManagerClient;
use compass_worker_host::session::{Router, Session, Turn};
use compass_worker_host::storage_service::StorageService;
use compass_worker_host::{Worker, extension_manager};

/// The repository root, two levels above this crate.
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the root")
        .to_path_buf()
}

/// The runtime bundle, or `None` when it has not been built.
fn runtime() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("COMPASS_EXTENSION_RUNTIME") {
        let path = PathBuf::from(from_env);
        assert!(
            path.exists(),
            "COMPASS_EXTENSION_RUNTIME points at {}, which does not exist",
            path.display()
        );
        return Some(path);
    }

    let built = repo().join("src/typescript/extension-manager/dist/runtime.js");
    built.exists().then_some(built)
}

fn require_runtime() -> bool {
    std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_some_and(|v| v == "1")
}

fn node() -> PathBuf {
    for candidate in ["/usr/bin/node", "/usr/local/bin/node", "/bin/node"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("node")
}

/// A no-view command that writes one value and reads it back.
///
/// CommonJS. `load-no-view-command.ts` dynamic-`import()`s the entrypoint and
/// takes `module.default.default`; for a CommonJS file the namespace's
/// `default` *is* `module.exports`, so the function goes on
/// `module.exports.default`. `require` is patched inside the worker, so
/// `@vicinae/api` resolves to the bundled API rather than to node_modules.
const COMMAND: &str = r#"
const { LocalStorage } = require("@vicinae/api");

module.exports.default = async () => {
  await LocalStorage.setItem("greeting", "hello");
  const read = await LocalStorage.getItem("greeting");
  require("node:fs").writeFileSync(process.env.COMPASS_TEST_OUT, JSON.stringify({ read }));
};
"#;

fn load_options(entrypoint: &Path) -> extension_manager::LoadOptions {
    extension_manager::LoadOptions {
        // No view: the command runs, does its work and asks to be unloaded,
        // so nothing here needs a front end.
        mode: extension_manager::CommandMode::NoView,
        env: extension_manager::CommandEnv::Production,
        vicinae_path: "/tmp/vicinae".to_owned(),
        entrypoint: entrypoint.to_string_lossy().into_owned(),
        is_raycast: false,
        command_name: "store".to_owned(),
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

/// Runs `source` as a no-view command in a fresh runtime process, serving its
/// `Storage` calls from `storage`, until it has made `last` or closed. Returns
/// the methods answered and the JSON the command wrote. The process is ended
/// before this returns, as a kill would.
fn run_command(
    runtime: &Path,
    dir: &Path,
    storage: &LocalStorage<'_>,
    source: &str,
    last: &str,
) -> (Vec<String>, serde_json::Value) {
    let out = dir.join("observed.json");
    let _ = std::fs::remove_file(&out);
    let entrypoint = dir.join("command.js");
    std::fs::write(&entrypoint, source).expect("write the command");

    // Under `timeout`, so a runtime that never answers ends the test with a
    // failed assertion instead of hanging the suite: the host blocks on a read
    // until the pipe closes, and killing the child is what closes it.
    let mut command = std::process::Command::new("timeout");
    command
        .arg("30")
        .arg(node())
        .arg(runtime)
        .env("COMPASS_TEST_OUT", &out);

    let mut worker = Worker::spawn(command).expect("the runtime starts");
    // Kept so the runtime can be ended explicitly below. Without it the process
    // lives until `timeout` reaps it, which `cargo nextest` reports as a LEAK.
    let worker_pid = worker.pid();

    let load_id = ManagerClient::new(&mut worker)
        .load(&load_options(&entrypoint))
        .expect("sending load");
    let response = worker
        .next_message()
        .expect("reading the load response")
        .expect("the runtime answered");
    assert_eq!(response.id, Some(load_id));
    let session_id = response
        .result
        .as_ref()
        .and_then(|r| r.get("session_id"))
        .and_then(serde_json::Value::as_str)
        .expect("the load response carries a session id")
        .to_owned();

    // Within a second of the load reply, or the runtime unloads the worker:
    // "worker for command <id> did not complete handshake under 1s".
    ManagerClient::new(&mut worker)
        .ready(&session_id)
        .expect("sending ready");

    let service = StorageService::new(storage.scoped(&namespace_for("hn")));
    let mut session = Session::new(worker, &session_id, Router::new().with(&service));

    // A real runtime says more than a mock one: it starts a worker thread,
    // handshakes, loads the module and only then calls. The bound is a guard
    // against a hang rather than a count of the conversation.
    let mut answered = Vec::new();
    for _ in 0..200 {
        match session.pump_once().expect("a turn") {
            Turn::Answered { method } => answered.push(method),
            Turn::Closed => break,
            Turn::Crashed { reason } => panic!("the runtime crashed: {reason}"),
            Turn::Deferred { method, .. } => panic!("nothing in this command defers: {method}"),
            Turn::Nothing | Turn::OtherSession { .. } => {}
        }
        // Stop pumping once the command has had its last answer. Pumping past
        // it would block on a runtime that has nothing left to say.
        if answered.iter().any(|method| method == last) {
            break;
        }
    }

    // The command writes the file *after* its last reply lands. Five seconds
    // is a bound on a hang, not a tuning knob.
    for _ in 0..500 {
        if out.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // SIGTERM goes to `timeout`, which forwards it to node.
    drop(session);
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(worker_pid.to_string())
        .status();

    let observed =
        serde_json::from_slice(&std::fs::read(&out).expect("the command wrote its result"))
            .expect("it wrote JSON");
    (answered, observed)
}

#[test]
fn the_real_runtime_runs_a_command_that_stores_through_this_host() {
    let Some(runtime) = runtime() else {
        assert!(
            !require_runtime(),
            "COMPASS_REQUIRE_RUNTIME=1 but the runtime bundle is missing; \
             build it with `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle; see the module docs");
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = compass_sqlcipher_sys::open(&dir.path().join("vicinae.db"), &[])
        .expect("an unencrypted db");
    compass_db::vicinae::run(&db).expect("the migrations apply");
    let storage = LocalStorage::new(&db);

    let (answered, observed) = run_command(&runtime, dir.path(), &storage, COMMAND, "Storage/get");
    assert_eq!(
        answered,
        ["Storage/set".to_owned(), "Storage/get".to_owned()],
        "the runtime did not make the two calls the command makes"
    );
    assert_eq!(
        observed["read"], "hello",
        "the command read back something else: {observed}"
    );
    assert_eq!(
        storage
            .scoped(&namespace_for("hn"))
            .get("greeting")
            .expect("read")
            .expect("a row")
            .to_json(),
        serde_json::json!("hello"),
        "and the host's own store disagrees"
    );
}

/// Suite 1's persistence case: a value one worker stored is there for the
/// next, after the first process was killed.
#[test]
fn a_value_stored_before_the_worker_is_killed_is_read_by_the_next_one() {
    let Some(runtime) = runtime() else {
        assert!(
            !require_runtime(),
            "COMPASS_REQUIRE_RUNTIME=1 but the runtime bundle is missing; \
             build it with `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle; see the module docs");
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("vicinae.db");
    {
        let db = compass_sqlcipher_sys::open(&path, &[]).expect("an unencrypted db");
        compass_db::vicinae::run(&db).expect("the migrations apply");
        let storage = LocalStorage::new(&db);
        run_command(&runtime, dir.path(), &storage, COMMAND, "Storage/get");
    }

    // A new database handle as well as a new process: nothing survives in
    // memory on either side.
    let db = compass_sqlcipher_sys::open(&path, &[]).expect("reopened");
    let storage = LocalStorage::new(&db);
    let (answered, observed) = run_command(
        &runtime,
        dir.path(),
        &storage,
        r#"
const { LocalStorage } = require("@vicinae/api");
module.exports.default = async () => {
  const read = await LocalStorage.getItem("greeting");
  require("node:fs").writeFileSync(process.env.COMPASS_TEST_OUT, JSON.stringify({ read }));
};
"#,
        "Storage/get",
    );
    assert_eq!(answered, ["Storage/get".to_owned()]);
    assert_eq!(observed["read"], "hello", "{observed}");
}
