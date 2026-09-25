//! Deciding whether a GitHub release is an update worth offering.
//!
//! A port of `UpdateService` (`src/server/src/services/update/`), minus the
//! HTTP client, the timer and the platform installer.
//!
//! Compass only *checks*: it reads its own releases ([`DEFAULT_FEED_URL`]) and
//! says when a newer one is out ([`ReleaseCheck`]); installing is the package
//! manager's job. The engine does the fetching (`vicinae::updates`), at most
//! once per [`CHECK_INTERVAL_SECS`], remembering the answer in a [`CheckCache`].
//!
//! # Every gate here is a gate against offering the wrong thing
//!
//! Five conditions have to hold before a release becomes an offer: it parses
//! as a version, it is newer than this build, it is not a draft, it is not a
//! prerelease, it was not skipped, and it carries an asset this platform can
//! install. Any one of them failing is not an error — it is "no update", and
//! the difference matters: a build that treated a missing asset as a failure
//! would nag a person it cannot help.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::semver::Semver;

/// How often the C++ timer checks, in seconds — `6h`.
pub const CHECK_INTERVAL_SECS: u64 = 6 * 3600;

/// How long after installing before relaunching, from `RELAUNCH_DELAY`.
pub const RELAUNCH_DELAY_MS: u64 = 800;

/// The file the skipped version is kept in, under the state directory.
pub const STATE_FILE: &str = "updates.json";

/// Where releases are read from: Compass's own, where the C++'s
/// `Environment::updateFeedUrl` reads Vicinae's.
pub const DEFAULT_FEED_URL: &str = "https://api.github.com/repos/tuna-os/compass/releases/latest";

/// The file the last check is remembered in, under the cache directory.
pub const CACHE_FILE: &str = "latest-release.json";

/// The environment variable that overrides [`DEFAULT_FEED_URL`].
pub const FEED_URL_ENV: &str = "VICINAE_UPDATE_FEED_URL";

/// The environment variable that overrides the version being compared against.
pub const VERSION_OVERRIDE_ENV: &str = "VICINAE_UPDATE_VERSION";

/// The toast shown when an install finishes.
pub const INSTALLED_TITLE: &str = "Update installed";
/// Its body.
pub const INSTALLED_BODY: &str = "Restarting…";
/// The toast shown when anything in the install fails.
pub const FAILED_TITLE: &str = "Update failed";
/// What the null installer says on a platform that cannot self-update.
pub const UNSUPPORTED_MESSAGE: &str = "Self update is not supported on this platform";

/// The feed to read, honouring [`FEED_URL_ENV`].
#[must_use]
pub fn feed_url(env_value: Option<&str>) -> String {
    env_value
        .filter(|url| !url.is_empty())
        .unwrap_or(DEFAULT_FEED_URL)
        .to_owned()
}

/// One downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReleaseAsset {
    /// The file's name, matched exactly against the installer's.
    pub name: String,
    /// Where to fetch it.
    pub browser_download_url: String,
}

/// A GitHub release, as the feed reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Release {
    /// The tag, such as `v1.2.3`.
    pub tag_name: String,
    /// The page a person would read.
    pub html_url: String,
    /// Whether it is unpublished.
    #[serde(default)]
    pub draft: bool,
    /// Whether it is a prerelease.
    #[serde(default)]
    pub prerelease: bool,
    /// Its attached files.
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// An update this build could install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    /// The release's tag, `v` and all.
    pub tag: String,
    /// The tag without its leading `v`, for showing to a person.
    pub version: String,
    /// The release page.
    pub release_url: String,
    /// The asset to download, when the installer asked for one; `None` for
    /// a [`ReleaseCheck`], which installs nothing.
    pub asset_url: Option<String>,
}

/// Where the service is in the update cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Nothing happening, nothing offered.
    #[default]
    Idle,
    /// Asking the feed.
    Checking,
    /// Something is offered.
    UpdateAvailable,
    /// Fetching the asset.
    Downloading,
    /// Unpacking and replacing.
    Installing,
    /// Done; a relaunch follows.
    Installed,
    /// Something went wrong.
    Failed,
}

impl Status {
    /// Whether a new check or download must not start.
    ///
    /// Downloading, installing and installed are all "hands off": starting a
    /// second download over a half-written archive is how an update bricks an
    /// install.
    #[must_use]
    pub const fn is_busy(self) -> bool {
        matches!(self, Self::Downloading | Self::Installing | Self::Installed)
    }
}

/// What is kept on disk between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// A tag the person asked not to be offered again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
}

/// The path the state lives at, under a state directory.
#[must_use]
pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_FILE)
}

