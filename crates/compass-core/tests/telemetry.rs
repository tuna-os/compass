//! What telemetry sends, when, and what identifies it.
//!
//! Read off `TelemetryService` (`src/server/src/services/telemetry/`) and the
//! `Environment` helpers in `src/server/src/utils/environment.hpp`.

use std::cell::RefCell;

use compass_core::telemetry::{
    API_URL_ENV, DEFAULT_API_BASE_URL, FORGET_PATH, POLL_INTERVAL_SECS, Resolution, STATE_FILE,
    SYSTEM_INFO_INTERVAL_SECS, SYSTEM_INFO_PATH, ScreenInfo, State, SystemInfoRequest,
    TelemetryService, Transport, api_base_url, chassis_type, determine_product_id, load_state,
    save_state, should_send_system_info, state_path, to_lower, user_id_from_uuid,
};

/// A transport that records what it was given and answers on cue.
#[derive(Default)]
struct Recorder {
    accepts: bool,
    sent: RefCell<Vec<SystemInfoRequest>>,
    forgot: RefCell<Vec<String>>,
}

impl Recorder {
    /// A transport the server accepts everything from.
    fn accepting() -> Self {
        Self {
            accepts: true,
            ..Self::default()
        }
    }
}

impl Transport for Recorder {
    fn post_system_info(&self, request: &SystemInfoRequest) -> bool {
        self.sent.borrow_mut().push(request.clone());
        self.accepts
    }
    fn post_forget(&self, user_id: &str) -> bool {
        self.forgot.borrow_mut().push(user_id.to_owned());
        self.accepts
    }
}

/// A state with an id and nothing sent yet.
fn fresh_state() -> State {
    State {
        user_id: "user-abc".to_owned(),
        system_info_last_sent_at: None,
    }
}

#[test]
fn the_intervals_are_the_cpp_constants() {
    assert_eq!(SYSTEM_INFO_INTERVAL_SECS, 86_400, "one day");
    assert_eq!(POLL_INTERVAL_SECS, 3600, "one hour");
    // The timer must wake more often than the record is due, or a machine
    // that slept through its slot would skip a day.
    const { assert!(POLL_INTERVAL_SECS < SYSTEM_INFO_INTERVAL_SECS) };
}

#[test]
fn the_endpoints_and_the_default_host_are_unchanged() {
    assert_eq!(DEFAULT_API_BASE_URL, "https://api.vicinae.com/v1");
    assert_eq!(SYSTEM_INFO_PATH, "/telemetry/system-info");
    assert_eq!(FORGET_PATH, "/telemetry/forget");
    assert_eq!(API_URL_ENV, "COMPASS_VICINAE_API_URL");
    assert_eq!(STATE_FILE, "telemetry.json");
}

#[test]
fn the_api_url_can_be_overridden_by_the_environment() {
    assert_eq!(api_base_url(None), DEFAULT_API_BASE_URL);
    assert_eq!(
        api_base_url(Some("http://localhost:8080")),
        "http://localhost:8080"
    );
    assert_eq!(
        api_base_url(Some("")),
        DEFAULT_API_BASE_URL,
        "an empty override is not an override"
    );
}

#[test]
fn nothing_is_sent_until_telemetry_is_switched_on() {
    // The single most important property here. A freshly built service sends
    // nothing, however often it is asked.
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(!service.is_enabled());
    assert!(!service.try_send_system_info(1_000_000));
    assert!(service.transport().sent.borrow().is_empty());
}

#[test]
fn switching_it_on_sends_straight_away() {
    // Otherwise the first record would wait an hour for the timer.
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(service.set_enabled(true, 1_000_000));
    assert_eq!(service.transport().sent.borrow().len(), 1);
}

#[test]
fn switching_it_on_twice_sends_once() {
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    service.set_enabled(true, 1_000_000);
    assert!(
        !service.set_enabled(true, 1_000_001),
        "the second call changes nothing"
    );
    assert_eq!(service.transport().sent.borrow().len(), 1);
}

#[test]
fn switching_it_off_stops_it_sending() {
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    service.set_enabled(true, 1_000_000);
    service.set_enabled(false, 1_000_000);

    assert!(!service.try_send_system_info(1_000_000 + SYSTEM_INFO_INTERVAL_SECS));
    assert_eq!(service.transport().sent.borrow().len(), 1, "only the first");
}

#[test]
fn switching_it_off_first_is_not_treated_as_a_change_that_sends() {
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(service.set_enabled(false, 1_000_000));
    assert!(service.transport().sent.borrow().is_empty());
}

