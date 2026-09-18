//! Installing an extension bundle onto disk.
//!
//! Ported from `ExtensionRegistry::installFromZip`.

use std::path::PathBuf;

use compass_core::extension_install::{
    InstallFailure, MANIFEST_NAME, STORE_ID_PREFIX, STRIP_COMPONENTS, Step, cleanup_after,
    install_dir, install_steps, may_have_removed_previous, staging_dir, store_extension_id,
};

fn dir() -> PathBuf {
    PathBuf::from("/home/me/.local/share/vicinae/extensions")
}

fn steps() -> Vec<Step> {
    install_steps(&dir(), "store.raycast.slack")
}

/// The index of the first step matching a predicate.
fn position(steps: &[Step], f: impl Fn(&Step) -> bool) -> usize {
    steps.iter().position(f).expect("the step is there")
}

// --- naming -------------------------------------------------------------

#[test]
fn a_store_extension_is_filed_under_its_own_prefix() {
    // So one cannot collide with a locally developed extension of the same
    // name.
    assert_eq!(store_extension_id("slack"), "store.raycast.slack");
    assert_eq!(STORE_ID_PREFIX, "store.raycast.");
}

#[test]
fn the_staging_directory_sits_beside_the_target() {
    // The last step is a rename, and a rename across filesystems is a copy
    // that can fail halfway.
    let staging = staging_dir(&dir(), "x");
    let target = install_dir(&dir(), "x");
    assert_eq!(staging.parent(), target.parent());
}

#[test]
fn the_staging_directory_is_hidden_from_the_registrys_listing() {
    let staging = staging_dir(&dir(), "x");
    let name = staging
        .file_name()
        .and_then(|n| n.to_str())
        .expect("a name");
    assert!(name.starts_with('.'), "{name}");
    assert_eq!(name, ".staging-x");
}

#[test]
fn the_staging_and_install_directories_are_never_the_same() {
    assert_ne!(staging_dir(&dir(), "x"), install_dir(&dir(), "x"));
}

// --- the order of the steps ---------------------------------------------

#[test]
fn the_staging_directory_is_cleared_before_anything_is_unpacked() {
    // A previous install that died part-way leaves one behind, and unpacking
    // over it would mix two extensions into one.
    let steps = steps();
    let cleared = position(
        &steps,
        |s| matches!(s, Step::RemoveAll(path) if path == &staging_dir(&dir(), "store.raycast.slack")),
    );
    let extracted = position(&steps, |s| matches!(s, Step::Extract { .. }));
    assert!(cleared < extracted, "{steps:?}");
}

#[test]
fn the_archive_is_unpacked_into_staging_and_never_the_target() {
    let steps = steps();
    let Step::Extract { into, .. } =
        &steps[position(&steps, |s| matches!(s, Step::Extract { .. }))]
    else {
        panic!("no extract step");
    };
    assert_eq!(into, &staging_dir(&dir(), "store.raycast.slack"));
    assert_ne!(into, &install_dir(&dir(), "store.raycast.slack"));
}

#[test]
fn the_bundles_wrapping_directory_is_stripped() {
    // A published bundle wraps everything in one top-level directory named
    // after the extension; without stripping it, every extension installs one
    // level too deep and its manifest is never found.
    let steps = steps();
    let Step::Extract {
        strip_components, ..
    } = &steps[position(&steps, |s| matches!(s, Step::Extract { .. }))]
    else {
        panic!("no extract step");
    };
    assert_eq!(*strip_components, 1);
    assert_eq!(STRIP_COMPONENTS, 1);
}

#[test]
fn the_manifest_is_checked_in_staging() {
    let steps = steps();
    let Step::RequireManifest(path) =
        &steps[position(&steps, |s| matches!(s, Step::RequireManifest(_)))]
    else {
        panic!("no check step");
    };
    assert_eq!(
        path,
        &staging_dir(&dir(), "store.raycast.slack").join(MANIFEST_NAME)
    );
    assert_eq!(MANIFEST_NAME, "package.json");
}

#[test]
fn the_manifest_is_checked_before_the_target_is_touched() {
    // THIS IS THE PROPERTY THE WHOLE SHAPE EXISTS FOR. A truncated or wrong
    // archive is discarded with the installed version untouched.
    let steps = steps();
    let checked = position(&steps, |s| matches!(s, Step::RequireManifest(_)));
    let target_removed = position(
        &steps,
        |s| matches!(s, Step::RemoveAll(path) if path == &install_dir(&dir(), "store.raycast.slack")),
    );
    assert!(
        checked < target_removed,
        "the installed version must not be removed before the replacement is known good: {steps:?}"
    );
}

