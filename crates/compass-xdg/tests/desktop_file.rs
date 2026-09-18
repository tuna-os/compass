//! Desktop file identifiers, and finding an entry by one.
//!
//! Ported from `src/lib/xdgpp/xdgpp/desktop-entry/file.cpp`, whose own tests
//! are in `src/lib/xdgpp/tests/file.cpp`.

use std::path::{Path, PathBuf};

use compass_xdg::desktop_file::{
    DESKTOP_SUFFIX, identify, lookup_candidates, relative_id_dotted, resolve_id, standalone_id,
};
use compass_xdg::desktop_file_id;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

// --- the C++'s own test cases, verbatim ---------------------------------

#[test]
fn a_file_with_no_directory_is_identified_by_its_name() {
    // `DesktopFile::fromFile(FIXTURES / "firefox-bin.desktop", {})` — the C++
    // expects `firefox-bin.desktop`, suffix and all.
    assert_eq!(
        standalone_id(Path::new("/usr/share/applications/firefox-bin.desktop")),
        "firefox-bin.desktop"
    );
}

#[test]
fn a_file_directly_in_the_directory_keeps_its_name() {
    assert_eq!(
        relative_id_dotted(
            Path::new("/usr/share/applications/firefox-bin.desktop"),
            Path::new("/usr/share/applications")
        ),
        "firefox-bin.desktop"
    );
}

#[test]
fn a_nested_file_joins_its_directories_with_dots() {
    // The C++ test: `relativeId("./assets/nested/nested2/firefox-bin.desktop",
    // "./assets")` is `nested.nested2.firefox-bin.desktop`.
    assert_eq!(
        relative_id_dotted(
            Path::new("./assets/nested/nested2/firefox-bin.desktop"),
            Path::new("./assets")
        ),
        "nested.nested2.firefox-bin.desktop"
    );
}

// --- the two schemes disagree -------------------------------------------

#[test]
fn the_two_id_schemes_agree_on_a_flat_file() {
    let file = p("/usr/share/applications/firefox.desktop");
    let dir = p("/usr/share/applications");
    assert_eq!(
        relative_id_dotted(&file, &dir),
        desktop_file_id(&dir, &file).expect("an id")
    );
}

#[test]
fn the_two_id_schemes_disagree_on_a_nested_file() {
    // The specification says `/` becomes `-`; the C++ turns it into `.`. The
    // id is the key an application's frecency score, alias and enabled state
    // are stored under, so the two engines would not find each other's
    // records for any nested application. This test exists to make that
    // disagreement impossible to change by accident in either direction.
    let file = p("/usr/share/applications/kde4/konsole.desktop");
    let dir = p("/usr/share/applications");
    assert_eq!(relative_id_dotted(&file, &dir), "kde4.konsole.desktop");
    assert_eq!(
        desktop_file_id(&dir, &file).as_deref(),
        Some("kde4-konsole.desktop")
    );
}

#[test]
fn the_dotted_scheme_makes_the_suffix_indistinguishable_from_a_separator() {
    // `kde4.konsole.desktop` could be a nested `konsole` or a flat file of
    // that exact name, and nothing recovers the difference.
    let nested = relative_id_dotted(Path::new("/apps/kde4/konsole.desktop"), Path::new("/apps"));
    let flat = relative_id_dotted(Path::new("/apps/kde4.konsole.desktop"), Path::new("/apps"));
    assert_eq!(nested, flat);
}

#[test]
fn the_suffix_is_part_of_the_id_and_not_stripped() {
    assert!(
        relative_id_dotted(Path::new("/apps/firefox.desktop"), Path::new("/apps"))
            .ends_with(DESKTOP_SUFFIX)
    );
}

// --- relative paths -----------------------------------------------------

#[test]
fn several_levels_of_nesting_all_become_dots() {
    assert_eq!(
        relative_id_dotted(Path::new("/a/b/c/d/e.desktop"), Path::new("/a")),
        "b.c.d.e.desktop"
    );
}

#[test]
fn a_file_outside_the_directory_walks_up_with_dot_dot() {
    // `lexically_relative` is textual and produces `..` components; they end
    // up in the id as dots, which is nonsense as an id but is what the C++
    // produces and what a caller passing the wrong directory would see.
    assert_eq!(
        relative_id_dotted(Path::new("/a/x.desktop"), Path::new("/a/b")),
        "...x.desktop"
    );
}

