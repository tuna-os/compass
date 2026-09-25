//! The bug-report link and the fallback manager.
//!
//! Ported from `src/server/src/builtins/vicinae/bug-report-url.hpp` and
//! `manage-fallback-*.cpp`.

use compass_core::bug_report::{
    CREATE_EXTENSION_ISSUE_URL, CREATE_ISSUE_URL, FallbackSection, ISSUE_TEMPLATE, ISSUE_TYPE,
    SystemInfo, fallback_action_label, fallback_sections, issue_body, issue_query, order_enabled,
    os_description,
};

fn info() -> SystemInfo {
    SystemInfo {
        version: "v0.14.2".to_owned(),
        commit: "abc1234".to_owned(),
        build_info: "Release".to_owned(),
        provenance: "nixpkgs".to_owned(),
        os: "Bluefin - 42".to_owned(),
        qt_platform: "wayland".to_owned(),
        desktop: "GNOME".to_owned(),
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

// --- where a report goes ------------------------------------------------

#[test]
fn a_launcher_bug_goes_to_the_launchers_tracker() {
    assert_eq!(
        CREATE_ISSUE_URL,
        "https://github.com/tuna-os/compass/issues/new"
    );
}

#[test]
fn an_extension_bug_goes_to_a_different_repository_by_a_different_path() {
    // `/issues/new/choose` rather than `/issues/new`, because that repository
    // offers templates and the launcher's does not.
    assert_eq!(
        CREATE_EXTENSION_ISSUE_URL,
        "https://github.com/vicinaehq/extensions/issues/new/choose"
    );
    assert!(CREATE_EXTENSION_ISSUE_URL.ends_with("/choose"));
    assert!(!CREATE_ISSUE_URL.ends_with("/choose"));
}

// --- the pre-filled body ------------------------------------------------

#[test]
fn the_template_asks_for_the_seven_system_fields() {
    // These are the ones nobody remembers to include and everybody is asked
    // for, so pre-filling them is most of what this command does.
    for field in [
        "{version}",
        "{commit}",
        "{build_info}",
        "{provenance}",
        "{os}",
        "{qt_platform}",
        "{desktop}",
    ] {
        assert!(ISSUE_TEMPLATE.contains(field), "missing {field}");
    }
}

#[test]
fn the_template_keeps_its_five_prose_sections() {
    for heading in [
        "**System information**",
        "**Describe the bug**",
        "**To Reproduce**",
        "**Expected behavior**",
        "**Screenshots**",
        "**Additional context**",
    ] {
        assert!(ISSUE_TEMPLATE.contains(heading), "missing {heading}");
    }
}

#[test]
fn the_template_was_copied_rather_than_retyped() {
    // Its measurements, as taken from the C++ raw string literal with the
    // seven `%n` placeholders renamed.
    assert_eq!(ISSUE_TEMPLATE.len(), 531);
    assert_eq!(ISSUE_TEMPLATE.lines().count(), 28);
}

#[test]
fn every_field_is_filled_in() {
    let body = issue_body(&info());
    assert!(body.contains("- Version: v0.14.2 (abc1234)"));
    assert!(body.contains("- Build info: Release"));
    assert!(body.contains("- Provenance: nixpkgs"));
    assert!(body.contains("- OS: Bluefin - 42"));
    assert!(body.contains("- QT Platform: wayland"));
    assert!(body.contains("- DE: GNOME"));
}

#[test]
fn no_placeholder_survives_into_the_report() {
    let body = issue_body(&info());
    assert!(!body.contains('{'), "{body}");
}

#[test]
fn an_unknown_field_leaves_an_empty_slot_rather_than_a_placeholder() {
    // Someone filing a report should not have to explain what `{provenance}`
    // meant.
    let body = issue_body(&SystemInfo::default());
    assert!(!body.contains('{'));
    assert!(body.contains("- Provenance: \n"));
}

// --- describing the system ----------------------------------------------

#[test]
fn os_release_gives_the_distributions_own_name() {
    // Which is the thing a maintainer actually needs.
    assert_eq!(
        os_description(Some(("Bluefin", "42")), "Linux", "x86_64"),
        "Bluefin - 42"
    );
}

#[test]
fn without_os_release_the_kernel_and_architecture_are_used() {
    assert_eq!(os_description(None, "Linux", "x86_64"), "Linux (x86_64)");
}

#[test]
fn the_two_formats_are_deliberately_different() {
    // A dash between the os-release fields, parentheses around the
    // architecture — so a reader can tell which one they are looking at.
    let with = os_description(Some(("Bluefin", "42")), "Linux", "x86_64");
    let without = os_description(None, "Linux", "x86_64");
    assert!(with.contains(" - "));
    assert!(without.contains('('));
    assert!(!with.contains('('));
}

// --- the link's query ---------------------------------------------------

#[test]
fn a_report_carries_its_body_and_its_type() {
    let query = issue_query(None, "hello");
    assert_eq!(
        query,
        [
            ("body", "hello".to_owned()),
            ("type", ISSUE_TYPE.to_owned())
        ]
    );
}

#[test]
fn a_title_comes_first_when_there_is_one() {
    let query = issue_query(Some("Crash on launch"), "hello");
    assert_eq!(query[0], ("title", "Crash on launch".to_owned()));
    assert_eq!(query.len(), 3);
}

#[test]
fn an_empty_title_is_left_out_rather_than_sent_empty() {
    // An empty `title=` would leave GitHub's own placeholder unused and the
    // field looking filled in.
    assert_eq!(issue_query(Some(""), "hello").len(), 2);
}

#[test]
fn a_report_is_labelled_a_bug() {
    assert_eq!(ISSUE_TYPE, "bug");
}

// --- the fallback manager -----------------------------------------------

#[test]
fn a_command_that_cannot_be_a_fallback_appears_in_neither_list() {
    // The manager is not a list of everything with a switch next to it;
    // showing commands that cannot be turned on would be showing switches
    // that do nothing.
    let items = vec![("search".to_owned(), true), ("quit".to_owned(), false)];
    let split = fallback_sections(&items, &[]);
    assert_eq!(split.len(), 1);
    assert_eq!(split[0].0, "search");
}

#[test]
fn an_enabled_command_is_in_the_enabled_list() {
    let items = vec![("search".to_owned(), true)];
    let split = fallback_sections(&items, &ids(&["search"]));
    assert_eq!(split[0].1, FallbackSection::Enabled);
}

#[test]
fn everything_else_suitable_is_available() {
    let items = vec![("search".to_owned(), true)];
    let split = fallback_sections(&items, &ids(&["other"]));
    assert_eq!(split[0].1, FallbackSection::Available);
}

#[test]
fn the_enabled_list_follows_the_stored_order_and_not_relevance() {
    // A fallback's position decides which of them answers a query first, so
    // any other order would show a ranking that is not the one in force.
    let enabled = ids(&["c", "a", "b"]);
    let order = ids(&["a", "b", "c"]);
    assert_eq!(order_enabled(&enabled, &order), ["a", "b", "c"]);
}

#[test]
fn a_command_missing_from_the_order_sorts_to_the_end() {
    let enabled = ids(&["ghost", "a"]);
    let order = ids(&["a"]);
    assert_eq!(order_enabled(&enabled, &order), ["a", "ghost"]);
}

#[test]
fn several_missing_commands_keep_their_relative_order() {
    // The sort is stable, so they stay in the order they arrived rather than
    // being shuffled among themselves.
    let enabled = ids(&["y", "z", "a"]);
    let order = ids(&["a"]);
    assert_eq!(order_enabled(&enabled, &order), ["a", "y", "z"]);
}

#[test]
fn an_empty_order_leaves_everything_as_it_was() {
    let enabled = ids(&["c", "a", "b"]);
    assert_eq!(order_enabled(&enabled, &[]), ["c", "a", "b"]);
}

#[test]
fn each_list_offers_the_switch_that_is_not_already_on() {
    assert_eq!(
        fallback_action_label(FallbackSection::Enabled),
        "Disable fallback"
    );
    assert_eq!(
        fallback_action_label(FallbackSection::Available),
        "Enable fallback"
    );
}

// --- os-release and the whole link ----------------------------------------

#[test]
fn os_release_gives_the_pretty_name_and_version_unquoted() {
    use compass_core::bug_report::parse_os_release;
    let text = "NAME=\"Fedora Linux\"\nVERSION=\"42 (Silverblue)\"\nID=fedora\nPRETTY_NAME=\"Fedora Linux 42 (Silverblue)\"\n";
    assert_eq!(
        parse_os_release(text),
        Some((
            "Fedora Linux 42 (Silverblue)".to_owned(),
            "42 (Silverblue)".to_owned()
        ))
    );
    assert_eq!(
        parse_os_release("PRETTY_NAME='Arch Linux'\n"),
        Some(("Arch Linux".to_owned(), String::new())),
        "no VERSION on a rolling release"
    );
    assert_eq!(
        parse_os_release("ID=nixos\n"),
        None,
        "invalid without a name"
    );
}

#[test]
fn the_report_link_carries_the_title_body_and_type() {
    use compass_core::bug_report::report_url;
    let url = url::Url::parse(&report_url(Some("Crash on start"), &info())).unwrap();
    assert!(url.as_str().starts_with(CREATE_ISSUE_URL));
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs[0], ("title".to_owned(), "Crash on start".to_owned()));
    assert_eq!(pairs[1].0, "body");
    assert!(pairs[1].1.contains("- OS: Bluefin - 42"));
    assert_eq!(pairs[2], ("type".to_owned(), ISSUE_TYPE.to_owned()));
    let untitled = url::Url::parse(&report_url(None, &info())).unwrap();
    assert_eq!(untitled.query_pairs().next().unwrap().0, "body");
}
