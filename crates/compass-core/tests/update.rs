//! Which releases become offers, and which do not.
//!
//! Read off `UpdateService` (`src/server/src/services/update/`).

use compass_core::update::{
    CACHE_FILE, CHECK_INTERVAL_SECS, CheckCache, DEFAULT_FEED_URL, FAILED_TITLE, FEED_URL_ENV,
    INSTALLED_BODY, INSTALLED_TITLE, NullUpdateInstaller, RELAUNCH_DELAY_MS, Rejected, Release,
    ReleaseAsset, ReleaseCheck, STATE_FILE, State, Status, UNSUPPORTED_MESSAGE, UpdateInstaller,
    UpdateService, VERSION_OVERRIDE_ENV, cache_path, display_version, feed_url, load_cache,
    load_state, save_cache, save_state, state_path,
};

/// An installer that can install one named asset.
struct Installer {
    supported: bool,
    asset: &'static str,
}

impl Default for Installer {
    fn default() -> Self {
        Self {
            supported: true,
            asset: "vicinae-linux-x86_64.tar.gz",
        }
    }
}

impl UpdateInstaller for Installer {
    fn supported(&self) -> bool {
        self.supported
    }
    fn asset_name(&self) -> String {
        self.asset.to_owned()
    }
}

/// A published release of `tag` carrying the installable asset.
fn release(tag: &str) -> Release {
    Release {
        tag_name: tag.to_owned(),
        html_url: format!("https://github.com/o/r/releases/{tag}"),
        draft: false,
        prerelease: false,
        assets: vec![
            ReleaseAsset {
                name: "vicinae-macos.dmg".to_owned(),
                browser_download_url: "https://example.test/mac".to_owned(),
            },
            ReleaseAsset {
                name: "vicinae-linux-x86_64.tar.gz".to_owned(),
                browser_download_url: "https://example.test/linux".to_owned(),
            },
        ],
    }
}

/// A service on v1.0.0 with nothing skipped.
fn service() -> UpdateService<Installer> {
    UpdateService::new(Installer::default(), "v1.0.0", State::default())
}

#[test]
fn the_constants_are_the_cpp_ones() {
    assert_eq!(CHECK_INTERVAL_SECS, 6 * 3600, "six hours");
    assert_eq!(RELAUNCH_DELAY_MS, 800);
    assert_eq!(STATE_FILE, "updates.json");
    assert_eq!(FEED_URL_ENV, "VICINAE_UPDATE_FEED_URL");
    assert_eq!(VERSION_OVERRIDE_ENV, "VICINAE_UPDATE_VERSION");
    assert_eq!(INSTALLED_TITLE, "Update installed");
    assert_eq!(INSTALLED_BODY, "Restarting…");
    assert_eq!(FAILED_TITLE, "Update failed");
    assert_eq!(
        UNSUPPORTED_MESSAGE,
        "Self update is not supported on this platform"
    );
}

#[test]
fn the_feed_url_can_be_overridden() {
    assert_eq!(feed_url(None), DEFAULT_FEED_URL);
    assert!(DEFAULT_FEED_URL.contains("releases/latest"));
    assert_eq!(
        feed_url(Some("http://localhost/feed")),
        "http://localhost/feed"
    );
    assert_eq!(feed_url(Some("")), DEFAULT_FEED_URL);
}

#[test]
fn a_newer_release_becomes_an_offer() {
    let mut service = service();
    let update = service.handle_release(&release("v1.1.0")).expect("offered");
    assert_eq!(update.tag, "v1.1.0");
    assert_eq!(update.version, "1.1.0", "the v is stripped for display");
    assert_eq!(
        update.asset_url.as_deref(),
        Some("https://example.test/linux")
    );
    assert_eq!(update.release_url, "https://github.com/o/r/releases/v1.1.0");
    assert_eq!(service.status(), Status::UpdateAvailable);
}

#[test]
fn the_same_version_is_not_an_update() {
    let mut service = service();
    assert_eq!(
        service.handle_release(&release("v1.0.0")),
        Err(Rejected::NotNewer)
    );
    assert_eq!(service.status(), Status::Idle);
    assert_eq!(service.available(), None);
}

