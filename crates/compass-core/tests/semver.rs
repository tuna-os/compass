//! What counts as a version, and which of two is newer.
//!
//! Read off `vicinae::Semver` (`src/server/src/services/update/semver.hpp`).

use compass_core::semver::Semver;

/// Parse, or panic with what failed.
fn v(text: &str) -> Semver {
    Semver::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
}

#[test]
fn a_release_tag_parses_with_or_without_its_v() {
    assert_eq!(v("v1.2.3").components, vec![1, 2, 3]);
    assert_eq!(v("1.2.3").components, vec![1, 2, 3]);
}

#[test]
fn only_one_leading_v_is_stripped() {
    assert_eq!(Semver::parse("vv1.2.3"), None);
}

#[test]
fn a_single_number_is_a_version() {
    assert_eq!(v("7").components, vec![7]);
}

#[test]
fn leading_zeroes_are_just_zeroes() {
    assert_eq!(v("1.02.003").components, vec![1, 2, 3]);
}

#[test]
fn a_prerelease_tag_does_not_parse_at_all() {
    // This is the load-bearing one. It is what keeps release candidates from
    // being offered as updates without anybody having to filter them, so it
    // must not be "fixed" into real semver.
    assert_eq!(Semver::parse("v1.2.3-rc1"), None);
    assert_eq!(Semver::parse("v1.2.3+build7"), None);
    assert_eq!(Semver::parse("v1.2.3beta"), None);
}

#[test]
fn a_nightly_or_branch_tag_does_not_parse() {
    assert_eq!(Semver::parse("nightly"), None);
    assert_eq!(Semver::parse("main"), None);
    assert_eq!(Semver::parse(""), None);
    assert_eq!(Semver::parse("v"), None);
}

#[test]
fn a_dot_with_no_number_beside_it_does_not_parse() {
    assert_eq!(Semver::parse(".1.2"), None, "leading dot");
    assert_eq!(Semver::parse("1.2."), None, "trailing dot");
    assert_eq!(Semver::parse("1..2"), None, "empty component");
    assert_eq!(Semver::parse("."), None);
}

#[test]
fn whitespace_is_not_tolerated() {
    assert_eq!(Semver::parse(" 1.2.3"), None);
    assert_eq!(Semver::parse("1.2.3 "), None);
}

#[test]
fn a_higher_component_wins_wherever_it_sits() {
    assert!(v("2.0.0") > v("1.9.9"));
    assert!(v("1.10.0") > v("1.9.0"), "ten is after nine, not before it");
    assert!(v("1.0.1") > v("1.0.0"));
}

#[test]
fn a_missing_component_counts_as_zero() {
    // So 1.0 and 1.0.0 are the same release. A comparison that ordered by
    // length would offer 1.0.0 as an update over 1.0, for ever.
    assert_eq!(v("1.0"), v("1.0.0"));
    assert_eq!(v("1"), v("1.0.0.0"));
    assert!(v("1.0.1") > v("1.0"));
    assert!(v("1.0") < v("1.0.1"));
}

#[test]
fn a_version_is_not_newer_than_itself() {
    assert_eq!(v("1.2.3").cmp(&v("1.2.3")), std::cmp::Ordering::Equal);
    assert_eq!(v("1.2.3"), v("v1.2.3"), "the v is not part of the version");
}

#[test]
fn a_component_too_large_to_hold_is_refused_rather_than_wrapped() {
    // The C++ accumulates into an unsigned with no overflow check, so
    // 4294967296.0.0 wraps to 0.0.0 and a new release looks like no release.
    // See PARITY.md — this returns None, so the tag is "not a release tag".
    assert_eq!(Semver::parse("4294967296.0.0"), None);
    assert_eq!(v("4294967295").components, vec![u32::MAX], "one less fits");
}

#[test]
fn ordering_is_total_and_consistent() {
    let mut versions = vec![v("1.10.0"), v("1.2.0"), v("2.0.0"), v("1.2"), v("1.2.1")];
    versions.sort();
    assert!(
        versions.windows(2).all(|pair| pair[0] <= pair[1]),
        "sorted: {versions:?}"
    );
    // The two equal ones end up adjacent; which of them comes first is the
    // sort's business, not this port's, so it is not asserted.
    assert_eq!(versions.first().expect("non-empty").components[0], 1);
    assert_eq!(versions.last().expect("non-empty"), &v("2.0.0"));
    assert_eq!(versions[3], v("1.10.0"), "ten sorts after two");
}

#[test]
fn equality_agrees_with_the_comparison() {
    // The C++ defines operator== in terms of <=>. Comparing component lists
    // structurally instead would make 1.0 and 1.0.0 unequal but neither
    // greater — a contradiction a sort or a hash map can act on.
    use std::collections::HashSet;

    let a = v("1.0");
    let b = v("1.0.0");
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);

    let mut set = HashSet::new();
    set.insert(a);
    assert!(
        set.contains(&b),
        "equal versions must hash alike or a set holds both"
    );
}
