//! What is in the tray menu and what each entry does.
//!
//! Read off `TrayService` and `TrayServiceLinux`
//! (`src/server/src/services/tray/`).

use compass_core::tray::{
    ABOUT_LABEL, APP_NAME, Activation, CHECK_FOR_UPDATES_LABEL, DISCORD_LABEL, DISCORD_URL,
    EntryKind, FOLLOW_LABEL, FOLLOW_URL, Link, PREFERENCES_LABEL, QUIT_LABEL, SETTINGS_LABEL,
    SPONSOR_LABEL, SPONSOR_URL, SYSTEMD_INVOCATION_ENV, TOGGLE_LABEL, activate, entry_enabled,
    entry_label, menu_entries, update_available_label,
};

/// The kind of each entry, in order.
fn kinds(under_systemd: bool) -> Vec<EntryKind> {
    menu_entries(under_systemd)
        .into_iter()
        .map(|entry| entry.kind)
        .collect()
}

#[test]
fn the_labels_are_the_cpp_ones_under_the_compass_name() {
    // Invented wording here is wording nobody wrote, in the one menu a person
    // reaches when the launcher itself will not open. The product's name is
    // Compass (ADR-0012); the sponsor link is still upstream Vicinae's.
    assert_eq!(TOGGLE_LABEL, "Toggle Compass");
    assert_eq!(ABOUT_LABEL, "About Compass");
    assert_eq!(CHECK_FOR_UPDATES_LABEL, "Check for Updates…");
    assert_eq!(SETTINGS_LABEL, "Settings…");
    assert_eq!(PREFERENCES_LABEL, "Preferences…");
    assert_eq!(SPONSOR_LABEL, "Sponsor Vicinae");
    assert_eq!(DISCORD_LABEL, "Join the Discord");
    assert_eq!(FOLLOW_LABEL, "Follow on X");
    assert_eq!(QUIT_LABEL, "Quit Compass");
}

#[test]
fn settings_and_preferences_are_different_words_for_one_thing() {
    // macOS calls it Preferences. Both strings exist so a port to it does not
    // have to invent one.
    assert_ne!(SETTINGS_LABEL, PREFERENCES_LABEL);
    assert!(SETTINGS_LABEL.ends_with('…') && PREFERENCES_LABEL.ends_with('…'));
}

#[test]
fn the_update_label_names_the_tag() {
    assert_eq!(update_available_label("v1.2.3"), "Update Available: v1.2.3");
}

#[test]
fn the_menu_has_the_cpp_entries_in_order() {
    assert_eq!(
        kinds(false),
        vec![
            EntryKind::Toggle,
            EntryKind::Version,
            EntryKind::Separator,
            EntryKind::About,
            EntryKind::Settings,
            EntryKind::Separator,
            EntryKind::Sponsor,
            EntryKind::Discord,
            EntryKind::Follow,
            EntryKind::Separator,
            EntryKind::Quit,
        ]
    );
}

#[test]
fn quit_is_absent_under_systemd() {
    // The process was started by a unit: quitting would either be undone by
    // the restart policy or leave the unit stopped with no obvious way back.
    // Neither is what somebody clicking Quit wants.
    assert_eq!(SYSTEMD_INVOCATION_ENV, "INVOCATION_ID");
    let supervised = kinds(true);
    assert!(!supervised.contains(&EntryKind::Quit));
    assert_eq!(supervised.last(), Some(&EntryKind::Follow));
}

#[test]
fn the_separator_before_quit_goes_with_it() {
    // Otherwise the supervised menu ends with a dividing line and nothing
    // under it.
    let supervised = menu_entries(true);
    let unsupervised = menu_entries(false);
    assert_eq!(unsupervised.len(), supervised.len() + 2);
    assert_ne!(
        supervised.last().map(|entry| entry.kind),
        Some(EntryKind::Separator)
    );
}

#[test]
fn toggle_comes_first_whatever_else_is_shown() {
    // It is the entry somebody opens the tray for.
    for under_systemd in [true, false] {
        assert_eq!(kinds(under_systemd).first(), Some(&EntryKind::Toggle));
    }
}

#[test]
fn ids_start_at_one_and_are_unique() {
    // Zero is the menu's own root in the StatusNotifierItem protocol, so an
    // entry with that id would be the menu itself.
    let entries = menu_entries(false);
    assert_eq!(entries[0].id, 1);

    let mut ids: Vec<i32> = entries.iter().map(|entry| entry.id).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert!(ids.iter().all(|id| *id > 0));
}

#[test]
fn separators_get_ids_too() {
    // They are menu items on the bus like any other, so numbering has to
    // count them or every later id is off by one.
    let entries = menu_entries(false);
    let separators: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Separator)
        .collect();
    assert_eq!(separators.len(), 3);
    assert!(separators.iter().all(|entry| entry.id > 0));
}

