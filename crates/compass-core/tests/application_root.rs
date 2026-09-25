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

fn command_ids(hits: &[compass_core::RootHit<'_>]) -> Vec<String> {
    hits.iter()
        .filter_map(|hit| match hit {
            compass_core::RootHit::Command { command, .. } => Some(command.id()),
            compass_core::RootHit::App(_)
            | compass_core::RootHit::Extension { .. }
            | compass_core::RootHit::Shortcut { .. }
            | compass_core::RootHit::Script { .. }
            | compass_core::RootHit::RhaiScript { .. } => None,
        })
        .collect()
}

#[test]
fn builtin_commands_are_found_by_title_keyword_and_typo() {
    let (_dir, index) = index();
    for query in ["Clipboard History", "clipboard", "paste", "clipbaord"] {
        let hits = index.search_root_all(query, None);
        assert_eq!(
            command_ids(&hits).first().map(String::as_str),
            Some("commands:clipboard-history"),
            "{query}"
        );
    }
    let exact = index.search_root_all("Clipboard History", None);
    assert!(matches!(
        exact.first(),
        Some(compass_core::RootHit::Command {
            match_score: 100,
            ..
        })
    ));
}

#[test]
fn application_only_search_never_returns_a_command() {
    let (_dir, index) = index();
    for query in ["", "clipboard", "clipbaord"] {
        let all = index.search_root_all(query, None);
        let apps = index.search_root(query, None);
        let app_rows = all
            .iter()
            .filter(|hit| matches!(hit, compass_core::RootHit::App(_)))
            .count();
        assert_eq!(apps.len(), app_rows, "{query:?}");
    }
    assert!(index.search_root("clipboard", None).is_empty());
    assert!(
        index
            .position_by_entrypoint("commands:clipboard-history")
            .is_none()
    );
}

#[test]
fn a_command_used_often_rises_on_the_empty_query() {
    let (_dir, index) = index();
    let mut history = JsonFrecencyStore::in_memory(Arc::new(ManualClock::new(1700000000)));
    history.record_launch("commands:clipboard-history").unwrap();
    let hits = index.search_root_all("", Some(&history));
    assert!(
        matches!(hits.first(), Some(compass_core::RootHit::Command { .. })),
        "{hits:?}"
    );
}

/// An extension directory holding one extension with a view and a no-view
/// command, the second disabled by its manifest.
fn extensions() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ext = dir.path().join("github");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{
            "name": "github", "title": "GitHub", "author": "raycast",
            "preferences": [
                {"name": "token", "title": "Personal Access Token", "type": "password",
                 "required": true},
                {"name": "limit", "title": "Limit", "type": "textfield", "default": "20"}
            ],
            "commands": [
                {"name": "search-repositories", "title": "Search Repositories",
                 "mode": "view", "keywords": ["repo"]},
                {"name": "sync", "title": "Sync Notifications", "mode": "no-view",
                 "disabledByDefault": true}
            ]
        }"#,
    )
    .unwrap();
    dir
}

#[test]
fn installed_extension_commands_are_found_under_their_extension() {
    let apps = tempfile::tempdir().unwrap();
    let installed = extensions();
    let index = AppIndex::builder()
        .dir(apps.path())
        .extension_dirs([installed.path()])
        .build();

    for query in ["Search Repositories", "repo", "search repos"] {
        let hits = index.search_root_all(query, None);
        let Some(compass_core::RootHit::Extension { command, .. }) = hits.first() else {
            panic!("{query:?}: {hits:?}");
        };
        assert_eq!(
            command.id, "@raycast/github:search-repositories",
            "{query:?}"
        );
        assert_eq!(command.extension_title, "GitHub");
        assert_eq!(command.mode, compass_core::manifest::CommandMode::View);
        assert!(
            command
                .entrypoint
                .ends_with("github/search-repositories.js")
        );
    }
    assert!(
        index
            .search_root_all("Sync Notifications", None)
            .iter()
            .all(|hit| !matches!(
                hit,
                compass_core::RootHit::Extension { command, .. } if command.name == "sync"
            )),
        "a command its manifest disables stays out of the root"
    );
    assert!(
        index.extension("@raycast/github:sync").is_some(),
        "but is still known"
    );
    assert!(
        index.search_root("repo", None).is_empty(),
        "never an application hit"
    );
}

#[test]
fn an_index_built_without_extension_dirs_has_no_extensions() {
    let apps = tempfile::tempdir().unwrap();
    let index = AppIndex::builder().dir(apps.path()).build();
    assert!(index.extensions().is_empty());
}

#[test]
fn a_launch_passes_defaults_and_names_required_preferences_it_cannot_fill() {
    let apps = tempfile::tempdir().unwrap();
    let installed = extensions();
    let index = AppIndex::builder()
        .dir(apps.path())
        .extension_dirs([installed.path()])
        .build();
    let command = index
        .extension("@raycast/github:search-repositories")
        .unwrap();
    assert_eq!(command.extension_name, "github");
    assert_eq!(command.author, "raycast");
    assert_eq!(
        command.default_preferences(),
        Err(vec!["Personal Access Token".to_owned()])
    );

    let mut satisfied = command.clone();
    satisfied
        .preferences
        .retain(|preference| !preference.required);
    assert_eq!(
        satisfied.default_preferences(),
        Ok(serde_json::json!({"limit": "20"}))
    );
}

#[test]
fn stored_preferences_fill_what_defaults_cannot_and_override_the_rest() {
    let apps = tempfile::tempdir().unwrap();
    let installed = extensions();
    let index = AppIndex::builder()
        .dir(apps.path())
        .extension_dirs([installed.path()])
        .build();
    let command = index
        .extension("@raycast/github:search-repositories")
        .unwrap();

    let empty = serde_json::Map::new();
    let missing: Vec<&str> = command
        .preferences_with(&empty)
        .unwrap_err()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(missing, ["token"]);

    let stored = serde_json::json!({"token": "ghp_x", "limit": "50", "gone": 1});
    assert_eq!(
        command.preferences_with(stored.as_object().unwrap()),
        Ok(serde_json::json!({"token": "ghp_x", "limit": "50"})),
        "stored wins over the default, and an undeclared name is dropped"
    );
    let blank = serde_json::json!({"token": ""});
    assert!(
        command
            .preferences_with(blank.as_object().unwrap())
            .is_err(),
        "an empty required value is still missing"
    );
}
