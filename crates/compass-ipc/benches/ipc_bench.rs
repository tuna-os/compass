//! IPC round-trip benchmark.
//!
//! Target: p99 < 0.5 ms inside Flatpak (PLAN.md §6 Phase 2 exit gate).
//!
//! WHAT THIS MEASURES, AND WHAT IT USED TO
//!
//! The first version of this file timed a closure that created a Tokio runtime,
//! bound a listener, **slept 10 ms**, connected a client, sent one request and
//! tore the whole thing down. It reported 11.9 ms, and the gate it is written
//! against is 0.5 ms.
//!
//! That number is not a slow round-trip. It is the sleep, plus setup, with the
//! round-trip somewhere inside it — a benchmark that cannot observe the thing
//! its own doc comment names. Nobody had read it against the gate, because
//! 11.9 ms against a 0.5 ms target is not a near miss, it is a 24x miss that
//! would have been reported immediately.
//!
//! The giveaway was already in the old output: `concurrent_1` took 12.03 ms and
//! `concurrent_100` took 13.08 ms, so ninety-nine extra in-flight requests cost
//! about a millisecond between them. The per-request cost was always tiny; the
//! harness was measuring its own scaffolding.
//!
//! Now the runtime, the listener and the connected client are built ONCE,
//! outside the timed region, and each iteration is one request and one
//! response on an established connection. That is what "IPC round-trip" means
//! in the gate.

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::tempdir;
use tokio::runtime::Runtime;

use compass_ipc::{Client, Listener, Request, Response, SocketPath};

/// A running server plus its runtime, kept alive for the whole benchmark.
///
/// Held together so nothing here is inside a timed region: binding a listener
/// and spawning a task are setup costs, and mixing them into the measurement is
/// exactly what made the old numbers meaningless.
struct Harness {
    rt: Runtime,
    _dir: tempfile::TempDir,
    socket: SocketPath,
}

impl Harness {
    fn start() -> Self {
        let rt = Runtime::new().unwrap();
        let dir = tempdir().unwrap();
        let socket = SocketPath::in_dir(dir.path());

        let listen_path = socket.as_path().to_path_buf();
        rt.block_on(async {
            let listener = Listener::bind(&listen_path).await.unwrap();
            tokio::spawn(async move {
                listener
                    .serve(|request| async move {
                        match request {
                            Request::Ping => Response::Pong {
                                protocol_version: 1,
                                pid: std::process::id(),
                            },
                            Request::Query { text: _ } => Response::QueryResults { hits: vec![] },
                            _ => Response::Ack,
                        }
                    })
                    .await
                    .unwrap();
            });
        });

        // Wait for the socket to accept a connection rather than sleeping a
        // plausible number of milliseconds. The old 10 ms sleep was both the
        // largest term in the measurement and, on a loaded machine, not
        // necessarily long enough — the worst of both.
        rt.block_on(async {
            for _ in 0..1000 {
                if Client::connect(socket.as_path()).await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            panic!("the bench server never accepted a connection");
        });

        Self {
            rt,
            _dir: dir,
            socket,
        }
    }

    fn client(&self) -> Client {
        self.rt
            .block_on(async { Client::connect(self.socket.as_path()).await.unwrap() })
    }
}

fn bench_roundtrip(c: &mut Criterion) {
    let harness = Harness::start();
    let mut client = harness.client();

    let mut group = c.benchmark_group("ipc_roundtrip");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(1000);

    // One request, one response, on a connection that is already open. This is
    // the number the Phase 2 gate is about.
    group.bench_function("ping", |b| {
        b.iter(|| {
            let response = harness
                .rt
                .block_on(async { client.request(Request::Ping).await.unwrap() });
            criterion::black_box(response);
        });
    });

    group.finish();
}

fn bench_throughput(c: &mut Criterion) {
    let harness = Harness::start();

    let mut group = c.benchmark_group("ipc_throughput");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(100);

    // Connections are established per iteration here on purpose: this group is
    // about what happens when N clients arrive at once, and connection setup is
    // part of that. It is a different question from the round-trip above, and
    // conflating the two is what the old file did.
    for concurrent in [1, 10, 50, 100] {
        group.bench_with_input(
            format!("concurrent_{concurrent}"),
            &concurrent,
            |b, &concurrent| {
                b.iter(|| {
                    harness.rt.block_on(async {
                        let mut handles = Vec::with_capacity(concurrent);
                        for _ in 0..concurrent {
                            let path = harness.socket.as_path().to_path_buf();
                            handles.push(tokio::spawn(async move {
                                let mut client = Client::connect(&path).await.unwrap();
                                client.request(Request::Ping).await.unwrap()
                            }));
                        }
                        for handle in handles {
                            criterion::black_box(handle.await.unwrap());
                        }
                    });
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_roundtrip, bench_throughput);
criterion_main!(benches);
