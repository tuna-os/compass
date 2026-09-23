//! Which directories the indexer is asked to scan.
//!
//! Ported from `file-indexer/util.hpp`.

use std::path::{Path, PathBuf};

use compass_core::scan_roots::{
    compact_subtrees, file_size_for, is_covered_by_any, is_same_or_descendant_of, lexically_normal,
    normalize_path, normalize_paths,
};

fn paths(items: &[PathBuf]) -> Vec<String> {
    items
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn owned(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

#[test]
fn a_path_is_its_own_ancestor() {
    assert!(is_same_or_descendant_of(
        Path::new("/home/user"),
        Path::new("/home/user")
    ));
}

#[test]
fn a_child_is_a_descendant() {
    assert!(is_same_or_descendant_of(
        Path::new("/home/user/code"),
        Path::new("/home/user")
    ));
}

#[test]
fn a_parent_is_not_a_descendant_of_its_child() {
    assert!(!is_same_or_descendant_of(
        Path::new("/home"),
        Path::new("/home/user")
    ));
}

#[test]
fn a_sibling_whose_name_starts_the_same_is_not_a_descendant() {
    // `/home/user2` starts with the characters of `/home/user`. A string
    // prefix test says yes and drops one of the two from the scan set -- a bug
    // that only appears for the user whose name is a prefix of someone else's.
    assert!(!is_same_or_descendant_of(
        Path::new("/home/user2"),
        Path::new("/home/user")
    ));
}

#[test]
fn a_path_under_a_similarly_named_sibling_is_not_a_descendant() {
    assert!(!is_same_or_descendant_of(
        Path::new("/home/user2/code"),
        Path::new("/home/user")
    ));
}

#[test]
fn everything_is_under_the_filesystem_root() {
    assert!(is_same_or_descendant_of(
        Path::new("/home/user"),
        Path::new("/")
    ));
}

#[test]
fn coverage_asks_every_root() {
    let roots = owned(&["/opt", "/home/user"]);

    assert!(is_covered_by_any(Path::new("/home/user/code"), &roots));
    assert!(!is_covered_by_any(Path::new("/var/log"), &roots));
}

#[test]
fn nothing_is_covered_by_no_roots() {
    assert!(!is_covered_by_any(Path::new("/home/user"), &[]));
}

#[test]
fn a_root_inside_another_is_dropped() {
    let compacted = compact_subtrees(owned(&["/home/user", "/home/user/code"]));

    assert_eq!(paths(&compacted), ["/home/user"]);
}

#[test]
fn the_ancestor_survives_whichever_order_it_arrives_in() {
    let compacted = compact_subtrees(owned(&["/home/user/code", "/home/user"]));

    assert_eq!(paths(&compacted), ["/home/user"]);
}

#[test]
fn a_deep_descendant_is_dropped_even_when_it_sorts_before_a_shallower_root() {
    // The reason the sort is by component count rather than alphabetical:
    // `/a/b/c` sorts before `/a/bb`, so under a plain sort it would be weighed
    // against a set that did not yet hold `/a/b`.
    let compacted = compact_subtrees(owned(&["/a/b/c", "/a/bb", "/a/b"]));

    assert_eq!(paths(&compacted), ["/a/b", "/a/bb"]);
}

#[test]
fn unrelated_roots_all_survive() {
    let compacted = compact_subtrees(owned(&["/opt", "/home/user", "/var/log"]));

    assert_eq!(paths(&compacted), ["/opt", "/home/user", "/var/log"]);
}

#[test]
fn roots_of_equal_depth_come_back_in_a_stable_order() {
    // Two runs over the same settings must scan the same directories in the
    // same order, or a scan interrupted halfway covers a different half.
    let compacted = compact_subtrees(owned(&["/b/two", "/a/one", "/a/two"]));

    assert_eq!(paths(&compacted), ["/a/one", "/a/two", "/b/two"]);
}

#[test]
fn a_repeated_root_appears_once() {
    let compacted = compact_subtrees(owned(&["/home/user", "/home/user"]));

    assert_eq!(paths(&compacted), ["/home/user"]);
}

#[test]
fn a_root_of_everything_swallows_the_rest() {
    let compacted = compact_subtrees(owned(&["/home/user", "/", "/opt"]));

    assert_eq!(paths(&compacted), ["/"]);
}

#[test]
fn a_sibling_with_a_shared_prefix_is_kept() {
    let compacted = compact_subtrees(owned(&["/home/user", "/home/user2"]));

    assert_eq!(paths(&compacted), ["/home/user", "/home/user2"]);
}

#[test]
fn a_single_path_normalises_absolute_against_the_working_directory() {
    let absolute = normalize_path(Path::new("/home/user/./code"));
    assert_eq!(absolute.to_string_lossy(), "/home/user/code");

    let cwd = std::env::current_dir().expect("working directory");
    assert_eq!(
        normalize_path(Path::new("code")),
        cwd.join("code"),
        "relative roots resolve under the working directory, like fs::absolute"
    );
}

#[test]
fn normalising_resolves_dots_without_touching_the_disk() {
    // Compared as text, not as paths: Rust's `Components` drops `.` on its own,
    // so `PathBuf` equality cannot tell a normalised path from an unnormalised
    // one and a control said so.
    assert_eq!(
        lexically_normal(Path::new("/home/user/./code/../code")).to_string_lossy(),
        "/home/user/code"
    );
}

#[test]
fn a_leading_current_directory_is_dropped() {
    // The only `.` the code ever sees. Rust's `Components` normalises interior
    // ones away before the match arm is reached -- a control on that arm stayed
    // silent until this test existed, because nothing else could reach it.
    assert_eq!(
        lexically_normal(Path::new("./code")).to_string_lossy(),
        "code"
    );
}

#[test]
fn normalising_leaves_a_leading_parent_alone() {
    // With nothing above it to cancel, `..` is kept rather than swallowed.
    assert_eq!(
        lexically_normal(Path::new("../code")).to_string_lossy(),
        "../code"
    );
}

#[test]
fn normalising_does_not_climb_above_the_root() {
    assert_eq!(
        lexically_normal(Path::new("/../opt")).to_string_lossy(),
        "/opt"
    );
}

#[test]
fn normalising_makes_relative_paths_absolute_and_drops_duplicates() {
    let normalized = normalize_paths(owned(&["code", "/home/user/code", "docs"]), |path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            Path::new("/home/user").join(path)
        }
    });

    assert_eq!(paths(&normalized), ["/home/user/code", "/home/user/docs"]);
}

