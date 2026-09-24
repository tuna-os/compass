//! What file search owes a person, through the real SQLite reader and writer.
//!
//! A port of `src/file-indexer/tests/query-quality.cpp`, the C++ file
//! indexer's quality suite, run against the Rust engine's own database code:
//! real files on disk, indexed by `SqliteWriter`, queried through
//! `FileIndexerQueryEngine` over `SqliteReader`. Nothing here is stubbed.
//!
//! It exists as the safety net for PLAN.md §12.0 item 2: moving typo
//! correction off the vendored `spellfix1` extension, and re-basing the SQLite
//! wrapper on `rusqlite`. The cases are grouped by which part of the stack
//! they lean on, so a regression points at its cause:
//!
//! - *strict* and *bridging*: the `fuzzy_trigram` tokenizer's trigram and
//!   separator handling;
//! - *fallback*: the vocabulary typo correction `spellfix1` provides today;
//! - *skeleton*: the tokenizer's vowel-dropped tokens and skip-grams.
//!
//! Measured, not assumed: with `spellfix_suggestions` stubbed to return
//! nothing, exactly four cases fail — `fallback_transposition_typo`,
//! `fallback_a_typoed_extension_is_corrected`,
//! `fallback_a_numbered_family_does_not_crowd_out_the_real_correction` and
//! `fallback_weak_words_are_dropped_from_filtering_but_still_ranked`. The other
//! `fallback_*` cases are rescued by the skeleton index, so they say nothing
//! about spellfix; those four are the bar a replacement has to clear.
//!
//! The C++ cases about database bookkeeping (recent directories, sizes,
//! refresh times) are not search quality and are not ported here.

use std::path::PathBuf;
use std::sync::OnceLock;

use compass_db::db_writer::IndexDatabase as _;
use compass_db::query_engine::{IndexedFileCategory, SearchOptions};
use compass_db::query_reader::FileIndexerQueryEngine;
use compass_db::sqlite_reader::SqliteReader;
use compass_db::sqlite_writer::{SqliteWriter, prepare_file_index_database};

/// Every path is a real empty file: ranking stats paths and drops entries that
/// do not exist.
const CORPUS: &[&str] = &[
    "home/docs/budget_2024.xlsx",
    "home/docs/invoice.xlsx",
    "home/docs/Répondre à la nuit.epub",
    "home/docs/vers_la_modernisation.pdf",
    "home/docs/hello-yo.txt",
    "home/docs/notes/meeting_notes.md",
    "home/pictures/icons/frostwire.svg",
    "home/pictures/icons/firstperson.svg",
    "home/pictures/icons/firstparty.svg",
    "home/pictures/icons/SymbolEditor.svg",
    "home/pictures/wallpapers/4k-oled/jellyfish-amoled.png",
    "home/dev/sdk/include/sercom0.h",
    "home/dev/sdk/include/sercom1.h",
    "home/dev/sdk/include/sercom2.h",
    "home/dev/sdk/include/sercom3.h",
    "home/dev/sdk/include/sercom4.h",
    "home/dev/sdk/include/sercom5.h",
    "home/dev/sdk/include/sercom6.h",
    "home/dev/sdk/include/sercom7.h",
    "home/config/vivaldi/search_engines.json",
    "home/dev/kubeConfigBackup.yaml",
    "home/dev/searchEngine.js",
    "home/dev/captures/vrs-dump0.bin",
    "home/dev/captures/vrs-dump1.bin",
    "home/dev/captures/vrs-dump2.bin",
    "home/music/mayonnaise.flac",
    "home/music/yolo-compilation.mp4",
    "home/archive/downloads_backup.tar",
    "home/qmk_firmware/keyboards/planck/keymaps/default/keymap.c",
    "home/dev/kernel/vm_kmap.c",
    "home/docs/la belle data.pdf",
    "home/pictures/Screen Shot 2024-01-15 at 10.30.12.png",
    "home/fonts/SF-Pro-Display-Bold.otf",
    "home/dev/data_loader_utils.py",
    "home/apps/vicinae.AppImage",
    "home/dev/jai/modules/windows.jai",
    "home/pictures/VACATION.JPG",
    "home/pictures/forest_color.jpg",
];

