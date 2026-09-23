//! The file-index query engine: `query` and its spellfix fallback, over a read
//! trait.
//!
//! Ports `FileIndexerQueryEngine::query` and the anonymous-namespace
//! `queryWithCorrections` from `file-indexer-query-engine.cpp`. The scoring,
//! sort, and correction math live in [`crate::query_engine`], the query
//! shaping in [`crate::query_policy`]; this module is the orchestration
//! between them and the database: strict candidates first, skeleton
//! candidates merged in when the strict ranking is thin, spellfix
//! corrections when nothing ranks — or when nothing comes back at all.
//!
//! The database arrives as [`IndexReader`], mirroring how [`crate::db_writer`]
//! takes an `IndexDatabase`: the SQLite read surface implements the trait
//! without the orchestration changing. The trait is `Send` and deliberately
//! not `Sync`: one engine lives on one worker thread, the way the C++ query
//! pool gives each worker its own engine, and the scoring threads never touch
//! the reader — they only borrow the scorer and the candidates.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use compass_search::{MIN_QUALITY, Query};

use crate::db_writer::ScanRecord;

use crate::query_engine::{
    CANDIDATE_LIMIT, CORRECTION_FULL_PAGE_CONFIDENCE_THRESHOLD, IndexerFileResult,
    MAX_CORRECTION_PLANS, MAX_CORRECTIONS_PER_WORD, MIN_CORRECTABLE_WORD_LENGTH,
    SKELETON_MIN_QUALITY, SUGGESTION_FETCH_COUNT, SearchCandidate, SearchOptions,
    accepts_correction_results, correction_confidence, results_from_ranked, score_candidate,
    score_candidate_with_plan, score_candidates, skeleton_merge_decision, sort_scored,
};
use crate::query_policy::{
    QueryWord, SpellfixSuggestion, adjusted_suggestion_score, build_correction_plans,
    pick_corrections, prepare_candidate_search_query, prepare_correction_search_query,
    split_query_words,
};