#[test]
fn an_older_release_is_not_an_update() {
    let mut service = service();
    assert_eq!(
        service.handle_release(&release("v0.9.0")),
        Err(Rejected::NotNewer)
    );
}

#[test]
fn a_release_with_a_trailing_zero_is_the_same_version() {
    // v1.0 and v1.0.0 are the same release, so it must not be offered.
    let mut service = service();
    assert_eq!(
        service.handle_release(&release("v1.0")),
        Err(Rejected::NotNewer)
    );
}

#[test]
fn a_draft_is_never_offered() {
    let mut service = service();
    let mut draft = release("v1.1.0");
    draft.draft = true;
    assert_eq!(service.handle_release(&draft), Err(Rejected::Draft));
}

#[test]
fn a_prerelease_is_never_offered() {
    let mut service = service();
    let mut pre = release("v1.1.0");
    pre.prerelease = true;
    assert_eq!(service.handle_release(&pre), Err(Rejected::Prerelease));
}

#[test]
fn a_tag_that_is_not_a_version_is_not_offered() {
    let mut service = service();
    assert_eq!(
        service.handle_release(&release("nightly")),
        Err(Rejected::NotAVersion)
    );
}

#[test]
fn a_release_with_no_asset_for_this_platform_is_not_offered() {
    // Not a failure: a build that nagged about an update it cannot install
    // would be worse than one that says nothing.
    let mut service = service();
    let mut no_asset = release("v1.1.0");
    no_asset
        .assets
        .retain(|asset| asset.name == "vicinae-macos.dmg");
    assert_eq!(service.handle_release(&no_asset), Err(Rejected::NoAsset));
    assert_eq!(service.status(), Status::Idle);
}

#[test]
fn the_asset_is_matched_by_its_exact_name() {
    // A release carries assets for every platform; a prefix or substring match
    // would download the wrong one.
    let mut service = service();
    let mut near_miss = release("v1.1.0");
    near_miss.assets = vec![ReleaseAsset {
        name: "vicinae-linux-x86_64.tar.gz.sha256".to_owned(),
        browser_download_url: "https://example.test/checksum".to_owned(),
    }];
    assert_eq!(service.handle_release(&near_miss), Err(Rejected::NoAsset));
}

#[test]
fn a_skipped_version_is_not_offered_again() {
    let mut service = UpdateService::new(
        Installer::default(),
        "v1.0.0",
        State {
            skipped_version: Some("v1.1.0".to_owned()),
        },
    );
    assert_eq!(
        service.handle_release(&release("v1.1.0")),
        Err(Rejected::Skipped)
    );
}

#[test]
fn skipping_one_version_does_not_skip_the_next() {
    let mut service = UpdateService::new(
        Installer::default(),
        "v1.0.0",
        State {
            skipped_version: Some("v1.1.0".to_owned()),
        },
    );
    assert!(service.handle_release(&release("v1.2.0")).is_ok());
}

#[test]
fn skipping_records_the_tag_and_drops_the_offer() {
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");

    assert_eq!(service.skip_available_version().as_deref(), Some("v1.1.0"));
    assert_eq!(service.available(), None);
    assert_eq!(service.status(), Status::Idle);
    assert_eq!(service.state().skipped_version.as_deref(), Some("v1.1.0"));
}

#[test]
fn skipping_with_nothing_offered_does_nothing() {
    let mut service = service();
    assert_eq!(service.skip_available_version(), None);
    assert_eq!(service.state().skipped_version, None);
}

#[test]
fn a_development_build_does_not_check_at_all() {
    // Nothing to compare against, so every release would look newer or older
    // by accident. Disabling is the only safe answer.
    let service = UpdateService::new(Installer::default(), "dev-build", State::default());
    assert!(!service.checks_supported());
    assert_eq!(service.current_version(), None);
}

#[test]
fn a_platform_that_cannot_self_update_does_not_check() {
    let service = UpdateService::new(
        Installer {
            supported: false,
            ..Installer::default()
        },
        "v1.0.0",
        State::default(),
    );
    assert!(!service.checks_supported());
}

#[test]
fn the_null_installer_supports_nothing_and_names_no_asset() {
    let installer = NullUpdateInstaller;
    assert!(!installer.supported());
    assert_eq!(installer.asset_name(), "");
}