struct Env {
    _root: tempfile::TempDir,
    db: PathBuf,
}

fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("a temp dir");
        let corpus = root.path().join("corpus");
        let mut paths: Vec<PathBuf> = Vec::with_capacity(CORPUS.len() + 1);
        for relative in CORPUS {
            let path = corpus.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            paths.push(path);
        }
        paths.push(corpus.join("home/docs"));

        let db = root.path().join("file-index.db");
        // The scanner owns the schema; the writer only writes.
        prepare_file_index_database(&db).expect("the schema applies");
        let mut writer = SqliteWriter::open(&db).expect("the index opens");
        writer.index_files(&paths);
        writer.rebuild_spellfix_vocabulary();
        drop(writer);
        Env { _root: root, db }
    })
}

fn query(text: &str, category: Option<IndexedFileCategory>) -> Vec<PathBuf> {
    let reader = SqliteReader::open(&env().db).unwrap_or_else(|e| panic!("reopen: {e:?}"));
    let engine = FileIndexerQueryEngine::new(reader);
    engine
        .query(text, 10, &SearchOptions { category })
        .into_iter()
        .map(|hit| hit.path)
        .collect()
}

/// The 1-based rank of the first result whose path ends with `suffix`.
fn rank_of(text: &str, suffix: &str) -> Option<usize> {
    rank_in(&query(text, None), suffix)
}

fn rank_in(results: &[PathBuf], suffix: &str) -> Option<usize> {
    results
        .iter()
        .position(|path| path.to_string_lossy().ends_with(suffix))
        .map(|zero| zero + 1)
}

#[track_caller]
fn assert_in_top(text: &str, suffix: &str, n: usize) {
    let results = query(text, None);
    let rank = rank_in(&results, suffix);
    assert!(
        rank.is_some_and(|rank| rank <= n),
        "{text:?} must surface {suffix:?} in the top {n}, ranked {rank:?}; got {results:#?}"
    );
}

#[track_caller]
fn assert_in_top_of(text: &str, suffix: &str, n: usize, category: IndexedFileCategory) {
    let results = query(text, Some(category));
    let rank = rank_in(&results, suffix);
    assert!(
        rank.is_some_and(|rank| rank <= n),
        "{text:?} in {category:?} must surface {suffix:?} in the top {n}, ranked {rank:?}"
    );
}

/// CONTROL: the gibberish case below asserts emptiness, which an index that
/// failed to build would satisfy for the wrong reason.
#[test]
fn control_the_corpus_is_indexed() {
    assert_eq!(rank_of("budget", "budget_2024.xlsx"), Some(1));
    assert_eq!(rank_of("mayonnaise", "mayonnaise.flac"), Some(1));
}

#[test]
fn strict_exact_words_rank_their_file_first() {
    assert_eq!(rank_of("budget", "budget_2024.xlsx"), Some(1));
    assert_eq!(rank_of("invoice xlsx", "invoice.xlsx"), Some(1));
}

#[test]
fn strict_bigrams_match_through_prefix_queries() {
    assert_eq!(rank_of("hello yo", "hello-yo.txt"), Some(1));
}

#[test]
fn strict_search_ignores_diacritics() {
    assert_in_top_of(
        "repondre",
        "Répondre à la nuit.epub",
        3,
        IndexedFileCategory::Document,
    );
}

#[test]
fn fallback_transposition_typo() {
    assert_in_top("budgte", "budget_2024.xlsx", 3);
}

#[test]
fn fallback_missing_character_typo() {
    assert_in_top("mayonaise", "mayonnaise.flac", 3);
}

#[test]
fn fallback_close_correction_outranks_a_longer_far_one() {
    // firstperson/firstparty share more characters but are further away.
    assert_eq!(rank_of("frstwire", "frostwire.svg"), Some(1));
}

#[test]
fn fallback_word_fragment_corrects_against_vocabulary_prefixes() {
    assert_in_top("moderna", "vers_la_modernisation.pdf", 3);
}

#[test]
fn fallback_a_numbered_family_does_not_crowd_out_the_real_correction() {
    assert_in_top("serach engine", "search_engines.json", 3);
}