#[test]
fn each_service_keeps_its_own_enabled_flag() {
    // The C++ keeps it in a function-local static, so a second instance's
    // first setEnabled(true) is swallowed as "no change" and its telemetry
    // never starts. See PARITY.md — deliberately not reproduced.
    let mut first = TelemetryService::new(Recorder::accepting(), fresh_state());
    first.set_enabled(true, 1);

    let mut second = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(second.set_enabled(true, 1), "a fresh service starts unset");
    assert_eq!(second.transport().sent.borrow().len(), 1);
}

#[test]
fn a_record_is_due_when_none_has_ever_been_sent() {
    assert!(should_send_system_info(&fresh_state(), 0));
}

#[test]
fn a_record_is_not_due_again_within_the_day() {
    let state = State {
        user_id: "user-abc".to_owned(),
        system_info_last_sent_at: Some(1_000_000),
    };
    assert!(!should_send_system_info(&state, 1_000_000));
    assert!(!should_send_system_info(
        &state,
        1_000_000 + SYSTEM_INFO_INTERVAL_SECS - 1
    ));
    assert!(should_send_system_info(
        &state,
        1_000_000 + SYSTEM_INFO_INTERVAL_SECS
    ));
}

#[test]
fn a_clock_that_went_backwards_does_not_send_on_every_tick() {
    // saturating_sub: a machine whose clock jumped back would otherwise read
    // as "sent in the future", and an underflow would make it due immediately
    // and for ever.
    let state = State {
        user_id: "user-abc".to_owned(),
        system_info_last_sent_at: Some(2_000_000),
    };
    assert!(!should_send_system_info(&state, 1_000_000));
}

#[test]
fn the_timestamp_only_advances_on_a_record_the_server_took() {
    // A rejected record must not cost a day's data.
    let mut service = TelemetryService::new(Recorder::default(), fresh_state());
    service.set_enabled(true, 1_000_000);

    assert_eq!(service.state().system_info_last_sent_at, None);
    assert_eq!(
        service.transport().sent.borrow().len(),
        1,
        "it was attempted"
    );

    // Attempted again a second later rather than a day later. The return is
    // still false — the server is still refusing — so what is checked is that
    // a second record went out.
    assert!(!service.try_send_system_info(1_000_001));
    assert_eq!(service.transport().sent.borrow().len(), 2);
}

#[test]
fn an_accepted_record_records_when_it_went() {
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    service.set_enabled(true, 1_000_000);
    assert_eq!(service.state().system_info_last_sent_at, Some(1_000_000));
}

#[test]
fn every_record_carries_the_stored_user_id() {
    let mut service = TelemetryService::new(Recorder::accepting(), fresh_state());
    service.set_enabled(true, 1_000_000);
    assert_eq!(service.transport().sent.borrow()[0].user_id, "user-abc");
}

#[test]
fn forgetting_names_the_id_to_unlink() {
    let service = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(service.forget());
    assert_eq!(service.transport().forgot.borrow().as_slice(), ["user-abc"]);
}

#[test]
fn forgetting_works_whether_or_not_telemetry_is_on() {
    // Someone who just turned it off is exactly who wants this.
    let service = TelemetryService::new(Recorder::accepting(), fresh_state());
    assert!(!service.is_enabled());
    assert!(service.forget());
}

#[test]
fn a_user_id_is_prefixed_so_it_is_recognisable() {
    assert_eq!(user_id_from_uuid("0123-4567"), "user-0123-4567");
}

#[test]
fn the_record_uses_the_cpp_member_names_on_the_wire() {
    // glaze writes members as they are named, and no snake_case adapter is
    // registered for this struct. A renamed key is a record the server files
    // separately from the C++'s.
    let request = SystemInfoRequest {
        user_id: "user-abc".to_owned(),
        vicinae_version: "v1.2.3".to_owned(),
        qt_version: "6.8.0".to_owned(),
        display_protocol: "wayland".to_owned(),
        build_provenance: "flatpak".to_owned(),
        operating_system: "linux".to_owned(),
        chassis_type: "laptop".to_owned(),
        kernel_version: "6.11".to_owned(),
        product_id: "fedora".to_owned(),
        product_version: "41".to_owned(),
        screens: vec![ScreenInfo {
            resolution: Resolution {
                width: 2560,
                height: 1440,
            },
            scale: 2.0,
        }],
        ..SystemInfoRequest::default()
    };
    let json: serde_json::Value = serde_json::to_value(&request).expect("serialises");

    for key in [
        "userId",
        "vicinaeVersion",
        "qtVersion",
        "displayProtocol",
        "buildProvenance",
        "operatingSystem",
        "chassisType",
        "kernelVersion",
        "productId",
        "productVersion",
        "desktops",
        "screens",
        "locale",
        "architecture",
    ] {
        assert!(json.get(key).is_some(), "{key} is missing from {json}");
    }
    assert_eq!(json["screens"][0]["resolution"]["width"], 2560);
    assert_eq!(json["screens"][0]["scale"], 2.0);
}

