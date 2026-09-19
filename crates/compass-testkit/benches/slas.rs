//! `cargo bench --bench slas` — the target PLAN.md §8.7 has always named.
//!
//! WHY THIS TARGET EXISTS, AND WHAT IT IS NOT
//!
//! §8.7's pre-flight command has invoked `cargo bench --bench slas
//! -- --save-baseline pr` since it was written, against a target that did not
//! exist: the command failed with "no bench target named `slas`" for anyone who
//! ran the documented sequence. That is the same class of defect as Suite 0's
//! `--engine=cpp --json query` and the C++ `doctor` — an interface documented
//! into existence and never built. This file closes it.
//!
//! It is deliberately NOT the gate. §8.5 asks for "SLAs enforced as CI
//! failures, not advisory numbers", and a criterion bench prints a number and
//! exits zero whatever the number says, which is precisely how a 24x miss sat
//! unnoticed in `compass-ipc`'s bench. The enforcement therefore lives in tests,
//! which CI already runs on every PR:
//!
//!   * fuzzy rank  — `compass-search/tests/ranking_budget.rs`
//!   * IPC         — `compass-ipc`'s round-trip assertions
//!
//! What a bench gives that a test cannot is a *baseline*: `--save-baseline pr`
//! against `--baseline main` turns "is this over the line" into "did this
//! change move", which is the only way to see a 4% regression that never
//! crosses a threshold. That is the job here, and the reason the two rows are
//! gathered into one target rather than left in their own crates: a trend is
//! only comparable if every measurement in it ran under the same harness.
//!
//! ONLY THE ROWS THAT ARE HONESTLY MEASURABLE HERE
//!
//! §8.5 has six rows. Four of them cannot be measured in-process and are not
//! faked into this file:
//!
//!   * cold start to first frame  — needs a display; the VM tier reports a
//!     floor (`first_draw_ms`), see §8.5
//!   * summon to first frame      — needs a compositor and a portal activation
//!   * idle RSS                   — a property of the shipped process inside
//!     the Flatpak, not of a bench binary
//!   * peak RSS with extensions   — the same, plus a worker host
//!
//! A bench that produced a number for any of those would be measuring this
//! harness, which is the mistake this file's siblings were written to undo.

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::tempdir;
use tokio::runtime::Runtime;

use compass_ipc::{Client, Listener, Request, Response, SocketPath};
use compass_search::{Query, rank_with_query};

/// The corpus size the fuzzy row names.
const ITEMS: usize = 10_000;

/// How many results the fuzzy row asks for.
const TOP_N: usize = 20;

/// Ten thousand plausible application names.
///
/// Mirrors `ranking_budget.rs` on purpose: the bench and the gate must rank the
/// same haystack, or the baseline this target exists to produce would not be
/// comparable with the threshold the test enforces. Generated rather than
/// harvested because the SLA is about the *size* of the haystack and the real
/// corpus is 757 entries; varied rather than repeated because a haystack of
/// identical strings lets the matcher behave in a way no real index does.
fn haystack() -> Vec<String> {
    const HEADS: [&str; 20] = [
        "Firefox",
        "Files",
        "Text Editor",
        "System Monitor",
        "Calculator",
        "Terminal",
        "Settings",
        "Image Viewer",
        "Video Player",
        "Music",
        "Disk Usage",
        "Screenshot",
        "Archive Manager",
        "Document Viewer",
        "Password Manager",
        "Mail",
        "Calendar",
        "Contacts",
        "Weather",
        "Maps",
    ];
    const TAILS: [&str; 10] = [
        "",
        " Nightly",
        " Devel",
        " (Wayland)",
        " Preferences",
        " Beta",
        " Classic",
        " Extended",
        " Lite",
        " Pro",
    ];

    let mut out = Vec::with_capacity(ITEMS);
    let mut i = 0usize;
    while out.len() < ITEMS {
        let head = HEADS[i % HEADS.len()];
        let tail = TAILS[(i / HEADS.len()) % TAILS.len()];
        out.push(format!("{head}{tail} {i}"));
        i += 1;
    }
    out
}

/// §8.5 row 1: fuzzy search, top-20 of 10,000 items.
fn bench_fuzzy_rank(c: &mut Criterion) {
    let items = haystack();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    assert_eq!(refs.len(), ITEMS);

    // A query that matches broadly, so ranking has real work to do. One that
    // matched nothing would exit early and measure the wrong thing.
    let query = Query::new("ed");
    let warm = rank_with_query(&query, &refs);
    assert!(
        warm.len() > TOP_N,
        "the bench query matched {} items; it must match more than the top {TOP_N} or this \
         measures an early exit rather than ranking",
        warm.len()
    );

    let mut group = c.benchmark_group("slas");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(200);

    group.bench_function("fuzzy_rank_top20_of_10k", |b| {
        b.iter(|| {
            let ranked = rank_with_query(&query, &refs);
            let top: Vec<_> = ranked.into_iter().take(TOP_N).collect();
            criterion::black_box(top);
        });
    });

    group.finish();
}

/// A running server plus its runtime, kept alive for the whole benchmark.
///
/// Held together so nothing here lands in a timed region. Binding a listener
/// and spawning a task are setup costs, and mixing them into the measurement is
/// exactly what made `compass-ipc`'s first numbers meaningless.
struct Ipc {
    rt: Runtime,
    _dir: tempfile::TempDir,
    socket: SocketPath,
}

impl Ipc {
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
                            _ => Response::Ack,
                        }
                    })
                    .await
                    .unwrap();
            });
        });

        // Wait for the socket to accept rather than sleeping a plausible number
        // of milliseconds: a sleep is both the largest term in the measurement
        // and, on a loaded machine, not necessarily long enough.
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

/// §8.5 row 2: IPC round-trip on a local UDS.
fn bench_ipc_roundtrip(c: &mut Criterion) {
    let ipc = Ipc::start();
    let mut client = ipc.client();

    let mut group = c.benchmark_group("slas");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(1000);

    // One request, one response, on a connection that is already open. That is
    // what "IPC round-trip" means in the row; connection setup is a different
    // question and belongs in `compass-ipc`'s throughput group, not here.
    group.bench_function("ipc_roundtrip_ping", |b| {
        b.iter(|| {
            let response = ipc
                .rt
                .block_on(async { client.request(Request::Ping).await.unwrap() });
            criterion::black_box(response);
        });
    });

    group.finish();
}

criterion_group!(slas, bench_fuzzy_rank, bench_ipc_roundtrip);
criterion_main!(slas);
