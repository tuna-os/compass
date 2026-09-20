//! Suite 4b: warm, persistent IPC comparison. See HEAD-TO-HEAD.md for scope.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use compass_ipc::{Client, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    socket: PathBuf,
    /// Exact source revision and any instrumentation patch, never just "upstream".
    revision: String,
    build: String,
    /// Include independently launched UI/worker roots as well as the daemon.
    process_roots: Vec<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Query {
    text: String,
    /// Independent expected IDs, not the first engine's output. Scores differ.
    expected_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    cpp: Endpoint,
    rust: Endpoint,
    /// Describe package, display/backend, corpus digest, index and extension state.
    workload: String,
    samples: usize,
    warmup: usize,
    /// Empty for unmodified upstream, which does not expose rootQuery.
    queries: Vec<Query>,
}

#[derive(Debug, Serialize)]
struct Distribution {
    median: u64,
    p95: u64,
    p99: u64,
    min: u64,
    max: u64,
    samples: Vec<u64>,
}

fn distribution(samples: Vec<u64>) -> Distribution {
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let percentile = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
    Distribution {
        median: percentile(50),
        p95: percentile(95),
        p99: percentile(99),
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        samples,
    }
}

struct CppClient {
    stream: UnixStream,
    id: u64,
}

impl CppClient {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": self.id, "method": method, "params": params
        }))?;
        // Upstream's LocalSocket writes a native uint32_t, not network order.
        self.stream
            .write_all(&u32::try_from(payload.len())?.to_ne_bytes())
            .await?;
        self.stream.write_all(&payload).await?;
        loop {
            let mut prefix = [0; 4];
            self.stream.read_exact(&mut prefix).await?;
            let size = usize::try_from(u32::from_ne_bytes(prefix))?;
            ensure!(size <= MAX_FRAME, "C++ frame exceeds limit");
            let mut body = vec![0; size];
            self.stream.read_exact(&mut body).await?;
            let response: Value = serde_json::from_slice(&body)?;
            ensure!(response["jsonrpc"] == "2.0", "invalid C++ protocol version");
            if response.get("id").is_none() {
                continue; // Unsolicited notifications do not complete a call.
            }
            ensure!(
                response["id"].as_u64() == Some(self.id),
                "wrong C++ response id"
            );
            ensure!(response.get("error").is_none(), "C++ RPC error: {response}");
            return response
                .get("result")
                .cloned()
                .context("missing C++ result");
        }
    }
}

fn validate_ids(actual: &[String], query: &Query) -> Result<()> {
    ensure!(
        actual == query.expected_ids,
        "query {:?}: expected {:?}, got {:?}",
        query.text,
        query.expected_ids,
        actual
    );
    Ok(())
}

async fn cpp_action(client: &mut CppClient, query: Option<&Query>) -> Result<Vec<String>> {
    if let Some(query) = query {
        let value = client
            .call(
                "Ipc/rootQuery",
                json!({"q": query.text, "params": {
                    "limit": 0, "providerId": "applications", "includeDisabled": false
                }}),
            )
            .await?;
        value
            .as_array()
            .context("C++ query did not return an array")?
            .iter()
            .map(|hit| {
                let id = hit["id"].as_str().context("C++ hit has no ID")?;
                let id = id
                    .strip_prefix("applications:")
                    .context("non-application C++ hit")?;
                Ok(format!("{id}.desktop"))
            })
            .collect()
    } else {
        let reply = client.call("Ipc/ping", json!({})).await?;
        ensure!(reply["pid"].as_u64().is_some(), "invalid C++ pong");
        Ok(Vec::new())
    }
}

async fn rust_action(client: &mut Client, query: Option<&Query>) -> Result<Vec<String>> {
    let request = query.map_or(Request::Ping, |query| Request::Query {
        text: query.text.clone(),
    });
    match (query, client.call(request).await?) {
        (None, Response::Pong { .. }) => Ok(Vec::new()),
        (Some(_), Response::QueryResults { hits }) => {
            Ok(hits.into_iter().map(|hit| hit.id).collect())
        }
        (_, response) => bail!("unexpected Rust response: {response:?}"),
    }
}

