//! Which notices show, and how they stop showing.
//!
//! Read off `NewsService` (`src/server/src/services/news/`).

use compass_core::news::{
    ALL_ITEMS, NewsService, Relevance, STATE_FILE, State, TELEMETRY_NOTICE_ID, is_relevant,
    load_state, save_state, state_path,
};

/// Telemetry on, so the notice is relevant.
fn telemetry_on() -> Relevance {
    Relevance {
        telemetry_system_info: true,
    }
}

/// Telemetry off.
fn telemetry_off() -> Relevance {
    Relevance {
        telemetry_system_info: false,
    }
}

#[test]
fn the_telemetry_notice_says_what_the_cpp_says() {
    // Invented wording here would be wording nobody wrote, on the one screen
    // that tells a person data is being collected.
    let item = ALL_ITEMS
        .iter()
        .find(|item| item.id == TELEMETRY_NOTICE_ID)
        .expect("the telemetry notice exists");
    assert_eq!(item.title, "Telemetry");
    assert_eq!(
        item.subtitle,
        "We now collect basic usage statistics on startup"
    );
    assert_eq!(item.icon, "megaphone");
    assert_eq!(item.icon_background_tint, "Yellow");
    assert_eq!(item.learn_more_url, "https://docs.vicinae.com/telemetry");
}

#[test]
fn the_notice_id_is_versioned_so_the_subject_can_be_raised_again() {
    // A dismissal records the id, so saying the same thing later needs a new
    // one — telemetry-notice-v2 is not hidden by a v1 dismissal.
    assert_eq!(TELEMETRY_NOTICE_ID, "telemetry-notice-v1");
    assert!(TELEMETRY_NOTICE_ID.ends_with("-v1"));
}

#[test]
fn every_notice_has_an_id_title_and_subtitle() {
    for item in ALL_ITEMS {
        assert!(!item.id.is_empty());
        assert!(!item.title.is_empty(), "{} has no title", item.id);
        assert!(!item.subtitle.is_empty(), "{} has no subtitle", item.id);
    }
}

#[test]
fn notice_ids_are_unique() {
    // Two notices with one id would be dismissed together, silently.
    let mut ids: Vec<_> = ALL_ITEMS.iter().map(|item| item.id).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn a_fresh_service_shows_the_telemetry_notice_when_telemetry_is_on() {
    let service = NewsService::default();
    let active = service.active_items(telemetry_on());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, TELEMETRY_NOTICE_ID);
    assert!(service.has_unread_news(telemetry_on()));
}

#[test]
fn the_telemetry_notice_hides_itself_when_telemetry_is_off() {
    // This is why activeItems is not simply "everything not dismissed": the
    // notice tells people what is happening, and nothing is happening.
    let service = NewsService::default();
    assert!(service.active_items(telemetry_off()).is_empty());
    assert!(!service.has_unread_news(telemetry_off()));
}

#[test]
fn a_dismissed_notice_does_not_come_back() {
    let mut service = NewsService::default();
    assert!(service.dismiss(TELEMETRY_NOTICE_ID));
    assert!(service.is_dismissed(TELEMETRY_NOTICE_ID));
    assert!(service.active_items(telemetry_on()).is_empty());
}

#[test]
fn dismissing_twice_records_it_once_and_reports_no_change() {
    // The second dismissal must not ask for another save or another redraw.
    let mut service = NewsService::default();
    assert!(service.dismiss(TELEMETRY_NOTICE_ID));
    assert!(!service.dismiss(TELEMETRY_NOTICE_ID));
    assert_eq!(service.state().dismissed, vec![TELEMETRY_NOTICE_ID]);
}

#[test]
fn dismissing_an_unknown_id_is_recorded_anyway() {
    // Which is what keeps a notice pulled from a build dismissed if it ever
    // comes back.
    let mut service = NewsService::default();
    assert!(service.dismiss("some-future-notice"));
    assert!(service.is_dismissed("some-future-notice"));
}

#[test]
fn a_dismissal_survives_a_reload() {
    let mut service = NewsService::default();
    service.dismiss(TELEMETRY_NOTICE_ID);

    let reloaded = NewsService::new(service.state());
    assert!(reloaded.is_dismissed(TELEMETRY_NOTICE_ID));
    assert!(reloaded.active_items(telemetry_on()).is_empty());
}

#[test]
fn a_notice_hidden_by_configuration_is_not_marked_dismissed() {
    // Turning telemetry off and on again shows the notice again, because it
    // was never dismissed. That is the difference between the two rules.
    let service = NewsService::default();
    assert!(service.active_items(telemetry_off()).is_empty());
    assert!(!service.is_dismissed(TELEMETRY_NOTICE_ID));
    assert_eq!(service.active_items(telemetry_on()).len(), 1);
}

#[test]
fn relevance_only_gates_the_notice_it_is_about() {
    // A second notice must not be hidden by the telemetry setting.
    let other = compass_core::news::NewsItem {
        id: "something-else-v1",
        title: "Other",
        subtitle: "Other",
        icon: "star",
        icon_background_tint: "Blue",
        learn_more_url: "https://example.test",
    };
    assert!(is_relevant(&other, telemetry_off()));
    assert!(is_relevant(&other, telemetry_on()));
}

#[test]
fn the_state_file_is_named_and_shaped_the_way_the_cpp_writes_it() {
    assert_eq!(STATE_FILE, "news.json");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    save_state(
        &path,
        &State {
            dismissed: vec![TELEMETRY_NOTICE_ID.to_owned()],
        },
    )
    .expect("saves");

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("JSON");
    assert_eq!(json["dismissed"][0], TELEMETRY_NOTICE_ID);
}

#[test]
fn a_missing_state_file_dismisses_nothing() {
    // Showing a notice again is a nuisance; never showing it is a thing the
    // person never learns. The nuisance is the right failure.
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(load_state(&state_path(dir.path())), State::default());
}

#[test]
fn an_unreadable_state_file_dismisses_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    std::fs::write(&path, "{ not json").expect("write");
    assert_eq!(load_state(&path), State::default());
}

#[test]
fn a_state_file_with_no_dismissed_key_still_loads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    std::fs::write(&path, "{}").expect("write");
    assert_eq!(load_state(&path), State::default());
}

#[test]
fn the_dismissals_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    let state = State {
        dismissed: vec!["a".to_owned(), "b".to_owned()],
    };
    save_state(&path, &state).expect("saves");
    assert_eq!(load_state(&path), state);
}

#[test]
fn saving_into_a_directory_that_does_not_exist_fails_rather_than_creating_it() {
    // saveState opens an ofstream and warns; unlike the update and telemetry
    // services it does not create_directories first. Reproduced so a missing
    // state directory is noticed rather than papered over.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(&dir.path().join("not-there"));
    assert!(save_state(&path, &State::default()).is_err());
}