#[test]
fn the_version_entry_reads_as_the_app_name_until_a_version_arrives() {
    // setVersion is called after construction, and an empty line in the menu
    // in between would look like a bug.
    assert_eq!(entry_label(EntryKind::Version, ""), APP_NAME);
    assert_eq!(entry_label(EntryKind::Version, "1.2.3"), "Compass 1.2.3");
}

#[test]
fn a_separator_has_no_label() {
    assert_eq!(entry_label(EntryKind::Separator, "1.2.3"), "");
}

#[test]
fn every_clickable_entry_has_a_label() {
    for kind in [
        EntryKind::Toggle,
        EntryKind::About,
        EntryKind::Settings,
        EntryKind::Sponsor,
        EntryKind::Discord,
        EntryKind::Follow,
        EntryKind::Quit,
    ] {
        assert!(!entry_label(kind, "").is_empty(), "{kind:?}");
        assert!(entry_enabled(kind), "{kind:?}");
    }
}

#[test]
fn the_version_is_shown_and_not_clickable() {
    assert!(!entry_enabled(EntryKind::Version));
    assert!(!entry_label(EntryKind::Version, "").is_empty());
}

#[test]
fn toggling_is_what_the_first_entry_does() {
    let entries = menu_entries(false);
    assert_eq!(activate(&entries, entries[0].id), Some(Activation::Toggle));
}

#[test]
fn about_opens_settings_on_the_about_tab_and_settings_opens_no_tab() {
    let entries = menu_entries(false);
    let id_of = |kind: EntryKind| {
        entries
            .iter()
            .find(|entry| entry.kind == kind)
            .expect("present")
            .id
    };

    assert_eq!(
        activate(&entries, id_of(EntryKind::About)),
        Some(Activation::OpenSettings {
            tab: Some("about".to_owned())
        })
    );
    assert_eq!(
        activate(&entries, id_of(EntryKind::Settings)),
        Some(Activation::OpenSettings { tab: None })
    );
}

#[test]
fn the_three_links_go_where_the_cpp_sends_them() {
    assert_eq!(Link::Sponsor.url(), SPONSOR_URL);
    assert_eq!(Link::Discord.url(), DISCORD_URL);
    assert_eq!(Link::Follow.url(), FOLLOW_URL);
    assert!(SPONSOR_URL.starts_with("https://github.com/sponsors/"));
    assert!(DISCORD_URL.starts_with("https://discord.gg/"));
    assert!(FOLLOW_URL.starts_with("https://x.com/"));
}

#[test]
fn the_three_links_are_three_different_pages() {
    let mut urls = vec![Link::Sponsor.url(), Link::Discord.url(), Link::Follow.url()];
    urls.sort_unstable();
    urls.dedup();
    assert_eq!(urls.len(), 3);
}

#[test]
fn each_link_entry_asks_for_its_own_link() {
    let entries = menu_entries(false);
    for (kind, link) in [
        (EntryKind::Sponsor, Link::Sponsor),
        (EntryKind::Discord, Link::Discord),
        (EntryKind::Follow, Link::Follow),
    ] {
        let id = entries
            .iter()
            .find(|entry| entry.kind == kind)
            .expect("present")
            .id;
        assert_eq!(activate(&entries, id), Some(Activation::OpenLink(link)));
    }
}

#[test]
fn quitting_asks_to_quit() {
    let entries = menu_entries(false);
    let id = entries
        .iter()
        .find(|entry| entry.kind == EntryKind::Quit)
        .expect("present")
        .id;
    assert_eq!(activate(&entries, id), Some(Activation::Quit));
}

#[test]
fn the_version_and_the_separators_do_nothing_when_clicked() {
    let entries = menu_entries(false);
    for kind in [EntryKind::Version, EntryKind::Separator] {
        let id = entries
            .iter()
            .find(|entry| entry.kind == kind)
            .expect("present")
            .id;
        assert_eq!(activate(&entries, id), None, "{kind:?}");
    }
}

#[test]
fn an_unknown_id_does_nothing_rather_than_failing() {
    // The menu is rebuilt as the version and update state change, so a click
    // can arrive against the layout before it.
    let entries = menu_entries(false);
    assert_eq!(activate(&entries, 0), None);
    assert_eq!(activate(&entries, 9999), None);
    assert_eq!(activate(&[], 1), None);
}

#[test]
fn a_supervised_menu_cannot_be_made_to_quit_by_id() {
    // The id Quit would have had belongs to nothing, so a stale click cannot
    // reach it.
    let supervised = menu_entries(true);
    for id in 1..=20 {
        assert_ne!(
            activate(&supervised, id),
            Some(Activation::Quit),
            "id {id} should not quit"
        );
    }
}