#[test]
fn a_leading_dot_slash_is_not_a_component() {
    assert_eq!(
        relative_id_dotted(Path::new("./a/x.desktop"), Path::new("./a")),
        "x.desktop"
    );
}

#[test]
fn a_dot_component_on_only_one_side_still_cancels() {
    // With `./a` on both sides the leading dots cancel each other and a naive
    // implementation looks right. The case that shows whether `.` is dropped
    // is one where only one side carries it.
    assert_eq!(
        relative_id_dotted(Path::new("a/x.desktop"), Path::new("./a")),
        "x.desktop"
    );
    assert_eq!(
        relative_id_dotted(Path::new("./a/x.desktop"), Path::new("a")),
        "x.desktop"
    );
}

#[test]
fn a_trailing_separator_on_the_directory_changes_nothing() {
    assert_eq!(
        relative_id_dotted(Path::new("/a/x.desktop"), Path::new("/a/")),
        "x.desktop"
    );
}

// --- looking an id up ---------------------------------------------------

#[test]
fn an_id_is_tried_as_given_and_then_with_the_suffix() {
    // Which is what lets a lookup accept both `firefox` and
    // `firefox.desktop`.
    assert_eq!(
        lookup_candidates(Path::new("/apps"), "firefox"),
        [p("/apps/firefox"), p("/apps/firefox.desktop")]
    );
}

#[test]
fn an_id_that_already_ends_in_the_suffix_is_never_given_a_second_one() {
    // The first candidate matches, so `firefox.desktop.desktop` is never
    // reached.
    let found = resolve_id("firefox.desktop", &[p("/apps")], |path| {
        path == Path::new("/apps/firefox.desktop")
    });
    assert_eq!(found, Some(p("/apps/firefox.desktop")));
}

#[test]
fn a_bare_id_finds_the_suffixed_file() {
    let found = resolve_id("firefox", &[p("/apps")], |path| {
        path == Path::new("/apps/firefox.desktop")
    });
    assert_eq!(found, Some(p("/apps/firefox.desktop")));
}

#[test]
fn a_file_named_without_the_suffix_is_found_too() {
    let found = resolve_id("firefox", &[p("/apps")], |path| {
        path == Path::new("/apps/firefox")
    });
    assert_eq!(found, Some(p("/apps/firefox")));
}

#[test]
fn both_candidates_are_tried_in_one_directory_before_the_next() {
    // So a `firefox.desktop` in an earlier directory wins over a `firefox` in
    // a later one — which is what makes the search order a precedence, and is
    // how a user's own applications directory shadows the system's.
    let dirs = [p("/home/me/apps"), p("/usr/share/applications")];
    let found = resolve_id("firefox", &dirs, |path| {
        path == Path::new("/home/me/apps/firefox.desktop")
            || path == Path::new("/usr/share/applications/firefox")
    });
    assert_eq!(found, Some(p("/home/me/apps/firefox.desktop")));
}

#[test]
fn a_later_directory_is_reached_when_an_earlier_one_has_nothing() {
    let dirs = [p("/home/me/apps"), p("/usr/share/applications")];
    let found = resolve_id("firefox", &dirs, |path| {
        path == Path::new("/usr/share/applications/firefox.desktop")
    });
    assert_eq!(found, Some(p("/usr/share/applications/firefox.desktop")));
}

#[test]
fn an_id_that_matches_nothing_is_not_found() {
    assert_eq!(resolve_id("nope", &[p("/apps")], |_| false), None);
}

#[test]
fn no_directories_means_nothing_is_found() {
    assert_eq!(resolve_id("firefox", &[], |_| true), None);
}

// --- identifying a file -------------------------------------------------

#[test]
fn identifying_with_a_directory_uses_the_relative_id() {
    let found = identify(
        Path::new("/apps/kde4/konsole.desktop"),
        Some(Path::new("/apps")),
    );
    assert_eq!(found.id, "kde4.konsole.desktop");
    assert_eq!(found.path, p("/apps/kde4/konsole.desktop"));
}

#[test]
fn identifying_without_one_uses_the_filename() {
    let found = identify(Path::new("/apps/kde4/konsole.desktop"), None);
    assert_eq!(found.id, "konsole.desktop");
}

#[test]
fn two_files_of_the_same_name_collide_when_identified_without_a_directory() {
    // Which is why the application database always passes one.
    let a = identify(Path::new("/a/konsole.desktop"), None);
    let b = identify(Path::new("/b/konsole.desktop"), None);
    assert_eq!(a.id, b.id);
    assert_ne!(a.path, b.path);
}
