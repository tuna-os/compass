//! The Raycast store: which of its extensions are offered here, and what they
//! are called once they are.
//!
//! A port of `RaycastStoreService` (`src/server/src/services/raycast/`), minus
//! the HTTP client.
//!
//! # Linux takes everything, on purpose
//!
//! Raycast is a macOS product, so its extensions do not advertise Linux
//! compatibility — the platform list, where there is one, says `macos`.
//! Filtering on it here would offer nothing at all. So on a platform Raycast
//! does not itself run on, the filter is skipped entirely and every extension
//! is offered, best-effort. The C++ comment says exactly this, and it is the
//! difference between a store with thousands of entries and an empty one.
//!
//! The compatibility sheet is the other half of that bargain: it is fetched
//! only where the filter was skipped, and says which extensions actually work.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Where the Raycast API lives.
pub const API_BASE_URL: &str = "https://backend.raycast.com/api/v1";

/// The prefix every Raycast extension's id is rewritten with.
///
/// Distinct from the Vicinae store's `store.vicinae.` so a root-search row
/// from one is never mistaken for the other.
pub const ID_PREFIX: &str = "store.raycast.";

/// The endpoint the compatibility sheet comes from, on the Vicinae API.
pub const COMPAT_PATH: &str = "/raycast/get-compat";

/// The default page requested.
pub const DEFAULT_PAGE: u32 = 1;

/// The default page size.
pub const DEFAULT_PER_PAGE: u32 = 50;

/// Which page of the listing to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListPaginationOptions {
    /// The page, one-based.
    pub page: u32,
    /// How many per page.
    pub per_page: u32,
}

impl Default for ListPaginationOptions {
    fn default() -> Self {
        Self {
            page: DEFAULT_PAGE,
            per_page: DEFAULT_PER_PAGE,
        }
    }
}

/// The light and dark icons an extension or command ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Icons {
    /// The icon for a light theme.
    #[serde(default)]
    pub light: Option<String>,
    /// The icon for a dark theme.
    #[serde(default)]
    pub dark: Option<String>,
}

/// One command a Raycast extension provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Command {
    /// Its id.
    #[serde(default)]
    pub id: String,
    /// Its name.
    #[serde(default)]
    pub name: String,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// Its subtitle.
    #[serde(default)]
    pub subtitle: String,
    /// What it does.
    #[serde(default)]
    pub description: String,
    /// Extra search terms.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// `view` or `no-view`.
    #[serde(default)]
    pub mode: String,
    /// Whether it is off until enabled.
    #[serde(default)]
    pub disabled_by_default: bool,
    /// Whether it is marked beta.
    #[serde(default)]
    pub beta: bool,
    /// Its own icons.
    #[serde(default)]
    pub icons: Icons,
    /// Its extension's icons, copied in by [`post_process_extension`].
    ///
    /// Not sent by the API: a command row has to show *something*, and a
    /// command without its own icon falls back to its extension's. Carrying
    /// the copy on the command is what lets a row be rendered without a
    /// reference back to the extension it came from.
    #[serde(default, skip_deserializing)]
    pub extension_icons: Icons,
}

/// One extension in the Raycast store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Extension {
    /// Its id, rewritten by [`post_process_extension`].
    #[serde(default)]
    pub id: String,
    /// Its name, which the rewritten id is built from.
    #[serde(default)]
    pub name: String,
    /// The platforms it advertises. Absent means every platform.
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    /// Where its listing is.
    #[serde(default)]
    pub store_url: String,
    /// Where its bundle is.
    #[serde(default)]
    pub download_url: String,
    /// Its icons.
    #[serde(default)]
    pub icons: Icons,
    /// Its commands.
    #[serde(default)]
    pub commands: Vec<Command>,
}

/// A page of the listing, as the API returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListApiResponse {
    /// The extensions on it.
    #[serde(default)]
    pub data: Vec<Extension>,
}

/// What this build knows about one extension's compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CompatInfo {
    /// Whether it works.
    #[serde(default)]
    pub status: String,
    /// How sure we are.
    #[serde(default)]
    pub confidence: String,
    /// Anything worth saying about it.
    #[serde(default)]
    pub notes: Option<Vec<String>>,
    /// Whether something else does the same job.
    #[serde(default)]
    pub has_equivalent: Option<bool>,
}

/// The compatibility sheet, by extension name.
pub type CompatMap = HashMap<String, CompatInfo>;

/// Whether Raycast itself runs on this platform.
///
/// True on macOS and Windows, false everywhere else — which is what decides
/// whether the platform filter runs at all.
#[must_use]
pub const fn is_native_platform(platform: &str) -> bool {
    matches!(platform.as_bytes(), b"macos" | b"windows")
}