/// Read the state file, or start empty.
///
/// A missing or unreadable file is an empty state, which is what the C++ does
/// — and the right answer, since the only thing in it is "do not offer me this
/// version" and forgetting that offers an update rather than hiding one.
#[must_use]
pub fn load_state(path: &Path) -> State {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write `state`, creating the directory above it.
///
/// # Errors
///
/// Returns the io error if the directory or file cannot be written.
pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    fs::write(path, json)
}

/// The version shown to a person: the tag without its leading `v`.
#[must_use]
pub fn display_version(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_owned()
}

/// Whatever can actually replace this build.
pub trait UpdateInstaller {
    /// Whether this platform can self-update at all.
    fn supported(&self) -> bool;

    /// The exact asset name to look for in a release.
    ///
    /// Matched by equality, not by pattern: a release carrying assets for
    /// every platform must give this one exactly the file it can install.
    /// Empty for an installer that needs none ([`ReleaseCheck`]).
    fn asset_name(&self) -> String;
}

/// The installer on a platform that cannot self-update.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullUpdateInstaller;

impl UpdateInstaller for NullUpdateInstaller {
    fn supported(&self) -> bool {
        false
    }
    fn asset_name(&self) -> String {
        String::new()
    }
}

/// The installer of a build that only checks: it installs nothing and needs
/// no asset, so every newer published release is an offer. Compass is
/// updated by whatever installed it (a distribution package, the Flatpak).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReleaseCheck;

impl UpdateInstaller for ReleaseCheck {
    fn supported(&self) -> bool {
        true
    }
    fn asset_name(&self) -> String {
        String::new()
    }
}

/// The last answer the feed gave, so a check is made at most once per
/// [`CHECK_INTERVAL_SECS`] however often the launcher opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CheckCache {
    /// When the feed was last asked, in seconds since the epoch, whether it
    /// answered or not.
    #[serde(default)]
    pub checked_at: i64,
    /// The latest release it reported, kept across a failed check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<Release>,
}

impl CheckCache {
    /// Whether the last check is recent enough to answer from.
    ///
    /// A stamp in the future (the clock went back) is stale, so a wrong clock
    /// costs one extra check rather than silencing checks until it catches up.
    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        let age = now - self.checked_at;
        self.checked_at > 0
            && (0..i64::try_from(CHECK_INTERVAL_SECS).unwrap_or(i64::MAX)).contains(&age)
    }
}

/// The path the check cache lives at, under a cache directory.
#[must_use]
pub fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_FILE)
}

/// Read the check cache, or start with none: a missing or unreadable file
/// only means the next check asks the feed.
#[must_use]
pub fn load_cache(path: &Path) -> CheckCache {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the check cache, creating the directory above it.
///
/// # Errors
///
/// Returns the io error if the directory or file cannot be written.
pub fn save_cache(path: &Path, cache: &CheckCache) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(cache)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    fs::write(path, json)
}

/// Why a release was not offered.
///
/// Every one of these is "no update", not a failure — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// Its tag is not a release tag.
    NotAVersion,
    /// It is the same as, or older than, this build.
    NotNewer,
    /// It is unpublished.
    Draft,
    /// It is a prerelease.
    Prerelease,
    /// The person asked not to see it again.
    Skipped,
    /// It has nothing this platform can install.
    NoAsset,
}

/// Checks the feed and decides what to offer.
#[derive(Debug)]
pub struct UpdateService<I> {
    /// What can install.
    installer: I,
    /// This build's version, if its tag parses.
    current: Option<Semver>,
    /// What is on offer.
    available: Option<AvailableUpdate>,
    /// Where in the cycle we are.
    status: Status,
    /// The skipped version.
    state: State,
}

impl<I: UpdateInstaller> UpdateService<I> {
    /// A service for a build tagged `current_tag`.
    ///
    /// A tag that is not a release tag — a development build, say — disables
    /// checking entirely rather than comparing against nothing.
    pub fn new(installer: I, current_tag: &str, state: State) -> Self {
        Self {
            installer,
            current: Semver::parse(current_tag),
            available: None,
            status: Status::Idle,
            state,
        }
    }

    /// This build's parsed version, if it has one.
    #[must_use]
    pub const fn current_version(&self) -> Option<&Semver> {
        self.current.as_ref()
    }

    /// Whether checking is possible at all.
    ///
    /// Both halves matter: a development build has nothing to compare against,
    /// and a platform with no installer has nothing to do with the answer.
    pub fn checks_supported(&self) -> bool {
        self.current.is_some() && self.installer.supported()
    }

