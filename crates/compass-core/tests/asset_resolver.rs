//! The relative asset resolver, read against
//! `src/server/src/services/asset-resolver/asset-resolver.hpp`.

use std::path::{Path, PathBuf};

use compass_core::asset_resolver::AssetResolver;

#[test]
fn resolving_takes_the_first_base_that_has_the_file() {
    // `for (base : m_paths) { if (exists(base / relative)) return ...; }`
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/a");
    resolver.add("/ext/b");

    let found = resolver.resolve_with("icon.png", |path| path == Path::new("/ext/b/icon.png"));
    assert_eq!(found, Some(PathBuf::from("/ext/b/icon.png")));

    let both = resolver.resolve_with("icon.png", |_| true);
    assert_eq!(
        both,
        Some(PathBuf::from("/ext/a/icon.png")),
        "the first base wins when both have it"
    );
}

#[test]
fn an_unresolvable_asset_is_none_rather_than_a_guess() {
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/a");

    assert_eq!(resolver.resolve_with("missing.png", |_| false), None);
}

#[test]
fn an_empty_resolver_resolves_nothing() {
    assert_eq!(
        AssetResolver::new().resolve_with("icon.png", |_| true),
        None
    );
}

#[test]
fn the_search_order_is_the_order_paths_were_added() {
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/first");
    resolver.add("/ext/second");
    resolver.add("/ext/third");

    assert_eq!(
        resolver.paths(),
        [
            PathBuf::from("/ext/first"),
            PathBuf::from("/ext/second"),
            PathBuf::from("/ext/third"),
        ]
    );
}

#[test]
fn adding_the_same_path_twice_keeps_two_entries() {
    // `addPath` does not deduplicate, and `removePath` erases one match. That
    // pair is what lets two commands share an asset directory: the first to
    // unload does not blind the other.
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/shared");
    resolver.add("/ext/shared");
    assert_eq!(resolver.paths().len(), 2);

    resolver.remove("/ext/shared");
    assert_eq!(resolver.paths().len(), 1, "one command unloaded");
    assert_eq!(
        resolver.resolve_with("icon.png", |_| true),
        Some(PathBuf::from("/ext/shared/icon.png")),
        "the other can still find its assets"
    );

    resolver.remove("/ext/shared");
    assert!(resolver.paths().is_empty());
}

#[test]
fn removing_a_path_that_was_never_added_does_nothing() {
    // `if (it != m_paths.end())` -- an unload for a path that was never
    // registered is silent rather than an error.
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/a");

    resolver.remove("/ext/b");
    assert_eq!(resolver.paths(), [PathBuf::from("/ext/a")]);
}

#[test]
fn removing_takes_the_first_match_and_leaves_the_order_otherwise_intact() {
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/a");
    resolver.add("/ext/b");
    resolver.add("/ext/a");

    resolver.remove("/ext/a");
    assert_eq!(
        resolver.paths(),
        [PathBuf::from("/ext/b"), PathBuf::from("/ext/a")],
        "the first /ext/a went, the rest kept their order"
    );
}

#[test]
fn a_nested_relative_path_is_joined_onto_each_base() {
    let mut resolver = AssetResolver::new();
    resolver.add("/ext/a");

    assert_eq!(
        resolver.resolve_with("images/icon.png", |_| true),
        Some(PathBuf::from("/ext/a/images/icon.png"))
    );
}

#[test]
fn resolving_against_the_real_filesystem_finds_a_real_file() {
    // `resolve` is `resolve_with(exists)`, and `std::filesystem::exists` is
    // true for a directory too.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("second/images")).expect("mkdir");
    std::fs::write(dir.path().join("second/images/icon.png"), b"x").expect("write");

    let mut resolver = AssetResolver::new();
    resolver.add(dir.path().join("first"));
    resolver.add(dir.path().join("second"));

    assert_eq!(
        resolver.resolve("images/icon.png"),
        Some(dir.path().join("second/images/icon.png"))
    );
    assert_eq!(
        resolver.resolve("images"),
        Some(dir.path().join("second/images")),
        "a directory resolves too"
    );
    assert_eq!(resolver.resolve("nope.png"), None);
}
