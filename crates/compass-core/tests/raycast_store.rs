//! Which Raycast extensions are offered here, and what they are called.
//!
//! Read off `RaycastStoreService` (`src/server/src/services/raycast/`).

use compass_core::raycast_store::{
    API_BASE_URL, COMPAT_PATH, Command, CompatInfo, CompatMap, DEFAULT_PAGE, DEFAULT_PER_PAGE,
    Extension, ID_PREFIX, Icons, ListApiResponse, ListPaginationOptions, RaycastStoreService,
    available_on, encode_path_value, extension_path, has_compat_sheet, is_native_platform,
    list_path, post_process, post_process_extension, search_path,
};

/// An extension called `name`, advertising macOS only, as Raycast's do.
fn extension(name: &str) -> Extension {
    Extension {
        id: "whatever-the-api-said".to_owned(),
        name: name.to_owned(),
        platforms: Some(vec!["macOS".to_owned()]),
        ..Extension::default()
    }
}

#[test]
fn the_endpoints_and_the_prefix_are_the_cpp_ones() {
    assert_eq!(API_BASE_URL, "https://backend.raycast.com/api/v1");
    assert_eq!(ID_PREFIX, "store.raycast.");
    assert_eq!(COMPAT_PATH, "/raycast/get-compat");
    assert_eq!(DEFAULT_PAGE, 1);
    assert_eq!(DEFAULT_PER_PAGE, 50);
}

#[test]
fn the_raycast_prefix_is_not_the_vicinae_one() {
    // A root-search row from one store must never be mistaken for the other's.
    assert_ne!(ID_PREFIX, compass_core::extension_store::ID_PREFIX);
}

#[test]
fn linux_is_offered_every_extension_whatever_it_advertises() {
    // The whole bargain: Raycast will not advertise Linux compatibility, so
    // filtering on it would give an empty store rather than a best-effort one.
    let mac_only = extension("clipboard-history");
    assert!(available_on(&mac_only, "linux"));
}

#[test]
fn a_platform_raycast_runs_on_does_filter() {
    let mac_only = extension("clipboard-history");
    assert!(available_on(&mac_only, "macos"));
    assert!(!available_on(&mac_only, "windows"));
}

#[test]
fn an_extension_with_no_platforms_is_offered_everywhere() {
    let mut anywhere = extension("anywhere");
    anywhere.platforms = None;
    assert!(available_on(&anywhere, "macos"));
    assert!(available_on(&anywhere, "windows"));
    assert!(available_on(&anywhere, "linux"));
}

#[test]
fn the_platform_match_is_case_insensitive() {
    // The API says "macOS"; the platform string is lowercase.
    let mac_only = extension("clipboard-history");
    assert_eq!(mac_only.platforms.as_ref().expect("some")[0], "macOS");
    assert!(available_on(&mac_only, "macos"));
}

#[test]
fn only_macos_and_windows_are_native_platforms() {
    assert!(is_native_platform("macos"));
    assert!(is_native_platform("windows"));
    assert!(!is_native_platform("linux"));
    assert!(!is_native_platform("freebsd"));
}

#[test]
fn the_compat_sheet_exists_only_where_the_filter_was_skipped() {
    // It records what works on the platform Raycast does not support, so on
    // one it does there is nothing for it to say.
    assert!(has_compat_sheet("linux"));
    assert!(!has_compat_sheet("macos"));
    assert!(!has_compat_sheet("windows"));
    for platform in ["linux", "macos", "windows"] {
        assert_ne!(
            has_compat_sheet(platform),
            is_native_platform(platform),
            "{platform} should have a sheet exactly when Raycast is not native to it"
        );
    }
}

#[test]
fn an_extension_gets_a_raycast_store_id() {
    let mut ext = extension("clipboard-history");
    post_process_extension(&mut ext);
    assert_eq!(ext.id, "store.raycast.clipboard-history");
}

#[test]
fn every_command_is_given_its_extensions_icons() {
    // Not sent by the API. A command row has to show something, and carrying
    // the copy on the command is what lets a row render without a reference
    // back to the extension it came from.
    let mut ext = extension("clipboard-history");
    ext.icons = Icons {
        light: Some("light.png".to_owned()),
        dark: Some("dark.png".to_owned()),
    };
    ext.commands = vec![Command::default(), Command::default()];

    post_process_extension(&mut ext);
    for command in &ext.commands {
        assert_eq!(command.extension_icons, ext.icons);
    }
}

#[test]
fn a_commands_own_icons_are_not_overwritten() {
    let mut ext = extension("x");
    ext.icons = Icons {
        light: Some("ext.png".to_owned()),
        dark: None,
    };
    ext.commands = vec![Command {
        icons: Icons {
            light: Some("cmd.png".to_owned()),
            dark: None,
        },
        ..Command::default()
    }];

    post_process_extension(&mut ext);
    assert_eq!(ext.commands[0].icons.light.as_deref(), Some("cmd.png"));
    assert_eq!(
        ext.commands[0].extension_icons.light.as_deref(),
        Some("ext.png")
    );
}

#[test]
fn post_processing_a_list_on_linux_keeps_everything() {
    let mut extensions = vec![extension("a"), extension("b")];
    post_process(&mut extensions, "linux");
    assert_eq!(extensions.len(), 2);
    assert_eq!(extensions[0].id, "store.raycast.a");
}