#[test]
fn a_check_cannot_start_while_one_is_unsupported() {
    let mut service = UpdateService::new(Installer::default(), "dev", State::default());
    assert!(!service.begin_check());
    assert_eq!(service.status(), Status::Idle);
}

#[test]
fn a_check_cannot_start_while_downloading_installing_or_installed() {
    // Starting a second download over a half-written archive is how an update
    // bricks an install.
    for busy in [Status::Downloading, Status::Installing, Status::Installed] {
        let mut service = service();
        service.handle_release(&release("v1.1.0")).expect("offered");
        service.begin_download();
        match busy {
            Status::Installing => service.begin_install(),
            Status::Installed => {
                service.begin_install();
                service.finish_install();
            }
            _ => {}
        }
        assert_eq!(service.status(), busy);
        assert!(!service.begin_check(), "a check started during {busy:?}");
    }
}

#[test]
fn a_failed_check_goes_back_to_idle() {
    let mut service = service();
    service.begin_check();
    assert_eq!(service.status(), Status::Checking);
    service.check_failed();
    assert_eq!(service.status(), Status::Idle);
}

#[test]
fn a_late_check_failure_does_not_reset_a_download() {
    // The C++ guards with `if (m_status == Status::Checking)`. Without it, a
    // failure arriving from the previous check would knock a running download
    // back to idle and let a second one start over the same file.
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");
    service.begin_download();

    service.check_failed();
    assert_eq!(service.status(), Status::Downloading);
}

#[test]
fn a_download_cannot_start_without_an_offer() {
    let mut service = service();
    assert_eq!(service.begin_download(), None);
    assert_eq!(service.status(), Status::Idle);
}

#[test]
fn a_download_cannot_start_twice() {
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");
    assert!(service.begin_download().is_some());
    assert_eq!(service.begin_download(), None, "already downloading");
}

#[test]
fn the_install_cycle_runs_through_its_states() {
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");
    assert_eq!(service.status(), Status::UpdateAvailable);
    service.begin_download();
    assert_eq!(service.status(), Status::Downloading);
    service.begin_install();
    assert_eq!(service.status(), Status::Installing);
    service.finish_install();
    assert_eq!(service.status(), Status::Installed);
}

#[test]
fn a_failure_anywhere_ends_in_failed() {
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");
    service.begin_download();
    service.fail();
    assert_eq!(service.status(), Status::Failed);
    assert!(
        !service.status().is_busy(),
        "a failed update must not block the next attempt"
    );
}

#[test]
fn a_new_offer_replaces_the_previous_one() {
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");
    let update = service.handle_release(&release("v1.2.0")).expect("offered");
    assert_eq!(update.tag, "v1.2.0");
}

#[test]
fn a_release_that_stops_qualifying_withdraws_the_offer() {
    // The feed can change under us — a release being marked prerelease after
    // publication, say. The offer has to go away, not linger.
    let mut service = service();
    service.handle_release(&release("v1.1.0")).expect("offered");

    let mut pre = release("v1.1.0");
    pre.prerelease = true;
    assert!(service.handle_release(&pre).is_err());
    assert_eq!(service.available(), None);
}

#[test]
fn the_displayed_version_drops_only_a_leading_v() {
    assert_eq!(display_version("v1.2.3"), "1.2.3");
    assert_eq!(display_version("1.2.3"), "1.2.3");
    assert_eq!(display_version("v1.2.3v"), "1.2.3v");
}

#[test]
fn the_download_toast_names_the_tag() {
    assert_eq!(
        UpdateService::<Installer>::downloading_toast("v1.1.0"),
        "Downloading Vicinae v1.1.0…"
    );
}

#[test]
fn progress_is_only_shown_when_the_total_is_known() {
    // A percentage of an unknown total is a number that means nothing, and the
    // C++ guards with `if (total > 0)`.
    assert_eq!(
        UpdateService::<Installer>::downloading_progress_toast("v1.1.0", 50, 200).as_deref(),
        Some("Downloading Vicinae v1.1.0… 25%")
    );
    assert_eq!(
        UpdateService::<Installer>::downloading_progress_toast("v1.1.0", 50, 0),
        None
    );
    assert_eq!(
        UpdateService::<Installer>::downloading_progress_toast("v1.1.0", 50, -1),
        None
    );
}