    /// Where in the cycle the service is.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// What is on offer.
    #[must_use]
    pub const fn available(&self) -> Option<&AvailableUpdate> {
        self.available.as_ref()
    }

    /// The state, for a caller that has to save it.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// Whether a check may start now.
    pub fn may_check(&self) -> bool {
        self.checks_supported() && !self.status.is_busy()
    }

    /// Note that a check has begun.
    pub fn begin_check(&mut self) -> bool {
        if !self.may_check() {
            return false;
        }
        self.status = Status::Checking;
        true
    }

    /// Note that the feed could not be read.
    ///
    /// Only a check that is still running goes back to idle: a download that
    /// started in between must not be reset by a late failure from the check
    /// before it.
    pub fn check_failed(&mut self) {
        if self.status == Status::Checking {
            self.status = Status::Idle;
        }
    }

    /// Decide what `release` means.
    ///
    /// # Errors
    ///
    /// Returns why it was not offered; the service goes idle in every case.
    pub fn handle_release(&mut self, release: &Release) -> Result<&AvailableUpdate, Rejected> {
        let rejection = self.evaluate(release);
        if let Some(rejection) = rejection {
            self.available = None;
            self.status = Status::Idle;
            return Err(rejection);
        }

        let asset_name = self.installer.asset_name();
        let asset_url = (!asset_name.is_empty())
            .then(|| release.assets.iter().find(|asset| asset.name == asset_name))
            .flatten()
            .map(|asset| asset.browser_download_url.clone());

        self.available = Some(AvailableUpdate {
            tag: release.tag_name.clone(),
            version: display_version(&release.tag_name),
            release_url: release.html_url.clone(),
            asset_url,
        });
        self.status = Status::UpdateAvailable;
        Ok(self.available.as_ref().expect("just set"))
    }

    /// Why `release` is not an offer, if it is not.
    fn evaluate(&self, release: &Release) -> Option<Rejected> {
        let Some(remote) = Semver::parse(&release.tag_name) else {
            return Some(Rejected::NotAVersion);
        };
        let Some(current) = self.current.as_ref() else {
            return Some(Rejected::NotAVersion);
        };
        if remote.cmp(current) != Ordering::Greater {
            return Some(Rejected::NotNewer);
        }
        if release.draft {
            return Some(Rejected::Draft);
        }
        if release.prerelease {
            return Some(Rejected::Prerelease);
        }
        if self.state.skipped_version.as_deref() == Some(release.tag_name.as_str()) {
            return Some(Rejected::Skipped);
        }
        let asset_name = self.installer.asset_name();
        if !asset_name.is_empty() && !release.assets.iter().any(|asset| asset.name == asset_name) {
            return Some(Rejected::NoAsset);
        }
        None
    }

    /// Whether a download may start.
    pub fn may_download(&self) -> bool {
        self.available
            .as_ref()
            .is_some_and(|update| update.asset_url.is_some())
            && !self.status.is_busy()
    }

    /// Begin downloading the offered update.
    pub fn begin_download(&mut self) -> Option<&AvailableUpdate> {
        if !self.may_download() {
            return None;
        }
        self.status = Status::Downloading;
        self.available.as_ref()
    }

    /// The download finished; installing begins.
    pub fn begin_install(&mut self) {
        self.status = Status::Installing;
    }

    /// The install finished.
    pub fn finish_install(&mut self) {
        self.status = Status::Installed;
    }

    /// Something in the download or install failed.
    pub fn fail(&mut self) {
        self.status = Status::Failed;
    }

    /// Never offer the current offer again.
    ///
    /// Returns the tag that was skipped, so the caller can save the state and
    /// say so. The offer is dropped and the service goes idle, which is what
    /// makes the next check quiet rather than immediately re-offering it.
    pub fn skip_available_version(&mut self) -> Option<String> {
        let tag = self.available.take()?.tag;
        self.state.skipped_version = Some(tag.clone());
        self.status = Status::Idle;
        Some(tag)
    }

    /// The toast shown while downloading.
    #[must_use]
    pub fn downloading_toast(tag: &str) -> String {
        format!("Downloading Vicinae {tag}…")
    }

    /// The toast shown while downloading, with progress.
    ///
    /// Only shown when the total size is known: a percentage of an unknown
    /// total is a number that means nothing.
    #[must_use]
    pub fn downloading_progress_toast(tag: &str, received: i64, total: i64) -> Option<String> {
        if total <= 0 {
            return None;
        }
        let percent = received * 100 / total;
        Some(format!("Downloading Vicinae {tag}… {percent}%"))
    }
}
