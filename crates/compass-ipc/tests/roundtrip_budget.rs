//! The Phase 2 exit gate: IPC round-trip p99 < 0.5 ms (PLAN.md §6, §8.5).
//!
//! WHY A TEST AND NOT JUST THE BENCHMARK
//!
//! `benches/ipc_bench.rs` measures this properly now, but a criterion bench
//! reports a number and exits zero whatever it says. That is how the gate went
//! unnoticed: the previous benchmark timed its own setup — runtime creation, a
//! listener bind, a 10 ms sleep, connect, one request, teardown — and printed
//! 11.9 ms against a 0.5 ms target. A 24x miss on a stated gate should have
//! been impossible to miss, and it was invisible because nothing asserted it.
//!
//! So the budget is asserted here, where a regression fails a run.
//!
//! WHY 500 µs IS A SAFE BOUND TO ASSERT
//!
//! Measured p99 on this codebase is around 48 µs, so the gate sits about ten
//! times above what the code actually does. That headroom is what makes it
//! reasonable to assert on a shared CI runner: a stall large enough to fail
//! this is a 10x degradation, which is worth a red build rather than a shrug.
//!
//! If this does start flaking without a real regression, raise the sample count
//! before raising the bound — the bound is the gate, and moving it silently
//! changes what the project promises.

use std::time::{Duration, Instant};

use compass_ipc::{Client, Listener, Request, Response, SocketPath};
use tempfile::tempdir;

/// The gate, in microseconds. PLAN.md §6 Phase 2.
const BUDGET_US: u128 = 500;

/// Enough samples for a p99 to mean something; 1000 keeps the test well under
/// a second at ~30 µs per round trip.
const SAMPLES: usize = 1000;

#[tokio::test]
async fn roundtrip_p99_is_within_the_phase_2_budget() {
    let dir = tempdir().expect("temp dir");
    let socket = SocketPath::in_dir(dir.path());

    let listen_path = socket.as_path().to_path_buf();
    let listener = Listener::bind(&listen_path).await.expect("bind");
    let server = tokio::spawn(async move {
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
    });

    // Wait for a state, not a duration: the old benchmark's 10 ms sleep was
    // both the largest term in its measurement and no guarantee the listener
    // was up.
    let mut client = None;
    for _ in 0..1000 {
        if let Ok(c) = Client::connect(socket.as_path()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mut client = client.expect("the server never accepted a connection");

    // Warm the connection so the first-request cost of any lazy allocation
    // does not land in the sample set as a fake tail.
    for _ in 0..50 {
        client.request(Request::Ping).await.expect("warmup");
    }

    let mut timings: Vec<u128> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let response = client.request(Request::Ping).await.expect("request");
        timings.push(start.elapsed().as_micros());
        assert!(
            matches!(response, Response::Pong { .. }),
            "the server answered something other than Pong"
        );
    }

    timings.sort_unstable();
    let p50 = timings[SAMPLES / 2];
    let p99 = timings[SAMPLES * 99 / 100];
    let max = timings[SAMPLES - 1];

    // Printed whether or not it passes: a budget with no visible number turns
    // into a number nobody knows, which is how this one drifted.
    println!("IPC round-trip over {SAMPLES} samples: p50 {p50} µs, p99 {p99} µs, max {max} µs");

    assert!(
        p99 < BUDGET_US,
        "IPC round-trip p99 is {p99} µs, over the Phase 2 budget of {BUDGET_US} µs \
         (p50 {p50} µs, max {max} µs). This is the §6 exit gate."
    );

    server.abort();
}
