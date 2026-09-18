//! The one-off notices shown in the root list, and how they go away.
//!
//! A port of `NewsService` (`src/server/src/services/news/`), minus the action
//! panel it builds.
//!
//! # A notice nobody can dismiss is a notice that has failed
//!
//! Everything here exists to make a notice disappear and stay disappeared:
//! dismissal is idempotent, it is written to disk immediately, and a notice
//! whose subject no longer applies hides itself without being dismissed at
//! all. The last one is why `activeItems` is not simply "everything not
//! dismissed".

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file dismissals are kept in, under the state directory.
pub const STATE_FILE: &str = "news.json";

/// The id of the telemetry notice.
pub const TELEMETRY_NOTICE_ID: &str = "telemetry-notice-v1";

/// One notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    /// Its id, which is also what a dismissal records.
    ///
    /// The `-v1` suffix is how the same subject gets said again later: a
    /// `telemetry-notice-v2` is a different id, so an old dismissal does not
    /// hide it.
    pub id: &'static str,
    /// The heading.
    pub title: &'static str,
    /// The body.
    pub subtitle: &'static str,
    /// The built-in icon's name.
    pub icon: &'static str,
    /// The semantic colour tinting the icon's background.
    pub icon_background_tint: &'static str,
    /// Where the notice's primary action goes.
    pub learn_more_url: &'static str,
}

/// Every notice this build knows about.
///
/// A static table, as `allItems()` is: notices are written into the binary and
/// released with it, not fetched, so there is no version of this that can show
/// something the build did not ship.
pub const ALL_ITEMS: &[NewsItem] = &[NewsItem {
    id: TELEMETRY_NOTICE_ID,
    title: "Telemetry",
    subtitle: "We now collect basic usage statistics on startup",
    icon: "megaphone",
    icon_background_tint: "Yellow",
    learn_more_url: "https://docs.vicinae.com/telemetry",
}];

/// What is kept on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct State {
    /// The ids already dismissed.
    #[serde(default)]
    pub dismissed: Vec<String>,
}

/// The path the state lives at, under a state directory.
#[must_use]
pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_FILE)
}

/// Read the dismissals, or start with none.
///
/// A missing or unreadable file means nothing has been dismissed, which shows
/// a notice again rather than hiding one — the right way round for a failure,
/// since a notice that reappears is a nuisance and one that never appears is a
/// thing the person never learns.
#[must_use]
pub fn load_state(path: &Path) -> State {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the dismissals.
///
/// # The directory above is not created
///
/// `saveState` opens an `ofstream` and warns if it fails; unlike the update
/// and telemetry services it does not `create_directories` first. Reproduced,
/// because the state directory is made at startup and adding the call here
/// would paper over a missing one rather than let it be noticed.
///
/// # Errors
///
/// Returns the io error if the file cannot be written.
pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    let json = serde_json::to_string(state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    fs::write(path, json)
}

/// What the notices need to know about the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Relevance {
    /// `config.telemetry.systemInfo`.
    pub telemetry_system_info: bool,
}

/// The notices and which have been dismissed.
#[derive(Debug, Clone, Default)]
pub struct NewsService {
    /// Ids already dismissed, in the order they were.
    dismissed: Vec<String>,
}

impl NewsService {
    /// A service holding `state`'s dismissals.
    #[must_use]
    pub fn new(state: State) -> Self {
        Self {
            dismissed: state.dismissed,
        }
    }

    /// The dismissals, for saving.
    #[must_use]
    pub fn state(&self) -> State {
        State {
            dismissed: self.dismissed.clone(),
        }
    }

    /// Whether `id` has been dismissed.
    #[must_use]
    pub fn is_dismissed(&self, id: &str) -> bool {
        self.dismissed.iter().any(|seen| seen == id)
    }

    /// Dismiss `id`, returning whether this changed anything.
    ///
    /// Idempotent: dismissing twice does not record it twice, and does not ask
    /// for another save or another redraw. An id that is not a notice is
    /// recorded anyway, which is what makes a notice removed from the build
    /// stay dismissed if it ever comes back.
    pub fn dismiss(&mut self, id: &str) -> bool {
        if self.is_dismissed(id) {
            return false;
        }
        self.dismissed.push(id.to_owned());
        true
    }

    /// The notices to show, given the configuration.
    #[must_use]
    pub fn active_items(&self, relevance: Relevance) -> Vec<&'static NewsItem> {
        ALL_ITEMS
            .iter()
            .filter(|item| !self.is_dismissed(item.id))
            .filter(|item| is_relevant(item, relevance))
            .collect()
    }

    /// Whether anything is showing.
    #[must_use]
    pub fn has_unread_news(&self, relevance: Relevance) -> bool {
        !self.active_items(relevance).is_empty()
    }
}

/// Whether `item`'s subject still applies.
///
/// The telemetry notice is hidden when telemetry is off, which is the whole
/// point of it being a notice rather than a setting: it tells people what is
/// happening, and nothing is happening when it is off.
#[must_use]
pub fn is_relevant(item: &NewsItem, relevance: Relevance) -> bool {
    if item.id == TELEMETRY_NOTICE_ID {
        return relevance.telemetry_system_info;
    }
    true
}
