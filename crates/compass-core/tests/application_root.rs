use std::sync::Arc;

use compass_core::{AppIndex, FrecencyStore, JsonFrecencyStore, ManualClock};
use compass_xdg::Locale;

fn index() -> (tempfile::TempDir, AppIndex) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser\nTryExec=compass-not-installed-sentinel\nGenericName=GenericSentinel\nComment=CommentSentinel\nCategories=CategorySentinel;\nKeywords=KeywordSentinel;\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=browser --private\n").unwrap();
    std::fs::write(
        dir.path().join("files.desktop"),
        "[Desktop Entry]\nType=Application\nName=Files\nName[de]=Dateien\nExec=files\n",
    )
    .unwrap();
    let index = AppIndex::builder()
        .dir(dir.path())
        .locale(Locale::parse("de_DE"))
        .build();
    (dir, index)
}

#[test]
fn root_rows_are_applications_even_when_host_try_exec_is_not_visible() {
    let (_dir, index) = index();
    let hits = index.search_root("", None);
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| !hit.item.is_action()));
    let browser = hits
        .iter()
        .find(|hit| hit.item.key() == "browser.desktop")
        .unwrap();
    assert!(!browser.item.launchable());
    assert_eq!(index.items()[browser.index].key(), browser.item.key());
    assert!(index.search_root("private", None).is_empty());
}

#[test]
fn application_root_settings_filter_alias_and_reset_without_changing_launch_keys() {
    let (_dir, mut index) = index();
    let config = compass_core::Config::parse(
        r#"{"providers":{"applications":{"entrypoints":{
            "browser":{"alias":"uniquealias"},"files":{"enabled":false}
        }}}}"#,
        std::path::Path::new("config.json"),
    )
    .unwrap();
    index.apply_root_config(&config.root_config());
    let hits = index.search_root("uniquealias", None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.key(), "browser.desktop");
    assert_eq!(index.items()[hits[0].index].key(), "browser.desktop");
    assert!(index.search_root("Files", None).is_empty());
    assert_eq!(index.search_root("", None).len(), 1);
    index.apply_root_config(&Default::default());
    assert!(index.search_root("uniquealias", None).is_empty());
    assert_eq!(index.search_root("Files", None).len(), 1);
    assert_eq!(index.search_root("", None).len(), 2);
}

#[test]
fn root_fields_exclude_descriptions_but_include_categories_as_keywords() {
    let (_dir, index) = index();
    for query in ["GenericSentinel", "CommentSentinel"] {
        assert!(index.search_root(query, None).is_empty(), "{query}");
    }
    assert_eq!(
        index.search_root("KeywordSentinel", None)[0].item.key(),
        "browser.desktop"
    );
    let expected = compass_search::score_weighted(
        &[compass_search::WeightedField::new("CategorySentinel", 0.6)],
        &compass_search::Query::new("CategorySentinel"),
    );
    assert_eq!(
        index.search_root("CategorySentinel", None)[0].match_score,
        expected.score
    );
}

#[test]
fn an_english_name_finds_the_localized_root_at_title_weight() {
    let (_dir, index) = index();
    let hits = index.search_root("Files", None);
    assert_eq!(hits[0].item.name(), "Dateien");
    assert_eq!(hits[0].match_score, 100);
}

#[test]
fn root_history_uses_the_existing_desktop_key_and_keeps_wire_scores_unboosted() {
    let (_dir, index) = index();
    let mut history = JsonFrecencyStore::in_memory(Arc::new(ManualClock::new(1700000000)));
    history.record_launch("files.desktop").unwrap();
    let hits = index.search_root("", Some(&history));
    assert_eq!(hits[0].item.key(), "files.desktop");
    assert_eq!(hits[0].match_score, 0);
    assert_eq!(
        index.search_root("Files", Some(&history))[0].match_score,
        100
    );
}

#[test]
fn equal_root_scores_keep_case_insensitive_display_order_not_file_order() {
    let dir = tempfile::tempdir().unwrap();
    for (id, name) in [("a.desktop", "Chromium Web"), ("z.desktop", "chroma")] {
        std::fs::write(
            dir.path().join(id),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n"),
        )
        .unwrap();
    }
    let index = AppIndex::builder().dir(dir.path()).build();
    for query in ["", "chrom"] {
        let hits = index.search_root(query, None);
        assert_eq!(
            hits.iter().map(|hit| hit.item.key()).collect::<Vec<_>>(),
            ["z.desktop", "a.desktop"]
        );
        for hit in hits {
            assert_eq!(index.items()[hit.index].key(), hit.item.key());
        }
    }
}
