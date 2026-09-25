//! The update check, engine side: asking Compass's GitHub releases whether a
//! newer version is out.
//!
//! Ports the fetching half of `UpdateService`
//! (`src/server/src/services/update/`); which release is an offer is
//! [`compass_core::update`]. Compass only checks, it never installs
//! ([`ReleaseCheck`]): the package manager that installed it updates it.
//!
//! The launcher asks ([`compass_ipc::Request::UpdateStatus`]) each time it
//! opens, and the engine answers from the last check while it is younger than
//! [`update::CHECK_INTERVAL_SECS`], so however often the launcher opens the
//! feed is asked at most every six hours, as the C++ timer asks it. The last
//! answer is kept in the cache directory, so a restarted engine does not ask
//! again either. Nothing is asked with `launcher.check_for_updates` off.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use compass_core::update::{self, CheckCache, Release, ReleaseCheck, UpdateService};
use compass_ipc::UpdateOffer;

/// How long one check may take, connection to last byte.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The largest release document accepted. GitHub's is a few kilobytes plus
/// the release notes.
const MAX_RELEASE_BYTES: u64 = 4 * 1024 * 1024;

/// Where the latest release comes from.
///
/// Blocking: [`Updates`] calls it on the blocking pool. A trait so tests can
/// answer without a network.
pub trait ReleaseFeed: Send + Sync + fmt::Debug {
    /// The latest published release.
    ///
    /// # Errors
    ///
    /// Why it could not be read, for the log.
    fn latest(&self) -> Result<Release, String>;
}

/// The GitHub REST feed, `repos/<owner>/<repo>/releases/latest`, read with
/// the engine's HTTP client.
#[derive(Debug, Clone)]
pub struct GithubFeed {
    url: String,
}

impl GithubFeed {
    /// A feed at `url`.
    #[must_use]
    pub const fn new(url: String) -> Self {
        Self { url }
    }
}

impl ReleaseFeed for GithubFeed {
    fn latest(&self) -> Result<Release, String> {
        let mut response = crate::stores::agent()
            .get(&self.url)
            .header("Accept", "application/vnd.github+json")
            .config()
            .timeout_global(Some(TIMEOUT))
            .build()
            .call()
            .map_err(|err| err.to_string())?;
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RELEASE_BYTES)
            .read_to_vec()
            .map_err(|err| err.to_string())?;
        serde_json::from_slice(&body)
            .map_err(|err| format!("the feed answered unexpectedly: {err}"))
    }
}

/// A feed that is never reached: the engine built for a test.
#[derive(Debug, Clone, Copy)]
struct NoFeed;

impl ReleaseFeed for NoFeed {
    fn latest(&self) -> Result<Release, String> {
        Err("no release feed".to_owned())
    }
}

/// The engine's update check.
#[derive(Debug)]
pub struct Updates {
    feed: Arc<dyn ReleaseFeed>,
    /// The running version's tag, such as `v0.1.0`.
    current: String,
    /// `updates.json` under the state directory: the skipped version.
    state_path: Option<PathBuf>,
    /// The last check, under the cache directory.
    cache_path: Option<PathBuf>,
    /// Held across a check, so two launchers opening at once ask once.
    checking: tokio::sync::Mutex<()>,
}

impl Default for Updates {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl Updates {
    /// Compass's releases, or the feed `COMPASS_UPDATE_FEED_URL` names, and
    /// this build's version, or the one `COMPASS_UPDATE_VERSION` names.
    #[must_use]
    pub fn from_environment() -> Self {
        let env =
            |name| compass_xdg::brand::env_var(name).filter(|value: &String| !value.is_empty());
        let url = update::feed_url(env(update::FEED_URL_ENV).as_deref());
        let current = env(update::VERSION_OVERRIDE_ENV)
            .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
        Self::new(
            Arc::new(GithubFeed::new(url)),
            current,
            compass_core::xdg_dirs::state_dir().map(|dir| update::state_path(&dir)),
            compass_core::xdg_dirs::cache_home()
                .map(|dir| update::cache_path(&dir.join("compass"))),
        )
    }

    /// A check over `feed` for a build tagged `current`, keeping its state
    /// and cache at these paths (nowhere, when `None`).
    #[must_use]
    pub fn new(
        feed: Arc<dyn ReleaseFeed>,
        current: String,
        state_path: Option<PathBuf>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        Self {
            feed,
            current,
            state_path,
            cache_path,
            checking: tokio::sync::Mutex::new(()),
        }
    }

