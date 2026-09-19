//! Ranking experiments: what the §8.5 fuzzy row actually costs, and what moves it.
//!
//! Two haystack shapes, because they are not the same measurement:
//!
//!   * `plain` — `&str` items, one weighted field each. This is the shape
//!     `ranking_budget.rs` measures and the shape the §8.5 row is quoted
//!     against.
//!   * `rich` — five fields per item (name, app name, generic name, keyword,
//!     comment), which is what `AppItem::fuzzy_fields` actually emits. Real
//!     ranking does roughly five times the matcher work the SLA bench does.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use compass_search::{
    FuzzySearchable, Query, RankOptions, WeightedField, rank_indices_sequential, rank_with_query,
};

const ITEMS: usize = 10_000;

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

fn names() -> Vec<String> {
    let mut out = Vec::with_capacity(ITEMS);
    let mut i = 0usize;
    while out.len() < ITEMS {
        out.push(format!(
            "{}{} {i}",
            HEADS[i % HEADS.len()],
            TAILS[(i / HEADS.len()) % TAILS.len()]
        ));
        i += 1;
    }
    out
}

/// An item with the field shape `AppItem` really has.
struct Rich {
    name: String,
    app_name: String,
    generic: String,
    keyword: String,
    comment: String,
}

impl FuzzySearchable for Rich {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(&self.name, 1.0));
        out.push(WeightedField::new(&self.app_name, 0.8));
        out.push(WeightedField::new(&self.generic, 0.6));
        out.push(WeightedField::new(&self.keyword, 0.5));
        out.push(WeightedField::new(&self.comment, 0.3));
    }
}

fn rich() -> Vec<Rich> {
    names()
        .into_iter()
        .enumerate()
        .map(|(i, name)| Rich {
            app_name: HEADS[i % HEADS.len()].to_owned(),
            generic: format!("Example {i}"),
            keyword: "utility".to_owned(),
            comment: "A synthetic entry standing in for a real one".to_owned(),
            name,
        })
        .collect()
}

/// Does the pool pay for itself on a corpus the size users actually have?
///
/// 10,000 is the SLA's number, not a desktop's. A Bluefin image harvests 757
/// entries and a typical install is a few hundred, so a parallel ranker that
/// only wins at 10k would be the wrong trade for the real case.
fn bench_sizes(c: &mut Criterion) {
    let query = Query::new("fi");
    let mut group = c.benchmark_group("corpus_size");
    group.sample_size(100);

    for size in [200usize, 757, 2_000, 10_000] {
        let all = rich();
        let items = &all[..size.min(all.len())];

        group.bench_function(format!("seq/{size}"), |b| {
            b.iter(|| {
                black_box(rank_indices_sequential(&query, items, RankOptions::default()).len())
            });
        });
        group.bench_function(format!("par/{size}"), |b| {
            b.iter(|| black_box(rank_with_query(&query, items).len()));
        });
    }

    group.finish();
}

fn bench(c: &mut Criterion) {
    let owned = names();
    let plain: Vec<&str> = owned.iter().map(String::as_str).collect();
    let rich_items = rich();

    // Three query shapes, because the cost is not the same:
    //   "ed"  matches broadly — the shape ranking_budget.rs uses
    //   "fire" narrower, still thousands of candidates
    //   "zzq"  matches nothing — the pure rejection path a prefilter targets
    for (label, text) in [
        ("broad_ed", "ed"),
        ("narrow_fire", "fire"),
        ("miss_zzq", "zzq"),
    ] {
        let query = Query::new(text);

        let mut group = c.benchmark_group("ranking");
        group.sample_size(100);

        group.bench_function(format!("plain/{label}"), |b| {
            b.iter(|| {
                black_box(rank_indices_sequential(&query, &plain, RankOptions::default()).len())
            });
        });
        group.bench_function(format!("rich/{label}"), |b| {
            b.iter(|| {
                black_box(
                    rank_indices_sequential(&query, &rich_items, RankOptions::default()).len(),
                )
            });
        });
        group.bench_function(format!("plain_par/{label}"), |b| {
            b.iter(|| black_box(rank_with_query(&query, &plain).len()));
        });
        group.bench_function(format!("rich_par/{label}"), |b| {
            b.iter(|| black_box(rank_with_query(&query, &rich_items).len()));
        });

        group.finish();
    }
}

criterion_group!(experiments, bench, bench_sizes);
criterion_main!(experiments);