/// Whether a compatibility sheet exists for this platform.
///
/// Linux only: it is a record of what works on the platform Raycast does not
/// support, so on a platform Raycast does support there is nothing to say.
#[must_use]
pub const fn has_compat_sheet(platform: &str) -> bool {
    matches!(platform.as_bytes(), b"linux")
}

/// Whether `extension` is offered on `platform`.
#[must_use]
pub fn available_on(extension: &Extension, platform: &str) -> bool {
    // Raycast will not advertise Linux compatibility, and we want its
    // extensions on Linux anyway. Filtering here would offer nothing at all.
    if !is_native_platform(platform) {
        return true;
    }
    match &extension.platforms {
        None => true,
        Some(platforms) => platforms
            .iter()
            .any(|listed| listed.to_lowercase() == platform),
    }
}

/// Give `extension` its store id and push its icons onto its commands.
pub fn post_process_extension(extension: &mut Extension) {
    extension.id = format!("{ID_PREFIX}{}", extension.name);
    for command in &mut extension.commands {
        command.extension_icons = extension.icons.clone();
    }
}

/// Drop what is not offered here, and post-process what remains.
pub fn post_process(extensions: &mut Vec<Extension>, platform: &str) {
    extensions.retain(|extension| available_on(extension, platform));
    for extension in extensions.iter_mut() {
        post_process_extension(extension);
    }
}

/// Percent-encode a query-string value.
///
/// # A divergence, the same one as the Vicinae store's
///
/// `search` builds `"/store_listings/search?q=%1"` with `QString::arg`, and
/// `fetchExtension` builds `"/extensions/%1/%2"` the same way. A query or an
/// extension name containing `&`, `#` or `/` therefore changes the URL's shape
/// rather than its values. Escaped here, with every unreserved character left
/// alone so an ordinary request is unchanged.
#[must_use]
pub fn encode_path_value(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, crate::uri::UNRESERVED).to_string()
}

/// The path that lists a page of the store.
#[must_use]
pub fn list_path(options: ListPaginationOptions) -> String {
    format!(
        "/store_listings?page={}&per_page={}",
        options.page, options.per_page
    )
}

/// The path that searches the store.
#[must_use]
pub fn search_path(query: &str) -> String {
    format!("/store_listings/search?q={}", encode_path_value(query))
}

/// The path for one extension by author and name.
#[must_use]
pub fn extension_path(author: &str, name: &str) -> String {
    format!(
        "/extensions/{}/{}",
        encode_path_value(author),
        encode_path_value(name)
    )
}

/// Holds the pages already fetched and the compatibility sheet.
#[derive(Debug, Clone, Default)]
pub struct RaycastStoreService {
    /// Pages already fetched, by page number.
    cached_pages: HashMap<u32, Vec<Extension>>,
    /// The compatibility sheet, once fetched.
    compat: CompatMap,
    /// Whether it has been fetched successfully.
    compat_fetched: bool,
}

impl RaycastStoreService {
    /// A service with nothing cached.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The page already held, if any.
    ///
    /// The listing does not change from one keystroke to the next, and paging
    /// back and forth is the normal way to browse it, so a page fetched once
    /// is kept for the session.
    #[must_use]
    pub fn cached_page(&self, page: u32) -> Option<&[Extension]> {
        self.cached_pages.get(&page).map(Vec::as_slice)
    }

    /// Record a fetched page, post-processed.
    pub fn store_page(
        &mut self,
        page: u32,
        mut extensions: Vec<Extension>,
        platform: &str,
    ) -> &[Extension] {
        post_process(&mut extensions, platform);
        self.cached_pages.entry(page).or_insert(extensions)
    }

    /// The compatibility sheet as it stands.
    #[must_use]
    pub const fn compat(&self) -> &CompatMap {
        &self.compat
    }

    /// Whether the compatibility sheet should be fetched now.
    ///
    /// Not on a platform that has no sheet, and not again once one arrived.
    #[must_use]
    pub fn should_fetch_compat(&self, platform: &str) -> bool {
        has_compat_sheet(platform) && !self.compat_fetched
    }

    /// Record a fetched sheet.
    pub fn set_compat(&mut self, compat: CompatMap) {
        self.compat = compat;
        self.compat_fetched = true;
    }

    /// Record that the fetch failed.
    ///
    /// The C++ warns, returns an *empty map* rather than an error, and leaves
    /// `m_compatFetched` false. So a failure is not fatal — the store still
    /// works, just without compatibility notes — and the next request tries
    /// again. Both halves are reproduced.
    pub fn compat_fetch_failed(&mut self) -> CompatMap {
        CompatMap::new()
    }

    /// What is known about `name`, if anything.
    #[must_use]
    pub fn compat_for(&self, name: &str) -> Option<&CompatInfo> {
        self.compat.get(name)
    }
}