    /// A check that never reaches a feed and keeps nothing, for an engine
    /// built around a test's index.
    #[must_use]
    pub fn offline() -> Self {
        Self::new(
            Arc::new(NoFeed),
            format!("v{}", env!("CARGO_PKG_VERSION")),
            None,
            None,
        )
    }

    /// The running version's tag.
    #[must_use]
    pub fn current(&self) -> &str {
        &self.current
    }

    /// The newer release, if there is one, asking the feed first when the
    /// last check is six hours old or more. `now` is seconds since the epoch.
    ///
    /// `None` when checking is off (nothing is asked), when this build's
    /// version is not a release tag (the C++'s `checksSupported`), when the
    /// feed has never answered, and when the latest release is not newer,
    /// is a draft or prerelease, or was skipped.
    pub async fn status(&self, enabled: bool, now: i64) -> Option<UpdateOffer> {
        if !enabled {
            return None;
        }
        let state = self
            .state_path
            .as_deref()
            .map(update::load_state)
            .unwrap_or_default();
        let mut service = UpdateService::new(ReleaseCheck, &self.current, state);
        if !service.checks_supported() {
            return None;
        }
        let release = self.latest(now).await?;
        let offer = service.handle_release(&release).ok()?;
        Some(UpdateOffer {
            tag: offer.tag.clone(),
            version: offer.version.clone(),
            release_url: offer.release_url.clone(),
        })
    }

    /// The latest release the feed reported, from the cache while it is
    /// fresh. A failed check is remembered as a check, keeping the release
    /// from the one before, so an unreachable feed is asked again in six
    /// hours rather than on every summon.
    async fn latest(&self, now: i64) -> Option<Release> {
        let _checking = self.checking.lock().await;
        let mut cache = self
            .cache_path
            .as_deref()
            .map(update::load_cache)
            .unwrap_or_default();
        if !cache.is_fresh(now) {
            let feed = Arc::clone(&self.feed);
            match tokio::task::spawn_blocking(move || feed.latest()).await {
                Ok(Ok(release)) => cache.release = Some(release),
                Ok(Err(reason)) => tracing::warn!(%reason, "Update check failed"),
                Err(err) => tracing::warn!(%err, "Update check failed"),
            }
            cache.checked_at = now;
            self.remember(&cache);
        }
        cache.release
    }

    fn remember(&self, cache: &CheckCache) {
        if let Some(path) = &self.cache_path
            && let Err(err) = update::save_cache(path, cache)
        {
            tracing::warn!(path = %path.display(), %err, "Failed to write the update check cache");
        }
    }

    /// Never offer `tag` again (`skipAvailableVersion`).
    ///
    /// # Errors
    ///
    /// The sentence to show when the state cannot be written.
    pub fn skip(&self, tag: &str) -> Result<(), String> {
        let path = self
            .state_path
            .as_deref()
            .ok_or_else(|| "There is no state directory to remember it in".to_owned())?;
        let mut state = update::load_state(path);
        state.skipped_version = Some(tag.to_owned());
        update::save_state(path, &state).map_err(|err| format!("Failed to skip {tag}: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A feed answering `tag`, counting how often it is asked.
    #[derive(Debug, Default)]
    struct Fake {
        tag: std::sync::Mutex<Option<String>>,
        asked: AtomicUsize,
    }

    impl Fake {
        fn answering(tag: &str) -> Arc<Self> {
            Arc::new(Self {
                tag: std::sync::Mutex::new(Some(tag.to_owned())),
                asked: AtomicUsize::new(0),
            })
        }
        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    impl ReleaseFeed for Fake {
        fn latest(&self) -> Result<Release, String> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let tag = self.tag.lock().unwrap().clone().ok_or("unreachable")?;
            Ok(Release {
                html_url: format!("https://github.com/tuna-os/compass/releases/tag/{tag}"),
                tag_name: tag,
                ..Release::default()
            })
        }
    }

    const NOW: i64 = 1_800_000_000;

    fn updates(feed: &Arc<Fake>, dir: &std::path::Path, current: &str) -> Updates {
        Updates::new(
            Arc::clone(feed) as Arc<dyn ReleaseFeed>,
            current.to_owned(),
            Some(update::state_path(&dir.join("state"))),
            Some(update::cache_path(&dir.join("cache"))),
        )
    }

    #[tokio::test]
    async fn a_newer_release_is_offered_and_asked_for_once_in_six_hours() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.2.0");
        let check = updates(&feed, dir.path(), "v0.1.0");

        let offer = check.status(true, NOW).await.expect("offered");
        assert_eq!(offer.tag, "v0.2.0");
        assert_eq!(offer.version, "0.2.0");
        assert_eq!(
            offer.release_url,
            "https://github.com/tuna-os/compass/releases/tag/v0.2.0"
        );
        assert!(check.status(true, NOW + 3600).await.is_some());
        assert_eq!(feed.asked(), 1, "answered from the cache");

        // A restarted engine reads the same cache.
        let again = updates(&feed, dir.path(), "v0.1.0");
        assert!(again.status(true, NOW + 7200).await.is_some());
        assert_eq!(feed.asked(), 1);

        assert!(check.status(true, NOW + 6 * 3600).await.is_some());
        assert_eq!(feed.asked(), 2, "six hours on, the feed is asked again");
    }

    #[tokio::test]
    async fn checking_off_asks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.2.0");
        let check = updates(&feed, dir.path(), "v0.1.0");
        assert_eq!(check.status(false, NOW).await, None);
        assert_eq!(feed.asked(), 0);
    }

    #[tokio::test]
    async fn a_build_that_is_not_a_release_asks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.2.0");
        let check = updates(&feed, dir.path(), "dev-build");
        assert_eq!(check.status(true, NOW).await, None);
        assert_eq!(feed.asked(), 0);
    }