#[test]
fn the_target_is_removed_before_the_rename() {
    // A rename onto an existing directory does not replace it.
    let steps = steps();
    let removed = position(
        &steps,
        |s| matches!(s, Step::RemoveAll(path) if path == &install_dir(&dir(), "store.raycast.slack")),
    );
    let renamed = position(&steps, |s| matches!(s, Step::Rename { .. }));
    assert!(removed < renamed, "{steps:?}");
}

#[test]
fn the_rename_moves_staging_into_place_and_is_last() {
    let steps = steps();
    let Some(Step::Rename { from, to }) = steps.last() else {
        panic!("the rename is not last: {steps:?}");
    };
    assert_eq!(from, &staging_dir(&dir(), "store.raycast.slack"));
    assert_eq!(to, &install_dir(&dir(), "store.raycast.slack"));
}

#[test]
fn there_are_exactly_five_steps() {
    // Pinned so a step cannot be added in the middle without a test saying
    // where it goes — the order is the safety.
    assert_eq!(steps().len(), 5);
}

// --- cleaning up after a failure ----------------------------------------

#[test]
fn an_unreadable_archive_touches_nothing() {
    // The C++ returns before reaching the disk at all.
    assert!(cleanup_after(&InstallFailure::UnreadableArchive, &dir(), "x").is_empty());
}

#[test]
fn a_missing_manifest_clears_only_the_staging_directory() {
    // Reaching for the target here is how a cleanup path deletes a working
    // extension because its replacement was broken.
    let cleanup = cleanup_after(&InstallFailure::NoManifest, &dir(), "x");
    assert_eq!(cleanup, [Step::RemoveAll(staging_dir(&dir(), "x"))]);
}

#[test]
fn a_failed_rename_clears_only_the_staging_directory_too() {
    let cleanup = cleanup_after(&InstallFailure::RenameFailed, &dir(), "x");
    assert_eq!(cleanup, [Step::RemoveAll(staging_dir(&dir(), "x"))]);
}

#[test]
fn no_cleanup_path_ever_touches_the_install_directory() {
    for failure in [
        InstallFailure::UnreadableArchive,
        InstallFailure::NoManifest,
        InstallFailure::RenameFailed,
    ] {
        for step in cleanup_after(&failure, &dir(), "x") {
            let Step::RemoveAll(path) = step else {
                panic!("cleanup should only remove");
            };
            assert_ne!(path, install_dir(&dir(), "x"), "{failure:?}");
        }
    }
}

// --- what a failure means -----------------------------------------------

#[test]
fn only_a_failed_rename_can_have_lost_the_previous_version() {
    // It is the one failure that happens after the target is removed. The
    // difference matters: "the install did not happen" and "the extension is
    // now missing" send someone looking in different places.
    assert!(may_have_removed_previous(&InstallFailure::RenameFailed));
    assert!(!may_have_removed_previous(&InstallFailure::NoManifest));
    assert!(!may_have_removed_previous(
        &InstallFailure::UnreadableArchive
    ));
}

#[test]
fn a_rejected_bundle_leaves_the_installed_version_alone() {
    // Stated as the round trip a caller actually cares about: the steps that
    // ran before a missing manifest, plus the cleanup, never mention the
    // install directory.
    let steps = steps();
    let checked = position(&steps, |s| matches!(s, Step::RequireManifest(_)));
    let ran: &[Step] = &steps[..=checked];
    let target = install_dir(&dir(), "store.raycast.slack");

    for step in ran
        .iter()
        .chain(cleanup_after(&InstallFailure::NoManifest, &dir(), "store.raycast.slack").iter())
    {
        match step {
            Step::RemoveAll(path) => assert_ne!(path, &target),
            Step::Rename { to, .. } => assert_ne!(to, &target),
            Step::Extract { into, .. } => assert_ne!(into, &target),
            Step::RequireManifest(path) => assert!(!path.starts_with(&target)),
        }
    }
}

#[test]
fn two_extensions_stage_in_different_places() {
    // Two installs running at once must not unpack over each other.
    assert_ne!(staging_dir(&dir(), "a"), staging_dir(&dir(), "b"));
}

#[test]
fn the_steps_name_the_extension_they_are_for() {
    let slack = install_steps(&dir(), "store.raycast.slack");
    let figma = install_steps(&dir(), "store.raycast.figma");
    assert_ne!(slack, figma);
}
