//! The §8.5 SLA: fuzzy search, top-20 of 10,000 items, p99 < 2.0 ms.
//!
//! WHY THIS IS A TEST AND NOT A BENCHMARK
//!
//! §8.5 introduces its table as "criterion benches with SLAs enforced as CI
//! failures, not advisory numbers". At the time of writing this file, that was
//! not true of any row:
//!
//!   * the workspace contained exactly ONE benchmark, `compass-ipc`'s, and it
//!     timed its own setup — a runtime, a listener bind and a 10 ms sleep —
//!     reporting 11.9 ms against a 0.5 ms budget;
//!   * no CI job ran `cargo bench` at all, so no bench could fail anything;
//!   * §8.7's pre-flight command invokes `cargo bench --bench slas`, a target
//!     that does not exist.
//!
//! A criterion bench prints a number and exits zero whatever the number says.
//! That is fine for tracking a trend and useless as a gate, which is how a 24x
//! miss sat unnoticed. Tests, by contrast, already run in CI on every PR.
//!
//! So SLAs that can be checked without a display, a sandbox or a real session
//! are asserted here, where crossing them fails a build.

use std::time::Instant;

use compass_search::{Query, rank_with_query};

/// The SLA, in microseconds. PLAN.md §8.5.
const BUDGET_US: u128 = 2_000;

/// What to allow when the code is not optimised.
///
/// The SLA is a property of a release build and nothing else. Measured here:
/// **p99 1411 µs in release, 26 879 µs in debug** — nineteen times slower, which
/// is ordinary for tight matcher loops without optimisation and says nothing
/// about the shipped binary.
///
/// So the real budget is asserted only when optimised. Debug runs still assert
/// something, because a test that silently passes in the profile CI happens to
/// use is not a test — it is the shape of every check this project has had to
/// fix. The debug ceiling is deliberately loose: it catches a catastrophic
/// regression without pretending to measure the SLA.
const DEBUG_CEILING_US: u128 = 60_000;

/// The corpus size the SLA names.
const ITEMS: usize = 10_000;

/// Enough repeats for a p99 to be a p99.
///
/// The first version used 100, which makes `timings[SAMPLES * 99 / 100]` the
/// **maximum** — the single worst sample of the run, dressed up as a
/// percentile. Two consecutive release runs then reported 1411 µs and 2800 µs
/// against a 2000 µs budget, which looks like a flaky SLA and was really a
/// flaky statistic: with 100 samples nothing sits above the "p99", so one
/// scheduler hiccup is the whole number.
///
/// A thousand samples puts ten above the p99, which is what makes it robust to
/// exactly that. Debug runs use far fewer because they only check a loose
/// ceiling and 1000 unoptimised iterations is twenty-five seconds.
const SAMPLES: usize = if cfg!(debug_assertions) { 30 } else { 1_000 };

/// Ten thousand plausible application names.
///
/// Generated rather than harvested: the SLA is about the *size* of the haystack
/// and the real corpus is 757 entries. Names are varied deliberately — a
/// haystack of identical strings would let the matcher's internals behave in a
/// way no real index does.
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

#[test]
fn top_20_of_10k_is_within_the_sla() {
    let items = haystack();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    assert_eq!(refs.len(), ITEMS);

    // A query that matches broadly, so the ranking has real work to do. A query
    // matching nothing would exit early and measure the wrong thing — the same
    // mistake as timing a benchmark's own setup.
    let query = Query::new("ed");

    // Warm once so first-call allocation does not land in the sample set.
    let warm = rank_with_query(&query, &refs);
    assert!(
        warm.len() > 20,
        "the query matched {} items; it needs to match more than the top 20 for this \
         measurement to be about ranking rather than about an early exit",
        warm.len()
    );

    let mut timings: Vec<u128> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let ranked = rank_with_query(&query, &refs);
        let top: Vec<_> = ranked.into_iter().take(20).collect();
        timings.push(start.elapsed().as_micros());
        std::hint::black_box(top);
    }

    timings.sort_unstable();
    let p50 = timings[SAMPLES / 2];
    let p99 = timings[SAMPLES * 99 / 100];
    let max = timings[SAMPLES - 1];

    println!(
        "fuzzy rank, top 20 of {ITEMS}, over {SAMPLES} samples: p50 {p50} µs, p99 {p99} µs, max {max} µs"
    );

    // WHICH PERCENTILE THE SLA MEANS IS NOT WRITTEN DOWN, and it matters here in
    // a way it did not for IPC.
    //
    // WHICH PERCENTILE THE SLA MEANS — the question this used to leave open
    //
    // §8.5 says "Fuzzy search, top-20 of 10,000 items — < 2.0 ms" without naming
    // a statistic, and for a long time the answer mattered. Five release runs
    // against the single-threaded ranker:
    //
    //   p50   1209-1242 µs   comfortably inside, and stable across runs
    //   p99   1589-2548 µs   OVER the budget in two runs of five
    //   max   2277-2654 µs
    //
    // So this asserted the median and only reported the tail: gating a number
    // that failed two runs in five teaches people to re-run until it passes,
    // which is worse than no gate.
    //
    // Ranking is now scored across rayon's pool, and the question is moot. Five
    // runs of the same shape:
    //
    //   p50    790-806 µs    ~2.5x inside
    //   p99   1116-1237 µs   ~1.6x inside, over the budget in NONE of five
    //   max   2325-6044 µs   noisier than before, and deliberately not asserted
    //
    // So the tail is now gated rather than reported, because it can be. The
    // stricter reading of the row is the one that holds, which makes arguing
    // about the missing word unnecessary.
    //
    // MAX IS STILL NOT ASSERTED, and is now *noisier* than it was. A worker
    // pool trades a tighter p99 for a longer tail: a sample that happens to
    // land while the pool is waking costs milliseconds. That is a property of
    // scheduling, not of ranking, and asserting a single worst sample out of a
    // thousand would reintroduce exactly the flaky gate this comment argues
    // against — see the SAMPLES doc above for the same mistake made with a
    // 100-sample "p99".
    if cfg!(debug_assertions) {
        println!(
            "  (debug build: the §8.5 SLA of {BUDGET_US} µs is a release property and is \
             asserted there; this run only checks the {DEBUG_CEILING_US} µs sanity ceiling)"
        );
        assert!(
            p99 < DEBUG_CEILING_US,
            "fuzzy search p99 is {p99} µs even allowing for an unoptimised build, over the \
             {DEBUG_CEILING_US} µs debug ceiling (p50 {p50} µs, max {max} µs). The §8.5 SLA is \
             {BUDGET_US} µs in release."
        );
        return;
    }

    assert!(
        p50 < BUDGET_US,
        "fuzzy search median is {p50} µs, over the §8.5 SLA of {BUDGET_US} µs \
         (p99 {p99} µs, max {max} µs) for the top 20 of {ITEMS} items"
    );

    assert!(
        p99 < BUDGET_US,
        "fuzzy search p99 is {p99} µs, over the §8.5 SLA of {BUDGET_US} µs \
         (p50 {p50} µs, max {max} µs) for the top 20 of {ITEMS} items. This gate was \
         tightened from the median to the tail once the parallel ranker put p99 at \
         1116-1237 µs across five runs; a p99 back over 2000 µs means that headroom is gone"
    );
}