#[test]
fn normalising_sorts_so_the_answer_does_not_depend_on_the_settings_order() {
    let normalized = normalize_paths(owned(&["/b", "/a"]), |path| path.to_path_buf());

    assert_eq!(paths(&normalized), ["/a", "/b"]);
}

#[test]
fn two_spellings_of_one_path_collapse_to_one() {
    let normalized = normalize_paths(owned(&["/home/user/code", "/home/user/./code"]), |path| {
        path.to_path_buf()
    });

    assert_eq!(paths(&normalized), ["/home/user/code"]);
}

#[test]
fn a_path_written_with_a_dot_is_stored_without_one() {
    // The one that survives a dedupe is whichever sorted first, so the test
    // above passes even when nothing is normalised. This one has only the
    // unnormalised spelling to offer.
    let normalized = normalize_paths(owned(&["/home/user/./code"]), |path| path.to_path_buf());

    assert_eq!(paths(&normalized), ["/home/user/code"]);
}

#[test]
fn a_directory_has_no_size_rather_than_a_size_of_zero() {
    // Its entry's size on disk is a filesystem detail, and recording it lets a
    // search for large files return directories.
    assert_eq!(file_size_for(Some(4096), true), None);
}

#[test]
fn a_file_records_its_size() {
    assert_eq!(file_size_for(Some(1234), false), Some(1234));
}

#[test]
fn an_empty_file_records_zero_rather_than_nothing() {
    assert_eq!(file_size_for(Some(0), false), Some(0));
}

#[test]
fn a_size_that_could_not_be_read_is_absent() {
    assert_eq!(file_size_for(None, false), None);
}

#[test]
fn a_size_too_large_for_the_column_is_absent_rather_than_negative() {
    // The column is signed; a wrapped value reads as a negative size, which is
    // a worse answer than an absent one.
    assert_eq!(file_size_for(Some(u64::MAX), false), None);
    assert_eq!(file_size_for(Some(i64::MAX as u64), false), Some(i64::MAX));
}
