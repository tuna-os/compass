//! The parallel ranker must produce the *identical* vector, not an equivalent one.
//!
//! Ranking order is a contract: `rank` promises "total and deterministic —
//! descending score, then ascending input index", and Suite 0 diffs ranked
//! output against the C++ engine. A parallel implementation that returned the
//! same *set* in a different order would pass a careless test and fail parity,
//! so this compares the whole sequence element for element.

use compass_search::{
    FuzzySearchable, Query, RankOptions, WeightedField, rank_indices_sequential,
    rank_indices_with_query_and_options,
};

struct Item {
    name: String,
    generic: String,
    comment: String,
}

impl FuzzySearchable for Item {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(&self.name, 1.0));
        out.push(WeightedField::new(&self.generic, 0.6));
        out.push(WeightedField::new(&self.comment, 0.3));
    }
}

/// A corpus with deliberate score ties, because ties are where an order
/// contract is actually tested. Identical names at different indices must come
/// back in index order from both implementations.
fn corpus() -> Vec<Item> {
    const HEADS: [&str; 12] = [
        "Firefox",
        "Files",
        "Text Editor",
        "Fira Code",
        "System Monitor",
        "Calculator",
        "Terminal",
        "Settings",
        "Éditeur de texte",
        "Файлы",
        "画像ビューア",
        "Disk Usage",
    ];
    let mut out = Vec::new();
    for i in 0..2_000usize {
        let head = HEADS[i % HEADS.len()];
        out.push(Item {
            // Every twelfth item repeats a name exactly, producing real ties.
            name: if i % 12 == 0 {
                head.to_owned()
            } else {
                format!("{head} {i}")
            },
            generic: format!("Example {}", i % 7),
            comment: "A synthetic entry standing in for a real one".to_owned(),
        });
    }
    out
}

#[test]
fn the_parallel_ranker_returns_the_identical_sequence() {
    let items = corpus();
    let options = RankOptions::default();

    // Includes an empty query (a separate code path in both), a pure miss, a
    // broad match, single characters, and non-ASCII needles that exercise
    // transliteration rather than the ASCII path.
    let queries = [
        "",
        "f",
        "fi",
        "fir",
        "fire",
        "firefox",
        "ed",
        "e",
        "text editor",
        "zzq",
        "xyzzy",
        "фай",
        "éditeur",
        "editeur",
        "画像",
        "disk usage",
        "sys mon",
        "  fi  ",
    ];

    let mut compared = 0usize;
    let mut non_empty_results = 0usize;

    for text in queries {
        let query = Query::new(text);
        let sequential = rank_indices_sequential(&query, &items, options);
        let parallel = rank_indices_with_query_and_options(&query, &items, options);

        assert_eq!(
            sequential.len(),
            parallel.len(),
            "query {text:?}: {} results sequentially, {} in parallel",
            sequential.len(),
            parallel.len()
        );

        for (rank, (s, p)) in sequential.iter().zip(&parallel).enumerate() {
            assert_eq!(
                (s.item, s.score, s.quality, s.weighted, s.index),
                (p.item, p.score, p.quality, p.weighted, p.index),
                "query {text:?} diverges at rank {rank}: sequential {s:?}, parallel {p:?}"
            );
        }

        compared += sequential.len();
        if !sequential.is_empty() {
            non_empty_results += 1;
        }
    }

    // A comparison of two empty vectors is not a comparison. Without this, a
    // ranker that returned nothing at all would pass every assertion above —
    // the same silent-control defect this project keeps finding.
    // 9167 with the corpus and queries above. The floor is set below that with
    // room for the ranker to legitimately return somewhat fewer, but far above
    // the zero a broken ranker would produce.
    assert!(
        compared > 5_000,
        "only {compared} ranked entries were compared; the corpus or the queries are not \
         exercising the ranker"
    );
    assert!(
        non_empty_results >= queries.len() - 3,
        "only {non_empty_results} of {} queries matched anything, so most comparisons were \
         between empty vectors",
        queries.len()
    );
}