async fn sample(
    cpp: &mut CppClient,
    rust: &mut Client,
    is_rust: bool,
    query: Option<&Query>,
) -> Result<u64> {
    let start = Instant::now();
    let ids = tokio::time::timeout(TIMEOUT, async {
        if is_rust {
            rust_action(rust, query).await
        } else {
            cpp_action(cpp, query).await
        }
    })
    .await
    .context("IPC action timed out")??;
    let nanos = u64::try_from(start.elapsed().as_nanos())?;
    // Correctness assertions are outside the timed interval, on every response.
    if let Some(query) = query {
        validate_ids(&ids, query)?;
    }
    Ok(nanos)
}

#[derive(Debug, Serialize)]
struct Memory {
    pids: BTreeSet<u32>,
    rss_kib: u64,
    pss_kib: u64,
}

fn memory(roots: &[u32]) -> Result<Memory> {
    let mut pids: BTreeSet<_> = roots.iter().copied().collect();
    // Scan PPid instead of only /task/<leader>/children: any thread may spawn.
    let mut parents = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let path = entry?.path();
        let Some(pid) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Ok(status) = std::fs::read_to_string(path.join("status"))
            && let Some(parent) = status.lines().find_map(|line| {
                line.strip_prefix("PPid:")
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
        {
            parents.push((pid, parent));
        }
    }
    loop {
        let before = pids.len();
        for &(pid, parent) in &parents {
            if pids.contains(&parent) {
                pids.insert(pid);
            }
        }
        if before == pids.len() {
            break;
        }
    }
    let mut result = Memory {
        pids,
        rss_kib: 0,
        pss_kib: 0,
    };
    for pid in &result.pids {
        let path = format!("/proc/{pid}/smaps_rollup");
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("read {path}; missing processes are not zero memory"))?;
        let field = |key: &str| -> Result<u64> {
            contents
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|rest| rest.split_whitespace().next())
                .context("missing memory field")?
                .parse()
                .context("invalid memory field")
        };
        result.rss_kib += field("Rss:")?;
        result.pss_kib += field("Pss:")?;
    }
    ensure!(
        result.rss_kib > 0 && result.pss_kib > 0,
        "zero memory is not a measurement"
    );
    Ok(result)
}

fn validate(config: &Config) -> Result<()> {
    ensure!(
        config.samples >= 100 && config.samples <= 1_000_000,
        "samples must be 100..=1000000"
    );
    ensure!(config.warmup > 0, "warmup is required");
    ensure!(config.warmup <= 1_000_000, "warmup must not exceed 1000000");
    ensure!(!config.workload.trim().is_empty(), "describe the workload");
    for endpoint in [&config.cpp, &config.rust] {
        ensure!(
            !endpoint.revision.trim().is_empty() && !endpoint.build.trim().is_empty(),
            "revision and build provenance required"
        );
        ensure!(
            !endpoint.process_roots.is_empty(),
            "memory process roots required"
        );
    }
    if !config.queries.is_empty() {
        ensure!(
            config
                .queries
                .iter()
                .any(|query| !query.expected_ids.is_empty()),
            "queries need a positive control; two empty indexes must not pass"
        );
    }
    Ok(())
}

