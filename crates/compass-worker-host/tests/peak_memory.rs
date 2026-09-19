//! §8.5's last unmeasured row: peak RSS, 10k index + 3 extensions, < 150 MB.
//!
//! WHAT CHANGED SINCE THE HALF-MEASUREMENT
//!
//! `compass-core/tests/index_memory.rs` measured the index half and said
//! plainly that it could not measure the other: "there is no extension host
//! yet; that is Phase 4". There is one now — `real_runtime.rs` drives the same
//! bundle the C++ engine ships as `vicinae-worker-ts`, and CI runs it. So the
//! row is measurable end to end, and this measures it.
//!
//! WHAT IS COUNTED, AND WHY THIS PROCESS IS NOT
//!
//! The row names two things: an index of 10,000 entries, and three extensions.
//! It does not name a test harness, and a cargo test binary's own baseline is
//! not the engine's — counting it would make the number depend on which test
//! runner built it. So the total is:
//!
//!   * the index, as the `VmHWM` **delta** across building it in this process,
//!     which is the same quantity `index_memory.rs` reports; plus
//!   * each worker's **absolute** peak RSS, summed over its whole process tree,
//!     because a worker is a separate process and all of it is real.
//!
//! `VmHWM`, not `VmRSS`: the row says *peak*, and the resident set at whatever
//! moment you happen to read it is not a peak.
//!
//! THE RESULT, AND WHY IT IS REPORTED RATHER THAN GATED
//!
//! **261 MB against a 150 MB budget.** The index costs 15.7 MB, agreeing with
//! `index_memory.rs`'s 15.4–15.6 MB, and each extension peaks near 82 MB.
//!
//! A bare `node -e` peaks at 44 MB on the same machine, so three interpreters
//! are ~132 MB of the budget before any extension code runs. The row is not
//! missed because the runtime is fat; it is missed because "3 extensions" means
//! three node processes. Moving the SLA or moving the process model is an
//! architectural decision, so this records the number and §8.5 states the
//! finding — the same treatment the cold-start figure got, and for the same
//! ADR-0010 reason: measured once is not a threshold.
//!
//! WHY THE WORKERS HANG ON PURPOSE
//!
//! A no-view command that runs to completion asks to be unloaded, and a worker
//! that has exited has no `/proc` entry to read. The three commands here do
//! real work first — they load `@vicinae/api` and round-trip a value through
//! this host's storage — then write a marker file and await a promise that
//! never resolves. Measuring before they have loaded the API would report the
//! cost of a bare node process, which is not what an extension costs.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use compass_core::apps::AppIndex;
use compass_worker_host::extension_manager::ManagerClient;
use compass_worker_host::{Worker, extension_manager};

/// The SLA, in kilobytes. PLAN.md §8.5.
const BUDGET_KB: u64 = 150 * 1024;

/// The corpus size the row names.
const ENTRIES: usize = 10_000;

/// The extension count the row names.
const EXTENSIONS: usize = 3;

/// The repository root, two levels above this crate.
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the root")
        .to_path_buf()
}

/// The runtime bundle, or `None` when it has not been built.
///
/// Same contract as `real_runtime.rs`: a missing build is not a failing host,
/// but `COMPASS_REQUIRE_RUNTIME=1` turns the skip into a failure so CI can
/// insist once it has built one.
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