#[test]
fn lowercasing_is_what_the_cpp_does_to_the_fields_it_does_it_to() {
    assert_eq!(to_lower("X86_64"), "x86_64");
    assert_eq!(to_lower("Flatpak"), "flatpak");
}

#[test]
fn omarchy_is_detected_by_its_directory_rather_than_os_release() {
    // It does not override /etc/os-release, so it would be counted as Arch.
    let data_home = tempfile::tempdir().expect("tempdir");
    assert_eq!(determine_product_id(data_home.path(), "arch"), "arch");

    std::fs::create_dir(data_home.path().join("omarchy")).expect("mkdir");
    assert_eq!(determine_product_id(data_home.path(), "arch"), "omarchy");
}

#[test]
fn a_file_called_omarchy_is_not_a_distribution() {
    let data_home = tempfile::tempdir().expect("tempdir");
    std::fs::write(data_home.path().join("omarchy"), "").expect("write");
    assert_eq!(
        determine_product_id(data_home.path(), "arch"),
        "arch",
        "is_directory, not exists"
    );
}

#[test]
fn chassis_codes_map_the_way_the_dmi_table_says() {
    for code in [3, 4, 5, 6, 7] {
        assert_eq!(chassis_type(Some(code)), "desktop", "code {code}");
    }
    for code in [8, 9, 10, 11, 12, 13, 14, 30, 31, 32] {
        assert_eq!(chassis_type(Some(code)), "laptop", "code {code}");
    }
    assert_eq!(chassis_type(Some(1)), "other");
    assert_eq!(chassis_type(Some(23)), "other", "a rack-mount server");
}

#[test]
fn an_unreadable_chassis_file_is_unknown_and_not_other() {
    // "neither desktop nor laptop" and "we could not tell" are different
    // answers, and collapsing them would turn a gap into a data point.
    assert_eq!(chassis_type(None), "unknown");
    assert_ne!(chassis_type(None), chassis_type(Some(1)));
}

#[test]
fn a_missing_state_file_is_created_with_a_fresh_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());

    let state = load_state(&path, || "uuid-1".to_owned()).expect("loads");
    assert_eq!(state.user_id, "user-uuid-1");
    assert_eq!(state.system_info_last_sent_at, None);
    assert!(path.is_file(), "it is written back so the id is stable");
}

#[test]
fn an_existing_state_file_keeps_its_id_and_its_timestamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    save_state(
        &path,
        &State {
            user_id: "user-kept".to_owned(),
            system_info_last_sent_at: Some(1_234),
        },
    )
    .expect("saves");

    let state = load_state(&path, || panic!("must not generate a new id")).expect("loads");
    assert_eq!(state.user_id, "user-kept");
    assert_eq!(state.system_info_last_sent_at, Some(1_234));
}

#[test]
fn a_corrupt_state_file_gets_a_fresh_id_rather_than_an_empty_one() {
    // The C++ warns and carries on with whatever glaze left behind, which for
    // an unreadable file is an empty userId — so every record after that is
    // filed under "". See PARITY.md.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    std::fs::write(&path, "{ not json").expect("write");

    let state = load_state(&path, || "uuid-2".to_owned()).expect("loads");
    assert_eq!(state.user_id, "user-uuid-2");

    let reread = load_state(&path, || panic!("the fresh id must have been saved")).expect("loads");
    assert_eq!(reread.user_id, "user-uuid-2");
}

#[test]
fn a_state_file_with_an_empty_id_is_treated_as_corrupt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    std::fs::write(&path, r#"{"userId":"","systemInfoLastSentAt":5}"#).expect("write");

    let state = load_state(&path, || "uuid-3".to_owned()).expect("loads");
    assert_eq!(state.user_id, "user-uuid-3");
}

#[test]
fn the_state_file_is_written_under_a_directory_that_may_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(&dir.path().join("nested").join("deeper"));

    load_state(&path, || "uuid-4".to_owned()).expect("loads");
    assert!(path.is_file());
}

#[test]
fn the_state_file_round_trips_through_the_cpp_key_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    save_state(
        &path,
        &State {
            user_id: "user-x".to_owned(),
            system_info_last_sent_at: Some(9),
        },
    )
    .expect("saves");

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("JSON");
    assert_eq!(json["userId"], "user-x");
    assert_eq!(json["systemInfoLastSentAt"], 9);
}