#[test]
fn a_missing_state_file_is_an_empty_state() {
    // Forgetting a skip offers an update; inventing one hides an update. The
    // first is the safer failure, and is what the C++ does.
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(load_state(&state_path(dir.path())), State::default());
}

#[test]
fn an_unreadable_state_file_is_an_empty_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());
    std::fs::write(&path, "{ not json").expect("write");
    assert_eq!(load_state(&path), State::default());
}

#[test]
fn the_skipped_version_survives_a_round_trip_under_the_cpp_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(&dir.path().join("nested"));
    save_state(
        &path,
        &State {
            skipped_version: Some("v9.9.9".to_owned()),
        },
    )
    .expect("saves");

    assert_eq!(load_state(&path).skipped_version.as_deref(), Some("v9.9.9"));
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("JSON");
    assert_eq!(json["skippedVersion"], "v9.9.9");
}

#[test]
fn compass_reads_its_own_releases() {
    assert_eq!(
        DEFAULT_FEED_URL,
        "https://api.github.com/repos/tuna-os/compass/releases/latest"
    );
    assert_eq!(CACHE_FILE, "latest-release.json");
}

#[test]
fn a_release_check_offers_any_newer_published_release_without_an_asset() {
    let mut service = UpdateService::new(ReleaseCheck, "v1.0.0", State::default());
    assert!(service.checks_supported(), "checking needs no installer");
    let mut bare = release("v1.1.0");
    bare.assets.clear();
    let update = service.handle_release(&bare).expect("offered");
    assert_eq!(update.asset_url, None);
    assert_eq!(update.release_url, "https://github.com/o/r/releases/v1.1.0");
    assert!(!service.may_download(), "there is nothing to download");
}

#[test]
fn a_release_check_keeps_every_other_gate() {
    for (mutate, rejected) in [
        (
            (|r: &mut Release| r.tag_name = "v1.0.0".to_owned()) as fn(&mut Release),
            Rejected::NotNewer,
        ),
        (|r: &mut Release| r.draft = true, Rejected::Draft),
        (|r: &mut Release| r.prerelease = true, Rejected::Prerelease),
        (
            |r: &mut Release| r.tag_name = "v1.1.0-rc1".to_owned(),
            Rejected::NotAVersion,
        ),
    ] {
        let mut service = UpdateService::new(ReleaseCheck, "v1.0.0", State::default());
        let mut candidate = release("v1.1.0");
        mutate(&mut candidate);
        assert_eq!(service.handle_release(&candidate), Err(rejected));
    }
    let mut service = UpdateService::new(
        ReleaseCheck,
        "v1.0.0",
        State {
            skipped_version: Some("v1.1.0".to_owned()),
        },
    );
    assert_eq!(
        service.handle_release(&release("v1.1.0")),
        Err(Rejected::Skipped)
    );
}

#[test]
fn a_check_is_fresh_for_six_hours_and_not_after() {
    let now = 1_800_000_000;
    let at = |checked_at| CheckCache {
        checked_at,
        release: None,
    };
    assert!(!CheckCache::default().is_fresh(now), "never checked");
    assert!(at(now).is_fresh(now));
    assert!(at(now - 6 * 3600 + 1).is_fresh(now));
    assert!(!at(now - 6 * 3600).is_fresh(now), "six hours on");
    assert!(!at(now + 60).is_fresh(now), "a stamp from the future");
}

#[test]
fn the_check_cache_round_trips_and_a_bad_one_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = cache_path(&dir.path().join("nested"));
    assert_eq!(load_cache(&path), CheckCache::default());
    let cache = CheckCache {
        checked_at: 42,
        release: Some(release("v2.0.0")),
    };
    save_cache(&path, &cache).expect("saves");
    assert_eq!(load_cache(&path), cache);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("JSON");
    assert_eq!(json["checkedAt"], 42);
    std::fs::write(&path, "nope").expect("write");
    assert_eq!(load_cache(&path), CheckCache::default());
}