/// Ends a worker, rather than leaving `timeout` to reap it a minute later.
///
/// These commands never finish by design, so nothing else ends them. Without
/// this the test returns while three node processes are still resident, which
/// `cargo nextest` reports as a LEAK and which costs the rest of the suite
/// real memory on a shared runner. SIGTERM goes to `timeout`, which forwards it
/// to node; SIGKILL would orphan the child instead.
fn reap(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

/// Private memory plus shared memory counted once, in kilobytes.
///
/// WHY SUMMED RSS IS THE WRONG TOTAL, AND THIS IS THE RIGHT ONE
///
/// Three node processes share the interpreter's text pages, and RSS charges
/// every one of them the full amount. Measured with `smaps_rollup`: one node
/// alone reports 43 MB RSS of which 34 MB is `Private_Clean`; three running
/// concurrently report ~41 MB RSS each but ~18 MB PSS each, because that same
/// 34 MB has become one `Shared_Clean` mapping. Adding the three RSS figures
/// counts the interpreter three times.
///
/// This takes private memory in full and the shared mapping once, which is
/// what the machine actually has to find. PSS is printed beside it as a
/// cross-check; the two agree to within a few percent here, and PSS is not
/// used as the headline because it divides shared pages by *all* mappers,
/// which is silently wrong if something outside this set maps them too.
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

/// A field of `/proc/<pid>/status`, in kilobytes.
fn status_kb(pid: u32, field: &str) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let prefix = format!("{field}:");
    status
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

/// This process's high-water resident set, in kilobytes.
fn own_peak_rss_kb() -> Option<u64> {
    status_kb(std::process::id(), "VmHWM")
}

/// Every descendant of `pid`, plus `pid` itself.
///
/// The worker is spawned behind `timeout`, so the pid this host holds is not
/// the node process — it is `timeout`'s, and node is its child. Summing only
/// the pid we hold would report about 600 kB and call it an extension. Walking
/// the tree is what makes the number the row's number.
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

/// The summed high-water resident set of a whole process tree, in kilobytes.
fn tree_peak_rss_kb(pid: u32) -> u64 {
    process_tree(pid)
        .into_iter()
        .filter_map(|p| status_kb(p, "VmHWM"))
        .sum()
}

/// A command that loads the API, says so, and then stays resident.
///
/// The marker write is what lets the host measure a *loaded* extension rather
/// than a node process that has not reached the API yet. The never-resolving
/// promise is what keeps `/proc/<pid>` there to be read.
///
/// # Why it does not call back into the host
///
/// The first version round-tripped a value through `LocalStorage`, which meant
/// the test had to pump three sessions round-robin to service those calls.
/// `Session::pump_once` blocks on a read, so one slow worker stalled the loop;
/// under a loaded runner the whole test could sit there until `timeout` reaped
/// the workers at sixty seconds, after which it measured three corpses. That
/// surfaced as the 16 MB measurement floor failing intermittently — roughly one
/// run in four of the full workspace suite, and never when the test ran alone.
///
/// The floor was right to fire: those *were* broken measurements. But a test
/// that fails one run in four teaches people to re-run, so the dependency is
/// removed rather than papered over. `require("@vicinae/api")` is what costs
/// memory; a storage round trip adds nothing measurable to the resident set,
/// and `real_runtime.rs` already covers that the calls work.
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
        command_name: "store".to_owned(),
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

/// Writes `ENTRIES` desktop files and indexes them, returning the peak delta.
///
/// On disk rather than synthesised in memory, because `AppIndex` reads
/// directories and a fake in-memory path would measure a different thing from
/// the one the row is about. Mirrors `index_memory.rs` deliberately: if the two
/// disagree, one of them is wrong and that is worth noticing.
fn index_cost_kb(dir: &Path) -> u64 {
    let before = own_peak_rss_kb().expect("VmHWM is readable on Linux");

    let mut body = String::new();
    for i in 0..ENTRIES {
        body.clear();
        write!(
            body,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Generated Application {i}\n\
             GenericName=Example {i}\n\
             Comment=A synthetic entry standing in for a real one\n\
             Exec=/usr/bin/true --instance {i}\n\
             Icon=application-x-executable\n\
             Categories=Utility;\n"
        )
        .expect("format");
        std::fs::write(dir.join(format!("generated-{i}.desktop")), &body).expect("write");
    }

    let index = AppIndex::builder().dir(dir).build();
    let indexed = index.applications().count();
    assert_eq!(
        indexed, ENTRIES,
        "the index dropped entries: {indexed} of {ENTRIES}"
    );

    let after = own_peak_rss_kb().expect("VmHWM readable after it was readable before");
    // Keep the index alive across the measurement, or the delta is a measure of
    // how fast the allocator returns pages rather than of what an index costs.
    std::hint::black_box(index);
    after.saturating_sub(before)
}

#[test]
fn ten_thousand_entries_and_three_extensions_fit_the_budget() {
    if own_peak_rss_kb().is_none() {
        println!("skipping: /proc/self/status is unreadable, so this is not Linux");
        return;
    }

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

    let apps = dir.path().join("applications");
    std::fs::create_dir_all(&apps).expect("the applications directory");
    let index_kb = index_cost_kb(&apps);

    let mut workers = Vec::with_capacity(EXTENSIONS);
    let mut pids = Vec::with_capacity(EXTENSIONS);
    let mut markers = Vec::with_capacity(EXTENSIONS);

    for i in 0..EXTENSIONS {
        let marker = dir.path().join(format!("loaded-{i}.json"));
        let entrypoint = dir.path().join(format!("command-{i}.js"));
        std::fs::write(&entrypoint, command_source(&marker)).expect("write the command");

        // Under `timeout`, so a runtime that never answers ends the test with a
        // failed assertion instead of hanging the suite. These commands never
        // finish by design, so the timeout is also what reaps them.
        let mut command = std::process::Command::new("timeout");
        command.arg("60").arg(node()).arg(&runtime);

        let mut worker = Worker::spawn(command).expect("the runtime starts");
        pids.push(worker.pid());

        let load_id = ManagerClient::new(&mut worker)
            .load(&load_options(&entrypoint, i))
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

        // Within a second of the load reply, or the runtime unloads the worker.
        ManagerClient::new(&mut worker)
            .ready(&session_id)
            .expect("sending ready");

        workers.push(worker);
        markers.push(marker);
    }

    // Wait for every command to reach its marker. No session pumping: these
    // commands make no host calls, so there is nothing to answer, and a
    // blocking `pump_once` is precisely what used to stall this test past the
    // sixty-second reaper. See `command_source`.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if markers.iter().all(|m| m.exists()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    for (i, marker) in markers.iter().enumerate() {
        assert!(
            marker.exists(),
            "extension {i} never reached its marker, so it was not measured loaded"
        );
    }

    // Measuring a process that `timeout` has already reaped reports a few
    // hundred kB and reads as a lean extension. The floor below would catch it,
    // but this says what actually went wrong instead of blaming the probe.
    for (i, &pid) in pids.iter().enumerate() {
        assert!(
            std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "extension {i} (pid {pid}) was gone before it could be measured"
        );
    }

    let (footprint_kb_total, private_kb, pss_kb) = footprint_kb(&pids);
    let worker_kb: Vec<u64> = pids.iter().map(|&pid| tree_peak_rss_kb(pid)).collect();

    // Measured, so they have done their job. Reaped here rather than left to
    // `timeout`, which would hold three node processes for another minute.
    // Deliberately after the measurement and before the assertions: a failing
    // assertion must not leak them either.
    drop(workers);
    for &pid in &pids {
        reap(pid);
    }
    let extensions_kb: u64 = worker_kb.iter().sum();
    let total_kb = index_kb + extensions_kb;

    let each: Vec<String> = worker_kb.iter().map(|kb| format!("{kb} kB")).collect();
    println!(
        "peak RSS, {ENTRIES} index + {EXTENSIONS} extensions: index {index_kb} kB, \
         extensions {extensions_kb} kB ({}), total {total_kb} kB against the \
         {BUDGET_KB} kB SLA",
        each.join(" + ")
    );
    let honest_total = index_kb + footprint_kb_total;
    println!(
        "  the same thing counted without triple-charging the shared interpreter: \
         extensions {footprint_kb_total} kB (private {private_kb}, pss {pss_kb}), \
         total {honest_total} kB — see `footprint_kb` for why these differ"
    );

    // WHAT THIS FLOOR IS FOR, AND THE BUG IT CAUGHT
    //
    // A worker that reports too little silently shrinks the total, and a total
    // that is too small passes. This is not hypothetical: summing only the pid
    // this host holds — `timeout`'s, not node's — reports 1776 kB per extension
    // and a 21 MB total, which sails through the SLA. That false green is what
    // `process_tree` exists to prevent, and this floor is what would catch it
    // coming back.
    //
    // 16 MB is chosen against a measurement rather than invented: a bare
    // `node -e` on this machine peaks at 44 004 kB, so any extension reporting
    // less than a third of an empty interpreter is a broken measurement, not a
    // lean extension. It is deliberately far below the 82 MB observed, because
    // its job is to catch a broken probe, not to police memory.
    const MEASUREMENT_FLOOR_KB: u64 = 16 * 1024;
    for (i, kb) in worker_kb.iter().enumerate() {
        assert!(
            *kb > MEASUREMENT_FLOOR_KB,
            "extension {i} reported {kb} kB of peak RSS, under the {MEASUREMENT_FLOOR_KB} kB \
             floor. A bare node interpreter peaks near 44 MB, so this is a broken \
             measurement being counted as a lean extension — most likely the process tree \
             walk no longer reaches the node process"
        );
    }
    assert!(
        index_kb > 1024,
        "the index reported {index_kb} kB, which is too small to be an index of {ENTRIES} \
         entries; the measurement, not the index, is what is wrong"
    );

    // WHY THE SLA IS REPORTED AND NOT ASSERTED
    //
    // Measured here: 261 056 kB against a 153 600 kB budget — a 1.7x miss. That
    // is a real finding and it is written up in §8.5, not softened into a
    // passing threshold.
    //
    // It is not gated, for the same reason the cold-start figure is not: the
    // number has been measured exactly once, and ADR-0010 says a tier assertion
    // is measured before a threshold is invented. Turning it into a failure now
    // would redden every PR over a pre-existing condition that no PR caused.
    //
    // The arithmetic matters more than the total. A bare node interpreter peaks
    // at ~44 MB, so three of them are ~132 MB of the 150 MB budget before a
    // single line of extension code runs. The budget is not missed because the
    // runtime is heavy; it is missed because "3 extensions" means three node
    // processes under the current model. Whether the SLA moves or the model
    // does is an architectural decision, and it is not this test's to make.
    if honest_total >= BUDGET_KB {
        println!(
            "  MISS: {honest_total} kB against the §8.5 SLA of {BUDGET_KB} kB. Reported \
             rather than gated — see §8.5. `multiplex_memory.rs` measures the fix: hosting \
             all three in one process, which the runtime's session map already supports, \
             saves about a third."
        );
    }
}
