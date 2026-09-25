//! Compass's fuzzy ranking, timed the way `scripts/bench/fuzzy/cpp_rank.cpp`
//! times upstream Vicinae's.
//!
//! `fuzzy-throughput HAYSTACK ITERATIONS QUERY...` prints, per query, one
//! tab-separated line for each of two rankers:
//!
//! * `mirror` — the same shape as the C++ bench: `score_weighted` over every
//!   line on one thread, keep accepted matches, stable-sort by score, take 20.
//!   This isolates the scorer.
//! * `shipped` — `rank_with_query`, what the engine actually calls, which
//!   scores across rayon's pool. Set `RAYON_NUM_THREADS=1` to pin it to one
//!   core.
//!
//! Columns: ranker, query, matches, median_ns, p95_ns. Used by
//! `scripts/bench/compare.sh`; see `docs/rust-engine/BENCHMARKS.md`.

use std::hint::black_box;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use compass_search::{Query, WeightedField, rank_with_query, score_weighted};

/// Matches `fuzzy::MIN_QUALITY` in the C++ scorer, as `scorer-parity` does.
const MIN_QUALITY: u32 = 60;
const TOP_N: usize = 20;
const WARMUP: usize = 10;

fn mirror(items: &[&str], query: &Query) -> usize {
    let mut hits: Vec<(usize, u32)> = Vec::with_capacity(items.len());
    for (index, text) in items.iter().enumerate() {
        let m = score_weighted(&[WeightedField::new(text, 1.0)], query);
        if m.quality >= MIN_QUALITY {
            hits.push((index, m.score));
        }
    }
    hits.sort_by(|a, b| b.1.cmp(&a.1));
    let matched = hits.len();
    hits.truncate(TOP_N);
    black_box(hits);
    matched
}

fn shipped(items: &[&str], query: &Query) -> usize {
    let ranked = rank_with_query(query, items);
    let matched = ranked.len();
    let top: Vec<_> = ranked.into_iter().take(TOP_N).collect();
    black_box(top);
    matched
}

fn time(iterations: usize, mut run: impl FnMut() -> usize) -> (usize, u128, u128) {
    let matched = run();
    for _ in 0..WARMUP {
        run();
    }
    let mut samples: Vec<u128> = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        run();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    (
        matched,
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [haystack, iterations, queries @ ..] = args.as_slice() else {
        bail!("usage: fuzzy-throughput HAYSTACK ITERATIONS QUERY...");
    };
    if queries.is_empty() {
        bail!("usage: fuzzy-throughput HAYSTACK ITERATIONS QUERY...");
    }
    let text = std::fs::read_to_string(haystack).with_context(|| format!("reading {haystack}"))?;
    let items: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    let iterations: usize = iterations.parse().context("ITERATIONS")?;
    if iterations == 0 {
        bail!("ITERATIONS must be positive");
    }
    for raw in queries {
        let query = Query::new(raw);
        for (name, ranker) in [
            ("mirror", mirror as fn(&[&str], &Query) -> usize),
            ("shipped", shipped),
        ] {
            let (matched, median, p95) = time(iterations, || ranker(&items, &query));
            println!("{name}\t{raw}\t{matched}\t{median}\t{p95}");
        }
    }
    Ok(())
}