    #[tokio::test]
    async fn the_same_version_is_not_an_offer() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.1.0");
        assert_eq!(
            updates(&feed, dir.path(), "v0.1.0").status(true, NOW).await,
            None
        );
    }

    #[tokio::test]
    async fn a_skipped_release_is_not_offered_but_the_next_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.2.0");
        let check = updates(&feed, dir.path(), "v0.1.0");
        check.skip("v0.2.0").unwrap();
        assert_eq!(check.status(true, NOW).await, None);
        let json = std::fs::read_to_string(update::state_path(&dir.path().join("state"))).unwrap();
        assert!(json.contains("\"skippedVersion\":\"v0.2.0\""), "{json}");

        *feed.tag.lock().unwrap() = Some("v0.3.0".to_owned());
        let offer = check.status(true, NOW + 6 * 3600).await.expect("offered");
        assert_eq!(offer.tag, "v0.3.0");
    }

    #[tokio::test]
    async fn an_unreachable_feed_keeps_the_last_answer_and_waits_six_hours() {
        let dir = tempfile::tempdir().unwrap();
        let feed = Fake::answering("v0.2.0");
        let check = updates(&feed, dir.path(), "v0.1.0");
        assert!(check.status(true, NOW).await.is_some());

        *feed.tag.lock().unwrap() = None;
        let later = NOW + 6 * 3600;
        assert!(
            check.status(true, later).await.is_some(),
            "the release from the check before"
        );
        assert!(check.status(true, later + 60).await.is_some());
        assert_eq!(feed.asked(), 2, "the failed check counts as one");
    }

    #[test]
    fn the_github_feed_reads_the_release_from_its_url() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let serve = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            let url = request.url().to_owned();
            let accept = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Accept"))
                .map(|header| header.value.to_string());
            let body = r#"{"tag_name": "v0.3.0", "html_url": "https://example.test/r",
                           "draft": false, "prerelease": false, "assets": [], "body": "notes"}"#;
            request
                .respond(tiny_http::Response::from_string(body))
                .unwrap();
            (url, accept)
        });
        let feed = GithubFeed::new(format!(
            "http://127.0.0.1:{port}/repos/tuna-os/compass/releases/latest"
        ));
        let release = feed.latest().expect("read");
        assert_eq!(release.tag_name, "v0.3.0");
        assert_eq!(release.html_url, "https://example.test/r");
        let (url, accept) = serve.join().unwrap();
        assert_eq!(url, "/repos/tuna-os/compass/releases/latest");
        assert_eq!(accept.as_deref(), Some("application/vnd.github+json"));
    }

    #[test]
    fn the_github_feed_reports_a_refusal() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let serve = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            request
                .respond(tiny_http::Response::from_string("rate limited").with_status_code(403))
                .unwrap();
        });
        let feed = GithubFeed::new(format!("http://127.0.0.1:{port}/latest"));
        assert!(feed.latest().is_err());
        serve.join().unwrap();
    }

    #[tokio::test]
    async fn an_offline_engine_offers_nothing() {
        assert_eq!(Updates::offline().status(true, NOW).await, None);
    }
}
