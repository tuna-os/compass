//! The extension store: which extensions are offered, and what they are called
//! once they are.
//!
//! A port of `VicinaeStoreService` and the `VicinaeStore` types
//! (`src/server/src/services/extension-store/`), minus the HTTP client.
//!
//! # Two rules run over every response
//!
//! Extensions that do not list this platform are dropped, and every surviving
//! one has its id rewritten to `store.vicinae.<name>`. Both happen in
//! `postProcess`, on the way out of *every* request, which is why they are one
//! function here too: a search that skipped them would offer a macOS-only
//! extension on Linux and file it under an id nothing else uses.

use serde::{Deserialize, Serialize};

/// The default page requested, from `ListPaginationOptions`.
pub const DEFAULT_PAGE: u32 = 1;

/// The default page size.
pub const DEFAULT_LIMIT: u32 = 50;

/// The page size `fetchAll` asks for.
pub const FETCH_ALL_LIMIT: u32 = 500;

/// The prefix every store extension's id is rewritten with.
pub const ID_PREFIX: &str = "store.vicinae.";

/// The icon an extension falls back to when it ships none.
pub const FALLBACK_ICON: &str = "plug";

/// Which page of the list to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListPaginationOptions {
    /// The page, one-based.
    pub page: u32,
    /// How many per page.
    pub limit: u32,
}

impl Default for ListPaginationOptions {
    fn default() -> Self {
        Self {
            page: DEFAULT_PAGE,
            limit: DEFAULT_LIMIT,
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

/// Which way round the theme is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeVariant {
    /// A light theme.
    #[default]
    Light,
    /// A dark theme.
    Dark,
}

impl Icons {
    /// The icon to show under `variant`, if there is one.
    ///
    /// A dark theme prefers the dark icon; otherwise light wins, and dark is
    /// taken as a last resort. The asymmetry is deliberate in the C++ and kept
    /// here: an extension that ships only a dark icon still gets *an* icon
    /// under a light theme, which looks wrong but is better than a blank.
    #[must_use]
    pub fn themed_icon(&self, variant: ThemeVariant) -> Option<&str> {
        if variant == ThemeVariant::Dark && self.dark.is_some() {
            return self.dark.as_deref();
        }
        self.light.as_deref().or(self.dark.as_deref())
    }
}

/// Who wrote an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    /// Their handle.
    pub handle: String,
    /// Their display name.
    pub name: String,
    /// Their avatar.
    pub avatar_url: String,
    /// Their profile page.
    pub profile_url: String,
}

/// A category an extension is filed under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Category {
    /// The category's id.
    pub id: String,
    /// Its display name.
    pub name: String,
}

/// One command an extension provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    /// The command's id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its title.
    pub title: String,
    /// Its subtitle.
    pub subtitle: String,
    /// What it does.
    pub description: String,
    /// Extra search terms.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// `view` or `no-view`.
    pub mode: String,
    /// Whether it is off until enabled.
    #[serde(default)]
    pub disabled_by_default: bool,
    /// Whether it is marked beta.
    #[serde(default)]
    pub beta: bool,
    /// Its icons.
    #[serde(default)]
    pub icons: Icons,
}

/// One extension in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    /// Its id, rewritten by [`post_process`].
    pub id: String,
    /// Its name, which the rewritten id is built from.
    pub name: String,
    /// Its title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// Who wrote it.
    #[serde(default)]
    pub author: Author,
    /// How many times it has been downloaded.
    #[serde(default)]
    pub download_count: i64,
    /// The API version it was built against.
    #[serde(default)]
    pub api_version: String,
    /// Its checksum.
    #[serde(default)]
    pub checksum: String,
    /// Whether the store marks it trending.
    #[serde(default)]
    pub trending: bool,
    /// Its icons.
    #[serde(default)]
    pub icons: Icons,
    /// Its categories.
    #[serde(default)]
    pub categories: Vec<Category>,
    /// The platforms it runs on; empty means all of them.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Its commands.
    #[serde(default)]
    pub commands: Vec<Command>,
    /// Where its source is.
    #[serde(default)]
    pub source_url: String,
    /// Where its README is.
    #[serde(default)]
    pub readme_url: String,
    /// Where its bundle is.
    #[serde(default)]
    pub download_url: String,
}

impl Extension {
    /// The icon to show, falling back to the built-in plug.
    ///
    /// Unlike a command's, an extension's icon is never absent: a store list
    /// with holes in it reads as broken rather than as sparse.
    #[must_use]
    pub fn themed_icon(&self, variant: ThemeVariant) -> &str {
        self.icons.themed_icon(variant).unwrap_or(FALLBACK_ICON)
    }
}

/// Where in the list a response sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    /// The page returned.
    pub page: u32,
    /// The page size.
    pub limit: u32,
    /// How many extensions there are in all.
    #[serde(default)]
    pub total: u32,
    /// How many pages that is.
    #[serde(default)]
    pub total_pages: u32,
    /// Whether there is a page after this one.
    #[serde(default)]
    pub has_next: bool,
    /// Whether there is one before it.
    #[serde(default)]
    pub has_prev: bool,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            page: DEFAULT_PAGE,
            limit: DEFAULT_LIMIT,
            total: 0,
            total_pages: 0,
            has_next: false,
            has_prev: false,
        }
    }
}

/// A page of the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListResponse {
    /// The extensions on it.
    #[serde(default)]
    pub extensions: Vec<Extension>,
    /// Where it sits.
    #[serde(default)]
    pub pagination: Pagination,
}

/// Whether `extension` runs here.
///
/// An empty platform list means every platform, which is how an extension that
/// is pure TypeScript avoids having to name them all.
#[must_use]
pub fn available_on(extension: &Extension, platform: &str) -> bool {
    extension.platforms.is_empty()
        || extension
            .platforms
            .iter()
            .any(|listed| listed.to_lowercase() == platform)
}

/// Drop what does not run here, and give what remains its store id.
pub fn post_process(response: &mut ListResponse, platform: &str) {
    response
        .extensions
        .retain(|extension| available_on(extension, platform));
    for extension in &mut response.extensions {
        extension.id = format!("{ID_PREFIX}{}", extension.name);
    }
}

/// Percent-encode `text` for use as a query-string value.
///
/// # A divergence: the search query is escaped
///
/// The C++ builds `"/store/search?q=%1"` with `QString::arg`, which substitutes
/// the query verbatim. A search containing `&`, `#` or `=` therefore changes
/// the shape of the URL rather than the value of `q` — searching for `a & b`
/// asks the server for `q=a ` plus a parameter called ` b`. This escapes, so
/// the query means what was typed. Everything unreserved is unchanged, so an
/// ordinary search produces the same URL as before.
#[must_use]
pub fn encode_query_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The path that lists a page of the store.
#[must_use]
pub fn list_path(options: ListPaginationOptions) -> String {
    format!("/store/list?page={}&limit={}", options.page, options.limit)
}

/// The path `fetchAll` asks for: the first page, five hundred at a time.
#[must_use]
pub fn fetch_all_path() -> String {
    list_path(ListPaginationOptions {
        limit: FETCH_ALL_LIMIT,
        ..ListPaginationOptions::default()
    })
}

/// The path that searches the store for `query`.
#[must_use]
pub fn search_path(query: &str) -> String {
    format!("/store/search?q={}", encode_query_value(query))
}
