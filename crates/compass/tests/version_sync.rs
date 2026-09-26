//! The release tag and the shipped binary report the same version.
//!
//! `scripts/bump_version.sh` writes the release tag into the repository-root
//! `manifest.yaml`; `compass --version` reports the workspace
//! `CARGO_PKG_VERSION` through clap's `version`. When the two drift, a release
//! ships a binary that misreports itself, and version-derived artifact names
//! (AppImage file, Homebrew formula, Nix derivation) disagree with the tag.
//! This test fails while they disagree.

/// The `tag:` value under `release:` in the repository-root `manifest.yaml`,
/// parsed as text: the file's shape is fixed by `scripts/bump_version.sh`,
/// which is not worth a YAML dependency in the test.
fn manifest_tag() -> String {
    let manifest = include_str!("../../../manifest.yaml");
    manifest
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("tag:"))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("manifest.yaml must carry a release tag")
}

#[test]
fn binary_version_matches_release_tag() {
    let tag = manifest_tag();
    let bare = tag.strip_prefix('v').unwrap_or(&tag);
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        bare,
        "workspace version must match the manifest.yaml release tag; bump both with scripts/bump_version.sh"
    );
}
