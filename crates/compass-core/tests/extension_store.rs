//! Which store extensions are offered, and what they are called.
//!
//! Read off `VicinaeStoreService` (`src/server/src/services/extension-store/`).

use compass_core::extension_store::{
    DEFAULT_LIMIT, DEFAULT_PAGE, Extension, FALLBACK_ICON, FETCH_ALL_LIMIT, ID_PREFIX, Icons,
    ListPaginationOptions, ListResponse, ThemeVariant, available_on, encode_query_value,
    fetch_all_path, list_path, post_process, search_path,
};

/// An extension called `name`, running everywhere.
fn extension(name: &str) -> Extension {
    Extension {
        id: "whatever-the-server-said".to_owned(),
        name: name.to_owned(),
        title: name.to_owned(),
        ..Extension::default()
    }
}

/// A response carrying `extensions`.
fn response(extensions: Vec<Extension>) -> ListResponse {
    ListResponse {
        extensions,
        ..ListResponse::default()
    }
}

#[test]
fn the_pagination_defaults_are_the_cpp_ones() {
    let options = ListPaginationOptions::default();
    assert_eq!(options.page, DEFAULT_PAGE);
    assert_eq!(options.limit, DEFAULT_LIMIT);
    assert_eq!(DEFAULT_PAGE, 1, "pages are one-based");
    assert_eq!(DEFAULT_LIMIT, 50);
    assert_eq!(FETCH_ALL_LIMIT, 500);
}

#[test]
fn the_list_path_carries_the_page_and_the_limit() {
    assert_eq!(
        list_path(ListPaginationOptions { page: 3, limit: 20 }),
        "/store/list?page=3&limit=20"
    );
}

#[test]
fn fetch_all_asks_for_the_first_page_five_hundred_at_a_time() {
    assert_eq!(fetch_all_path(), "/store/list?page=1&limit=500");
}

#[test]
fn an_ordinary_search_makes_the_url_it_always_made() {
    assert_eq!(search_path("todo"), "/store/search?q=todo");
    assert_eq!(
        search_path("spotify-controls"),
        "/store/search?q=spotify-controls"
    );
}

#[test]
fn a_search_containing_an_ampersand_stays_one_parameter() {
    // The C++ substitutes the query verbatim, so "a & b" asks the server for
    // q=a plus a parameter called " b". See PARITY.md — deliberately escaped.
    assert_eq!(search_path("a & b"), "/store/search?q=a%20%26%20b");
    assert!(!search_path("a & b").contains(" & "));
}

#[test]
fn a_search_containing_a_hash_does_not_become_a_fragment() {
    assert_eq!(search_path("c#"), "/store/search?q=c%23");
}

#[test]
fn the_unreserved_characters_are_left_alone() {
    // So an ordinary search is byte-identical to what the C++ sends.
    assert_eq!(encode_query_value("azAZ09-._~"), "azAZ09-._~");
}

#[test]
fn non_ascii_is_encoded_as_utf8_bytes() {
    assert_eq!(encode_query_value("é"), "%C3%A9");
}

#[test]
fn an_extension_with_no_platforms_runs_everywhere() {
    // Which is how a pure-TypeScript extension avoids naming them all.
    let ext = extension("todo");
    assert!(available_on(&ext, "linux"));
    assert!(available_on(&ext, "macos"));
    assert!(available_on(&ext, "windows"));
}

#[test]
fn an_extension_is_matched_against_its_platforms_case_insensitively() {
    let mut ext = extension("todo");
    ext.platforms = vec!["Linux".to_owned(), "macOS".to_owned()];
    assert!(available_on(&ext, "linux"));
    assert!(available_on(&ext, "macos"));
    assert!(!available_on(&ext, "windows"));
}

#[test]
fn an_extension_for_another_platform_is_dropped_from_the_response() {
    let mut mac_only = extension("mac-thing");
    mac_only.platforms = vec!["macos".to_owned()];

    let mut result = response(vec![extension("everywhere"), mac_only]);
    post_process(&mut result, "linux");

    let names: Vec<_> = result.extensions.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec!["everywhere".to_owned()]);
}

#[test]
fn every_surviving_extension_gets_a_store_id() {
    // The server's id is replaced, not merged: nothing else in the system uses
    // it, and the rewritten one is what a root-search row is keyed by.
    let mut result = response(vec![extension("todo")]);
    post_process(&mut result, "linux");

    assert_eq!(result.extensions[0].id, "store.vicinae.todo");
    assert_eq!(ID_PREFIX, "store.vicinae.");
}

