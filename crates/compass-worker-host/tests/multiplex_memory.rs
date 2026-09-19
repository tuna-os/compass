//! INVESTIGATION: what three extensions cost in one process versus three.
//!
//! `peak_memory.rs` measures the arrangement the host uses today — one node
//! process per command — and reports 261 MB of summed peak RSS against a
//! 150 MB budget. Two things about that number need separating before anyone
//! acts on it.
//!
//! **Summed RSS over-counts.** Three node processes share the interpreter's
//! text pages, and RSS charges every one of them the full amount. Measured
//! with `smaps_rollup`: three idle node processes report ~41 MB of RSS each but
//! ~18 MB of PSS each, because ~34 MB of it is one shared mapping. So the
//! honest system figure is private memory plus shared-counted-once, not a sum
//! of RSS.
//!
//! **The runtime is built to multiplex and the host is not using it.**
//! `extension-manager/src/index.ts` keeps `workerMap: Map<sessionId,
//! WorkerInfo>` and spawns `new Worker(__filename)` per session: many
//! extensions, one process, one isolate each. The `session_id` the load reply
//! carries exists for exactly that. Today's host spawns a fresh process per
//! command anyway, so node's fixed cost is paid N times instead of once.
//!
//! This test measures both arrangements back to back and prints the
//! difference. It asserts only that the multiplexed one is not *worse*, because
//! the size of the win is a measurement, not a promise.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use compass_worker_host::extension_manager::ManagerClient;
use compass_worker_host::{Worker, extension_manager};

const EXTENSIONS: usize = 3;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the root")
        .to_path_buf()
}

fn runtime() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("COMPASS_EXTENSION_RUNTIME") {
        let path = PathBuf::from(from_env);
        assert!(path.exists(), "COMPASS_EXTENSION_RUNTIME does not exist");
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

fn process_tree(pid: u32) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![pid];
    while let Some(current) = queue.pop() {
        if !seen.insert(current) {
            continue;
        }
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{current}/task")) else {
            continue;
        };
        for task in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            queue.extend(
                children
                    .split_whitespace()
                    .filter_map(|c| c.parse::<u32>().ok()),
            );
        }
    }
    seen
}

/// Private memory plus shared memory counted once, in kilobytes.
///
/// The honest way to add up a set of processes that share mappings. Summing
/// `Rss` charges each process for the whole shared interpreter; summing `Pss`
/// divides shared pages by the number of mappers, which is right for the
/// processes measured but silently wrong if something outside the set also
/// maps them. This takes private in full and shared once, which is what the
/// machine actually has to find.
fn footprint_kb(pids: &[u32]) -> (u64, u64, u64) {
    let mut private = 0u64;
    let mut pss = 0u64;
    let mut shared_max = 0u64;
    for &pid in pids {
        for p in process_tree(pid) {
            let Ok(r) = std::fs::read_to_string(format!("/proc/{p}/smaps_rollup")) else {
                continue;
            };
            let get = |f: &str| -> u64 {
                r.lines()
                    .find_map(|l| l.strip_prefix(&format!("{f}:")))
                    .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
                    .unwrap_or(0)
            };
            private += get("Private_Clean") + get("Private_Dirty");
            pss += get("Pss");
            shared_max = shared_max.max(get("Shared_Clean") + get("Shared_Dirty"));
        }
    }
    (private + shared_max, private, pss)
}

/// A command that loads the API, says so, and stays resident.
fn command_source(marker: &Path) -> String {
    format!(
        r#"
require("@vicinae/api");
module.exports.default = async () => {{
  require("node:fs").writeFileSync({marker:?}, "loaded");
  await new Promise(() => {{}});
}};
"#
    )
}