#[test]
fn fallback_a_typoed_extension_is_corrected() {
    assert_in_top("invoice xslx", "invoice.xlsx", 3);
}

#[test]
fn fallback_vowel_drop_against_camel_case_vocabulary() {
    assert_in_top("confg backup", "kubeConfigBackup.yaml", 3);
}

#[test]
fn fallback_weak_words_are_dropped_from_filtering_but_still_ranked() {
    // 'vrs' is a real, trusted corpus token, so the first corrected pass finds
    // nothing and the distrust retry must correct it to 'vers'.
    assert_in_top("vrs lo moderna", "vers_la_modernisation.pdf", 3);
}

#[test]
fn fallback_gibberish_returns_nothing_rather_than_noise() {
    let results = query("zzqqxxw", None);
    assert!(results.is_empty(), "{results:#?}");
}

#[test]
fn skeleton_a_fully_devowelled_word_matches() {
    // Distance 3 from 'downloads': out of spellfix's reach.
    assert_in_top("dwnlds", "downloads_backup.tar", 3);
}

#[test]
fn skeleton_a_real_world_abbreviation_mix() {
    assert_in_top("qmk firm dflt", "keymaps/default/keymap.c", 3);
}

#[test]
fn skeleton_merges_with_literal_matches_instead_of_being_gated_by_them() {
    assert_eq!(rank_of("kmap", "vm_kmap.c"), Some(1));
    assert_in_top("kmap", "keymaps/default/keymap.c", 4);
}

#[test]
fn skeleton_dropped_consonants_match_through_skip_grams() {
    assert_in_top("cfg backup", "kubeConfigBackup.yaml", 3);
    assert_in_top("cfg bkup", "kubeConfigBackup.yaml", 3);
    assert_in_top("dwld", "downloads_backup.tar", 3);
    assert_in_top("kmap", "keymaps/default/keymap.c", 3);
    assert_in_top("frs clr", "home/pictures/forest_color.jpg", 3);
}

#[test]
fn skeleton_short_tokens_are_kept_in_phrase_matches() {
    assert_in_top("jai wndws", "jai/modules/windows.jai", 3);
}

#[test]
fn bridging_concatenated_queries_match_across_word_boundaries() {
    assert_in_top("labelledata", "la belle data.pdf", 3);
    assert_in_top("screenshot", "Screen Shot 2024-01-15 at 10.30.12.png", 3);
}

#[test]
fn bridging_every_separator_collapses_like_a_space() {
    assert_in_top("sfpro", "SF-Pro-Display-Bold.otf", 3);
    assert_in_top("sf pro", "SF-Pro-Display-Bold.otf", 3);
    assert_in_top("dataloader", "data_loader_utils.py", 3);
    assert_in_top("invoicexlsx", "invoice.xlsx", 3);
}

#[test]
fn category_filter_returns_matching_files() {
    use IndexedFileCategory::*;
    for (text, suffix, category) in [
        ("invoice", "invoice.xlsx", Document),
        ("vacation", "VACATION.JPG", Image),
        ("mayonnaise", "mayonnaise.flac", Audio),
        ("yolo", "yolo-compilation.mp4", Video),
        ("downloads", "downloads_backup.tar", Archive),
        ("keymap", "keymap.c", Other),
        ("vicinae", "vicinae.AppImage", Application),
        ("vrs dump0", "vrs-dump0.bin", Other),
        ("docs", "home/docs", Directory),
    ] {
        assert_in_top_of(text, suffix, 3, category);
    }
}

#[test]
fn category_filter_is_case_insensitive_for_extensions() {
    assert_in_top_of("vacation", "VACATION.JPG", 3, IndexedFileCategory::Image);
    let docs = query("vacation", Some(IndexedFileCategory::Document));
    assert!(rank_in(&docs, "VACATION.JPG").is_none(), "{docs:#?}");
}

#[test]
fn an_exact_path_component_wins_and_a_mid_word_subsequence_is_dropped() {
    assert_eq!(rank_of("oled", "4k-oled/jellyfish-amoled.png"), Some(1));
    assert_eq!(rank_of("oled", "icons/SymbolEditor.svg"), None);
}