async fn run(config: Config) -> Result<Value> {
    validate(&config)?;
    let mut cpp = CppClient {
        stream: UnixStream::connect(&config.cpp.socket).await?,
        id: 0,
    };
    let mut rust = Client::connect(&config.rust.socket).await?;
    let cpp_ping = tokio::time::timeout(TIMEOUT, cpp.call("Ipc/ping", json!({}))).await??;
    let Response::Pong { pid: rust_pid, .. } =
        tokio::time::timeout(TIMEOUT, rust.call(Request::Ping)).await??
    else {
        bail!("Rust did not pong")
    };
    let cpp_pid = u32::try_from(cpp_ping["pid"].as_u64().context("missing C++ pid")?)?;
    ensure!(
        config.cpp.process_roots.contains(&cpp_pid),
        "C++ process roots omit the responding engine"
    );
    ensure!(
        config.rust.process_roots.contains(&rust_pid),
        "Rust process roots omit the responding engine"
    );
    let mut rows = Vec::with_capacity(config.queries.len() + 1);
    for query in std::iter::once(None).chain(config.queries.iter().map(Some)) {
        let mut cpp_samples = Vec::with_capacity(config.samples);
        let mut rust_samples = Vec::with_capacity(config.samples);
        for i in 0..config.warmup + config.samples {
            // AB/BA pairs balance both drift and the first/second position.
            for is_rust in [i % 2 == 1, i % 2 == 0] {
                let ns = sample(&mut cpp, &mut rust, is_rust, query).await?;
                if i >= config.warmup {
                    if is_rust {
                        rust_samples.push(ns);
                    } else {
                        cpp_samples.push(ns);
                    }
                }
            }
        }
        let cpp_stats = distribution(cpp_samples);
        let rust_stats = distribution(rust_samples);
        rows.push(
            json!({"action": query.map_or("ping", |_| "query"), "query": query.map(|q| &q.text),
            "unit": "ns", "cpp": cpp_stats, "rust": rust_stats}),
        );
    }
    let mut cpp_memory = Vec::with_capacity(20);
    let mut rust_memory = Vec::with_capacity(20);
    for i in 0..20 {
        let (a, b) = if i % 2 == 0 {
            (
                memory(&config.cpp.process_roots)?,
                memory(&config.rust.process_roots)?,
            )
        } else {
            let b = memory(&config.rust.process_roots)?;
            (memory(&config.cpp.process_roots)?, b)
        };
        ensure!(
            a.pids.is_disjoint(&b.pids),
            "process trees overlap; comparison would double-count a shared engine"
        );
        cpp_memory.push(a);
        rust_memory.push(b);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(json!({"schema": 1, "config": config, "cpp_ping": cpp_ping,
        "latency": rows, "memory": {"unit": "KiB", "cpp": cpp_memory, "rust": rust_memory},
        "timing_scope": "persistent IPC: host encode, engine execution, reply decode; no connect or CLI spawn",
        "memory_scope": "declared process roots plus descendants; RSS sums double-count shared pages; PSS apportions them",
        "exclusions": ["No overall speed or memory-efficiency verdict until equivalent feature workloads are verified",
            "Query is excluded when queries is empty; pristine upstream has no rootQuery",
            "No launch, window reply, first-frame, peak-memory or cold-start measurement"],
        "environment": {"kernel": std::fs::read_to_string("/proc/version")?,
            "cpu": std::fs::read_to_string("/proc/cpuinfo")?,
            "loadavg": std::fs::read_to_string("/proc/loadavg")?}}))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: head-to-head CONFIG.json REPORT.json"
    );
    let config: Config = serde_json::from_slice(&std::fs::read(Path::new(&args[0]))?)?;
    let report = run(config).await?;
    // create_new prevents overwriting a recorded baseline by accident.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(&args[1]))?;
    serde_json::to_writer_pretty(file, &report)?;
    for row in report["latency"].as_array().context("missing rows")? {
        println!(
            "{} {}: C++ median {} ns, Rust median {} ns (tails and samples in report)",
            row["action"], row["query"], row["cpp"]["median"], row["rust"]["median"]
        );
    }
    println!(
        "Recorded raw RSS/PSS samples. No full-port performance verdict; see report exclusions."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_keep_raw_order_and_report_tails() {
        let stats = distribution((1..=100).rev().collect());
        assert_eq!((stats.median, stats.p95, stats.p99), (50, 95, 99));
        assert_eq!(stats.samples[0], 100);
    }

    #[test]
    fn empty_wrong_or_reordered_results_fail_positive_control() {
        let query = Query {
            text: "app".into(),
            expected_ids: vec!["a.desktop".into(), "b.desktop".into()],
        };
        assert!(validate_ids(&[], &query).is_err());
        assert!(validate_ids(&["b.desktop".into(), "a.desktop".into()], &query).is_err());
        assert!(validate_ids(&query.expected_ids, &query).is_ok());
    }

    #[test]
    fn memory_reads_real_process_and_rejects_missing_one() {
        let reading = memory(&[std::process::id()]).expect("own memory");
        assert!(reading.rss_kib >= reading.pss_kib);
        assert!(memory(&[u32::MAX]).is_err());
    }

    #[tokio::test]
    async fn cpp_protocol_handles_fragmented_reply_and_notification() {
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        let responder = tokio::spawn(async move {
            let mut prefix = [0; 4];
            server.read_exact(&mut prefix).await.expect("request size");
            let mut request = vec![0; u32::from_ne_bytes(prefix) as usize];
            server.read_exact(&mut request).await.expect("request");
            let request: Value = serde_json::from_slice(&request).expect("JSON");
            assert_eq!(request["method"], "Ipc/ping");
            assert_eq!(request["params"], json!({}));
            for response in [
                json!({"jsonrpc":"2.0","method":"notice","params":{}}),
                json!({"jsonrpc":"2.0","id":request["id"],"result":{"pid":42,"version":"test"}}),
            ] {
                let payload = serde_json::to_vec(&response).expect("encode");
                server
                    .write_all(&u32::try_from(payload.len()).expect("length").to_ne_bytes())
                    .await
                    .expect("prefix");
                for byte in payload {
                    server.write_all(&[byte]).await.expect("fragment");
                }
            }
        });
        let mut client = CppClient {
            stream: client,
            id: 0,
        };
        assert_eq!(
            client.call("Ipc/ping", json!({})).await.expect("pong")["pid"],
            42
        );
        responder.await.expect("responder");
    }

    #[tokio::test]
    async fn cpp_protocol_rejects_rpc_error_wrong_id_and_oversized_frame() {
        for response in [
            json!({"jsonrpc":"2.0","id":1,"error":"unsupported"}),
            json!({"jsonrpc":"2.0","id":999,"result":{}}),
            json!(null),
        ] {
            let (client, mut server) = UnixStream::pair().expect("socket pair");
            let payload = serde_json::to_vec(&response).expect("encode");
            let size = if response.is_null() {
                u32::try_from(MAX_FRAME + 1).expect("limit")
            } else {
                u32::try_from(payload.len()).expect("length")
            };
            server.write_all(&size.to_ne_bytes()).await.expect("prefix");
            server.write_all(&payload).await.expect("body");
            let mut client = CppClient {
                stream: client,
                id: 0,
            };
            assert!(client.call("Ipc/ping", json!({})).await.is_err());
        }
    }

    #[tokio::test]
    async fn rust_protocol_uses_real_persistent_transport() {
        let dir = tempfile::tempdir().expect("directory");
        let path = dir.path().join("engine.sock");
        let listener = compass_ipc::Listener::bind(&path).await.expect("bind");
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.expect("accept");
            compass_ipc::serve_connection(stream, |request| async move {
                assert_eq!(request, Request::Ping);
                Response::Pong {
                    pid: 42,
                    protocol_version: compass_ipc::PROTOCOL_VERSION,
                }
            })
            .await
            .expect("serve");
        });
        let mut client = Client::connect(&path).await.expect("connect");
        for _ in 0..3 {
            assert!(
                rust_action(&mut client, None)
                    .await
                    .expect("ping")
                    .is_empty()
            );
        }
        drop(client);
        server.await.expect("server");
    }
}