fn load_options(entrypoint: &Path, id: usize) -> extension_manager::LoadOptions {
    extension_manager::LoadOptions {
        mode: extension_manager::CommandMode::NoView,
        env: extension_manager::CommandEnv::Production,
        vicinae_path: "/tmp/vicinae".to_owned(),
        entrypoint: entrypoint.to_string_lossy().into_owned(),
        is_raycast: false,
        command_name: format!("cmd{id}"),
        extension_id: format!("ext{id}"),
        extension_name: format!("Extension {id}"),
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

fn spawn(runtime: &Path) -> Worker {
    let mut command = std::process::Command::new("timeout");
    command.arg("60").arg(node()).arg(runtime);
    Worker::spawn(command).expect("the runtime starts")
}

fn load_into(worker: &mut Worker, entrypoint: &Path, id: usize) {
    let load_id = ManagerClient::new(worker)
        .load(&load_options(entrypoint, id))
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
    ManagerClient::new(worker)
        .ready(&session_id)
        .expect("sending ready");
}

fn reap(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

fn wait_for(markers: &[PathBuf]) -> bool {
    for _ in 0..600 {
        if markers.iter().all(|m| m.exists()) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[test]
fn one_process_hosting_three_extensions_costs_less_than_three() {
    if std::fs::read_to_string("/proc/self/smaps_rollup").is_err() {
        println!("skipping: no smaps_rollup, so this is not Linux");
        return;
    }
    let Some(runtime) = runtime() else {
        assert!(
            !require_runtime(),
            "COMPASS_REQUIRE_RUNTIME=1 but the runtime bundle is missing"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };

    // ---- arrangement A: one process per extension, as the host does today --
    let dir_a = tempfile::tempdir().expect("temp dir");
    let mut workers_a = Vec::new();
    let mut markers_a = Vec::new();
    for i in 0..EXTENSIONS {
        let marker = dir_a.path().join(format!("m{i}"));
        let entry = dir_a.path().join(format!("c{i}.js"));
        std::fs::write(&entry, command_source(&marker)).expect("write");
        let mut w = spawn(&runtime);
        load_into(&mut w, &entry, i);
        workers_a.push(w);
        markers_a.push(marker);
    }
    let loaded_a = wait_for(&markers_a);
    let pids_a: Vec<u32> = workers_a.iter().map(Worker::pid).collect();
    let (total_a, private_a, pss_a) = footprint_kb(&pids_a);
    for &pid in &pids_a {
        reap(pid);
    }
    drop(workers_a);

    // ---- arrangement B: one process, three sessions -----------------------
    let dir_b = tempfile::tempdir().expect("temp dir");
    let mut worker_b = spawn(&runtime);
    let mut markers_b = Vec::new();
    for i in 0..EXTENSIONS {
        let marker = dir_b.path().join(format!("m{i}"));
        let entry = dir_b.path().join(format!("c{i}.js"));
        std::fs::write(&entry, command_source(&marker)).expect("write");
        load_into(&mut worker_b, &entry, i);
        markers_b.push(marker);
    }
    let loaded_b = wait_for(&markers_b);
    let pid_b = worker_b.pid();
    let (total_b, private_b, pss_b) = footprint_kb(&[pid_b]);
    reap(pid_b);
    drop(worker_b);

    println!(
        "three processes : {total_a} kB (private {private_a}, pss {pss_a}), all loaded: {loaded_a}"
    );
    println!(
        "one  process    : {total_b} kB (private {private_b}, pss {pss_b}), all loaded: {loaded_b}"
    );
    if total_a > total_b {
        let saved = total_a - total_b;
        let pct = saved * 100 / total_a;
        println!("multiplexing saves {saved} kB ({pct}%)");
    }

    assert!(
        loaded_a && loaded_b,
        "an arrangement did not fully load (three processes: {loaded_a}, one: {loaded_b}); \
         comparing a loaded arrangement against a half-loaded one measures nothing"
    );

    // A floor, so a comparison between two broken measurements cannot pass.
    assert!(
        total_b > 16 * 1024,
        "the single process reported {total_b} kB, too small to be a loaded node runtime"
    );

    assert!(
        total_b <= total_a,
        "multiplexing three extensions into one process cost MORE ({total_b} kB) than three \
         separate processes ({total_a} kB), which contradicts the reason the runtime keeps a \
         session map at all"
    );
}