#[test]
fn post_processing_a_list_on_windows_drops_the_mac_only_ones() {
    let mut extensions = vec![extension("mac-only"), {
        let mut win = extension("cross");
        win.platforms = Some(vec!["macOS".to_owned(), "windows".to_owned()]);
        win
    }];
    post_process(&mut extensions, "windows");
    assert_eq!(extensions.len(), 1);
    assert_eq!(extensions[0].name, "cross");
}

#[test]
fn the_list_path_carries_the_page_and_the_page_size() {
    // per_page, not limit: a different parameter name from the Vicinae store's.
    assert_eq!(
        list_path(ListPaginationOptions {
            page: 2,
            per_page: 25
        }),
        "/store_listings?page=2&per_page=25"
    );
    assert_eq!(
        list_path(ListPaginationOptions::default()),
        "/store_listings?page=1&per_page=50"
    );
}

#[test]
fn an_ordinary_search_makes_the_url_it_always_made() {
    assert_eq!(
        search_path("clipboard"),
        "/store_listings/search?q=clipboard"
    );
}

#[test]
fn a_search_with_an_ampersand_stays_one_parameter() {
    assert_eq!(search_path("a & b"), "/store_listings/search?q=a%20%26%20b");
}

#[test]
fn an_extension_path_escapes_both_halves() {
    // A name containing a slash would otherwise add a path segment and ask
    // the API for something else entirely.
    assert_eq!(
        extension_path("raycast", "clipboard"),
        "/extensions/raycast/clipboard"
    );
    assert_eq!(extension_path("a/b", "c"), "/extensions/a%2Fb/c");
}

#[test]
fn unreserved_characters_are_never_escaped() {
    assert_eq!(encode_path_value("azAZ09-._~"), "azAZ09-._~");
}

#[test]
fn a_page_is_kept_once_it_has_been_fetched() {
    // The listing does not change between keystrokes, and paging back and
    // forth is how it is browsed.
    let mut service = RaycastStoreService::new();
    assert_eq!(service.cached_page(1), None);

    service.store_page(1, vec![extension("a")], "linux");
    assert_eq!(service.cached_page(1).expect("cached").len(), 1);
    assert_eq!(service.cached_page(2), None);
}

#[test]
fn a_stored_page_is_post_processed() {
    let mut service = RaycastStoreService::new();
    service.store_page(1, vec![extension("todo")], "linux");
    assert_eq!(
        service.cached_page(1).expect("cached")[0].id,
        "store.raycast.todo"
    );
}

#[test]
fn the_compat_sheet_is_fetched_once_on_linux_and_never_elsewhere() {
    let mut service = RaycastStoreService::new();
    assert!(service.should_fetch_compat("linux"));
    assert!(!service.should_fetch_compat("macos"));

    service.set_compat(CompatMap::new());
    assert!(
        !service.should_fetch_compat("linux"),
        "a sheet that arrived is not fetched again"
    );
}

#[test]
fn a_failed_compat_fetch_is_not_fatal_and_is_retried() {
    // The C++ warns and returns an empty map rather than an error, leaving
    // m_compatFetched false. The store still works, just without notes.
    let mut service = RaycastStoreService::new();
    let result = service.compat_fetch_failed();

    assert!(result.is_empty());
    assert!(
        service.should_fetch_compat("linux"),
        "a failure must not count as a fetch"
    );
}

#[test]
fn the_compat_sheet_answers_by_extension_name() {
    let mut service = RaycastStoreService::new();
    let mut compat = CompatMap::new();
    compat.insert(
        "clipboard-history".to_owned(),
        CompatInfo {
            status: "works".to_owned(),
            confidence: "high".to_owned(),
            notes: Some(vec!["needs wl-clipboard".to_owned()]),
            has_equivalent: Some(true),
        },
    );
    service.set_compat(compat);

    let info = service.compat_for("clipboard-history").expect("known");
    assert_eq!(info.status, "works");
    assert_eq!(info.confidence, "high");
    assert_eq!(info.has_equivalent, Some(true));
    assert_eq!(service.compat_for("something-else"), None);
}

#[test]
fn a_listing_response_parses_and_post_processes() {
    let json = r#"{
      "data": [{
        "id": "api-id", "name": "clipboard-history",
        "platforms": ["macOS"],
        "store_url": "https://raycast.com/x", "download_url": "https://raycast.com/x.zip",
        "icons": { "light": "l.png", "dark": "d.png" },
        "commands": [{ "id": "c", "name": "list", "title": "List", "mode": "view" }]
      }]
    }"#;
    let parsed: ListApiResponse = serde_json::from_str(json).expect("parses");
    let mut extensions = parsed.data;
    post_process(&mut extensions, "linux");

    assert_eq!(extensions[0].id, "store.raycast.clipboard-history");
    assert_eq!(extensions[0].store_url, "https://raycast.com/x");
    assert_eq!(
        extensions[0].commands[0].extension_icons.light.as_deref(),
        Some("l.png")
    );
}

#[test]
fn a_compat_sheet_parses_with_its_optional_fields_absent() {
    let compat: CompatMap =
        serde_json::from_str(r#"{"x": {"status":"broken","confidence":"low"}}"#).expect("parses");
    let info = compat.get("x").expect("present");
    assert_eq!(info.status, "broken");
    assert_eq!(info.notes, None);
    assert_eq!(info.has_equivalent, None);
}