#[test]
fn a_dropped_extension_does_not_get_an_id_or_a_slot() {
    let mut mac_only = extension("mac-thing");
    mac_only.platforms = vec!["macos".to_owned()];
    let mut result = response(vec![mac_only]);
    post_process(&mut result, "linux");

    assert!(result.extensions.is_empty());
}

#[test]
fn post_processing_an_empty_response_does_nothing() {
    let mut result = response(Vec::new());
    post_process(&mut result, "linux");
    assert!(result.extensions.is_empty());
}

#[test]
fn a_dark_theme_prefers_the_dark_icon() {
    let icons = Icons {
        light: Some("light.png".to_owned()),
        dark: Some("dark.png".to_owned()),
    };
    assert_eq!(icons.themed_icon(ThemeVariant::Dark), Some("dark.png"));
    assert_eq!(icons.themed_icon(ThemeVariant::Light), Some("light.png"));
}

#[test]
fn a_dark_theme_falls_back_to_the_light_icon() {
    let icons = Icons {
        light: Some("light.png".to_owned()),
        dark: None,
    };
    assert_eq!(icons.themed_icon(ThemeVariant::Dark), Some("light.png"));
}

#[test]
fn a_light_theme_falls_back_to_the_dark_icon() {
    // Looks wrong and is better than a blank, which is what the C++ decided.
    let icons = Icons {
        light: None,
        dark: Some("dark.png".to_owned()),
    };
    assert_eq!(icons.themed_icon(ThemeVariant::Light), Some("dark.png"));
}

#[test]
fn an_extension_with_no_icons_at_all_gets_the_plug() {
    // A store list with holes in it reads as broken rather than as sparse.
    let ext = extension("todo");
    assert_eq!(ext.icons.themed_icon(ThemeVariant::Light), None);
    assert_eq!(ext.themed_icon(ThemeVariant::Light), FALLBACK_ICON);
    assert_eq!(FALLBACK_ICON, "plug");
}

#[test]
fn an_extension_with_icons_uses_them_rather_than_the_plug() {
    let mut ext = extension("todo");
    ext.icons = Icons {
        light: Some("light.png".to_owned()),
        dark: None,
    };
    assert_eq!(ext.themed_icon(ThemeVariant::Light), "light.png");
}

#[test]
fn a_store_response_parses_from_the_servers_camel_case() {
    let json = r#"{
      "extensions": [{
        "id": "srv-1", "name": "todo", "title": "Todo",
        "description": "Keep a list",
        "author": { "handle": "ada", "name": "Ada", "avatarUrl": "a", "profileUrl": "p" },
        "downloadCount": 12, "apiVersion": "1.0.0", "checksum": "abc",
        "trending": true, "platforms": ["linux"],
        "sourceUrl": "s", "readmeUrl": "r", "downloadUrl": "d",
        "commands": [{
          "id": "c1", "name": "list", "title": "List", "subtitle": "",
          "description": "", "mode": "view", "disabledByDefault": true, "beta": true
        }]
      }],
      "pagination": { "page": 2, "limit": 10, "total": 30, "totalPages": 3,
                      "hasNext": true, "hasPrev": true }
    }"#;
    let mut parsed: ListResponse = serde_json::from_str(json).expect("parses");

    assert_eq!(parsed.extensions[0].download_count, 12);
    assert_eq!(parsed.extensions[0].author.avatar_url, "a");
    assert_eq!(parsed.extensions[0].source_url, "s");
    assert!(parsed.extensions[0].commands[0].disabled_by_default);
    assert!(parsed.extensions[0].commands[0].beta);
    assert_eq!(parsed.pagination.total_pages, 3);
    assert!(parsed.pagination.has_next);

    post_process(&mut parsed, "linux");
    assert_eq!(parsed.extensions[0].id, "store.vicinae.todo");
}

#[test]
fn a_response_missing_optional_fields_still_parses() {
    // The server can add and drop fields; a store that refused to parse would
    // show nothing at all rather than slightly less.
    let parsed: ListResponse = serde_json::from_str(
        r#"{"extensions":[{"id":"x","name":"n","title":"t","description":"d","mode":"view"}]}"#,
    )
    .expect("parses");
    assert_eq!(parsed.extensions.len(), 1);
    assert!(parsed.extensions[0].platforms.is_empty());
    assert_eq!(parsed.pagination.page, DEFAULT_PAGE);
}