/// What the query engine needs from the file-index database.
///
/// Mirrors the `FileIndexerDatabase` read methods the C++ engine calls, so
/// the SQLite port implements this trait without the orchestration changing.
///
/// `Send + 'static` so an engine moves into its worker thread, like the
/// writer's database; never `Sync`, because a SQLite connection must not be
/// touched from two threads at once.
pub trait IndexReader: Send + 'static {
    /// Whether the database opened. A closed database answers nothing, as in
    /// C++.
    fn is_open(&self) -> bool;
    /// Up to `limit` full-text path matches for an FTS match string.
    fn search_candidates(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchCandidate>;
    /// Up to `limit` skeleton-token matches for the same match string: the
    /// tokenizer skeletonizes the query itself.
    fn search_skeleton_candidates(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchCandidate>;
    /// Up to `top` vocabulary words near `word`, prefix-extended when asked.
    fn spellfix_suggestions(&self, word: &str, top: i32, prefix: bool) -> Vec<SpellfixSuggestion>;
    /// The indexed paths directly inside `path`: empty when the directory
    /// itself is not indexed.
    fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf>;
    /// Whether `path` has an indexed row at all.
    fn tracks_file(&self, path: &Path) -> bool;
    /// The latest succeeded scan — full or incremental — for `path`, if any.
    fn last_successful_scan(&self, path: &Path) -> Option<ScanRecord>;
}

/// One engine, one database reader.
///
/// Owns nothing but the reader: one engine per worker thread, the way the
/// C++ query pool gives each worker its own engine.
pub struct FileIndexerQueryEngine<R: IndexReader> {
    reader: R,
}

impl<R: IndexReader> FileIndexerQueryEngine<R> {
    /// Runs queries against `reader`.
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Whether there is a database to ask.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.reader.is_open()
    }

    /// Ranks indexed files for `query`, up to `limit` hits.
    ///
    /// Strict full-text candidates first; skeleton candidates merge in when
    /// the strict ranking is thin; spellfix corrections run when nothing
    /// came back or nothing ranked.
    #[must_use]
    pub fn query(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<IndexerFileResult> {
        if !self.reader.is_open() {
            return Vec::new();
        }
        let db_query = prepare_candidate_search_query(query);
        if db_query.is_empty() {
            return Vec::new();
        }

        let candidates = self
            .reader
            .search_candidates(&db_query, CANDIDATE_LIMIT, options);
        let candidate_count = candidates.len();

        let mut tried_corrections = false;
        if candidates.is_empty() {
            tried_corrections = true;
            let corrected = self.query_with_corrections(query, limit, options, true);
            if !corrected.is_empty() {
                return corrected;
            }
            let corrected = self.query_with_corrections(query, limit, options, false);
            if !corrected.is_empty() {
                return corrected;
            }
        }

        let fuzzy = Query::new(query);
        // Every strict path, including the ones scoring cuts: the skeleton
        // merge below must not re-add them.
        let seen: HashSet<PathBuf> = candidates
            .iter()
            .map(|candidate| candidate.path.clone())
            .collect();
        let mut ranked = Vec::new();
        // `no-strict-candidates` until a strict ranking says otherwise; the
        // decision only feeds the merge log below.
        let mut decision = skeleton_merge_decision(&ranked, query, limit);
        if !candidates.is_empty() {
            ranked = score_candidates(
                candidates,
                |candidate| score_candidate(candidate, &fuzzy, MIN_QUALITY),
                |path| path.is_dir(),
            );
            decision = skeleton_merge_decision(&ranked, query, limit);
            if !decision.should_merge {
                let results = results_from_ranked(&ranked, limit, Path::exists);
                if !results.is_empty() {
                    return results;
                }
                decision.should_merge = true;
                decision.reason = "strict-results-stale";
            }
        }

        if candidate_count < CANDIDATE_LIMIT {
            let fetched =
                self.reader
                    .search_skeleton_candidates(&db_query, CANDIDATE_LIMIT, options);
            tracing::debug!(
                reason = decision.reason,
                best_score = decision.best_score,
                confidence = decision.confidence,
                fetched = fetched.len(),
                "merging skeleton candidates"
            );
            let mut skeleton_candidates = Vec::new();
            for candidate in fetched {
                if !seen.contains(candidate.path.as_path()) {
                    skeleton_candidates.push(candidate);
                }
            }
            let mut skeleton_ranked = score_candidates(
                skeleton_candidates,
                |candidate| score_candidate(candidate, &fuzzy, SKELETON_MIN_QUALITY),
                |path| path.is_dir(),
            );
            ranked.append(&mut skeleton_ranked);
            sort_scored(&mut ranked, |path| path.is_dir());
        }

        let results = results_from_ranked(&ranked, limit, Path::exists);
        if !results.is_empty() {
            return results;
        }
        if tried_corrections {
            return Vec::new();
        }
        let corrected = self.query_with_corrections(query, limit, options, true);
        if !corrected.is_empty() {
            return corrected;
        }
        self.query_with_corrections(query, limit, options, false)
    }

    /// Searches each correction plan's reading of the query, keeping the most
    /// confident page of results.
    fn query_with_corrections(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
        trust_known_words: bool,
    ) -> Vec<IndexerFileResult> {
        let mut words: Vec<QueryWord> = split_query_words(query)
            .into_iter()
            .map(|word| QueryWord {
                word,
                corrections: Vec::new(),
            })
            .collect();
        let mut has_corrections = false;
        for query_word in &mut words {
            if query_word.word.len() < MIN_CORRECTABLE_WORD_LENGTH {
                continue;
            }
            let mut suggestions =
                self.reader
                    .spellfix_suggestions(&query_word.word, SUGGESTION_FETCH_COUNT, true);
            let mut exact =
                self.reader
                    .spellfix_suggestions(&query_word.word, SUGGESTION_FETCH_COUNT, false);
            suggestions.append(&mut exact);
            suggestions.sort_by(|a, b| {
                adjusted_suggestion_score(a).total_cmp(&adjusted_suggestion_score(b))
            });
            query_word.corrections = pick_corrections(
                &suggestions,
                &query_word.word,
                MAX_CORRECTIONS_PER_WORD,
                trust_known_words,
            );
            has_corrections |= !query_word.corrections.is_empty();
        }

        if !has_corrections {
            return Vec::new();
        }
        let plans = build_correction_plans(&words, MAX_CORRECTION_PLANS);
        if plans.is_empty() {
            return Vec::new();
        }

        let mut best_results = Vec::new();
        let mut best_confidence = 0.0;
        for plan in &plans {
            let correction_query = prepare_correction_search_query(plan);
            if correction_query.is_empty() {
                continue;
            }
            let candidates =
                self.reader
                    .search_candidates(&correction_query, CANDIDATE_LIMIT, options);
            let ranked = score_candidates(
                candidates,
                |candidate| score_candidate_with_plan(candidate, plan),
                |path| path.is_dir(),
            );
            let results = results_from_ranked(&ranked, limit, Path::exists);
            if results.is_empty() {
                continue;
            }
            let confidence = correction_confidence(&ranked, plan);
            tracing::debug!(
                correction = correction_query.as_str(),
                confidence,
                hits = results.len(),
                "spellfix fallback"
            );
            if confidence > best_confidence {
                best_confidence = confidence;
                best_results = results;
            }
            if accepts_correction_results(&ranked, plan, limit) {
                return best_results;
            }
        }

        if best_confidence >= CORRECTION_FULL_PAGE_CONFIDENCE_THRESHOLD {
            best_results
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_engine::IndexedFileCategory;
    use std::collections::HashMap;

    /// A reader scripted per query string.
    struct FakeReader {
        open: bool,
        hits: HashMap<String, Vec<PathBuf>>,
        skeleton_hits: HashMap<String, Vec<PathBuf>>,
        spellfix: HashMap<(String, bool), Vec<SpellfixSuggestion>>,
    }

    impl FakeReader {
        fn new() -> Self {
            Self {
                open: true,
                hits: HashMap::new(),
                skeleton_hits: HashMap::new(),
                spellfix: HashMap::new(),
            }
        }

        fn with_hit(mut self, query: &str, path: PathBuf) -> Self {
            self.hits.entry(query.to_owned()).or_default().push(path);
            self
        }

        fn with_skeleton_hit(mut self, query: &str, path: PathBuf) -> Self {
            self.skeleton_hits
                .entry(query.to_owned())
                .or_default()
                .push(path);
            self
        }

        fn with_suggestion(mut self, word: &str, suggestion: SpellfixSuggestion) -> Self {
            // The engine asks twice per word, prefix-extended then exact:
            // answer both the way a vocabulary containing the word would.
            self.spellfix
                .entry((word.to_owned(), true))
                .or_default()
                .push(suggestion.clone());
            self.spellfix
                .entry((word.to_owned(), false))
                .or_default()
                .push(suggestion);
            self
        }

        fn candidates(paths: Vec<PathBuf>) -> Vec<SearchCandidate> {
            paths
                .into_iter()
                .map(|path| SearchCandidate {
                    path,
                    category: IndexedFileCategory::Other,
                    mime_type: None,
                })
                .collect()
        }
    }

    impl IndexReader for FakeReader {
        fn is_open(&self) -> bool {
            self.open
        }

        fn search_candidates(
            &self,
            query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Self::candidates(self.hits.get(query).cloned().unwrap_or_default())
        }

        fn search_skeleton_candidates(
            &self,
            query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Self::candidates(self.skeleton_hits.get(query).cloned().unwrap_or_default())
        }

        fn spellfix_suggestions(
            &self,
            word: &str,
            _top: i32,
            prefix: bool,
        ) -> Vec<SpellfixSuggestion> {
            self.spellfix
                .get(&(word.to_owned(), prefix))
                .cloned()
                .unwrap_or_default()
        }

        fn list_indexed_directory_files(&self, _path: &Path) -> HashSet<PathBuf> {
            HashSet::new()
        }

        fn tracks_file(&self, _path: &Path) -> bool {
            false
        }

        fn last_successful_scan(&self, _path: &Path) -> Option<ScanRecord> {
            None
        }
    }

    fn touch(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, "x").expect("test file");
        path
    }

    fn engine(reader: FakeReader) -> FileIndexerQueryEngine<FakeReader> {
        FileIndexerQueryEngine::new(reader)
    }

    #[test]
    fn closed_reader_answers_nothing() {
        let reader = FakeReader::new();
        let engine = FileIndexerQueryEngine::new(FakeReader {
            open: false,
            ..reader
        });
        assert!(!engine.is_available());
        assert!(
            engine
                .query("report", 10, &SearchOptions::default())
                .is_empty()
        );
    }

    #[test]
    fn blank_query_asks_nothing() {
        let reader = FakeReader::new();
        let results = engine(reader).query("   ", 10, &SearchOptions::default());
        assert!(results.is_empty());
    }

    #[test]
    fn strict_hit_returns_the_file() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = touch(&dir, "report.txt");
        let reader = FakeReader::new().with_hit("\"report\"", path.clone());
        let results = engine(reader).query("report", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, path);
        assert!(results[0].rank > 0.0);
    }

    #[test]
    fn skeleton_does_not_double_count_a_strict_path() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = touch(&dir, "report.txt");
        let reader = FakeReader::new()
            .with_hit("\"report\"", path.clone())
            .with_skeleton_hit("\"report\"", path.clone());
        let results = engine(reader).query("report", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn stale_strict_falls_through_to_skeleton() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let live = touch(&dir, "quarterly-report.txt");
        let stale = dir.path().join("deleted-notes.txt");
        let reader = FakeReader::new()
            .with_hit("\"report\"", stale)
            .with_skeleton_hit("\"report\"", live.clone());
        let results = engine(reader).query("report", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, live);
    }

    #[test]
    fn corrections_answer_when_nothing_matches() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = touch(&dir, "report.txt");
        let reader = FakeReader::new()
            .with_hit("\"report\"", path.clone())
            .with_suggestion(
                "reprot",
                SpellfixSuggestion {
                    word: "report".to_owned(),
                    distance: 2,
                    score: 100,
                    rank: 5,
                },
            );
        let results = engine(reader).query("reprot", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, path);
    }

    #[test]
    fn no_suggestions_means_no_correction_results() {
        let reader = FakeReader::new();
        let results = engine(reader).query("reprot", 10, &SearchOptions::default());
        assert!(results.is_empty());
    }

    #[test]
    fn zero_limit_returns_nothing() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = touch(&dir, "report.txt");
        let reader = FakeReader::new().with_hit("\"report\"", path);
        let results = engine(reader).query("report", 0, &SearchOptions::default());
        assert!(results.is_empty());
    }
}
