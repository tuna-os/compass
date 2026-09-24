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

/// Reads a field the API may send as `null`, as its default.
///
/// The stores send `null` for fields that are usually strings (a command's
/// `subtitle`, a Raycast listing's `readme_url`); the C++ reads them through
/// glaze, which leaves the default in place. A strict reader refused the
/// whole listing over one such field.
fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

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
    #[serde(default, deserialize_with = "nullable")]
    pub handle: String,
    /// Their display name.
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    /// Their avatar.
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_url: String,
    /// Their profile page.
    #[serde(default, deserialize_with = "nullable")]
    pub profile_url: String,
}

/// A category an extension is filed under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Category {
    /// The category's id.
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    /// Its display name.
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
}

/// One command an extension provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    /// The command's id.
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    /// Its name.
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    /// Its title.
    #[serde(default, deserialize_with = "nullable")]
    pub title: String,
    /// Its subtitle.
    #[serde(default, deserialize_with = "nullable")]
    pub subtitle: String,
    /// What it does.
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    /// Extra search terms.
    #[serde(default, deserialize_with = "nullable")]
    pub keywords: Vec<String>,
    /// `view` or `no-view`.
    #[serde(default, deserialize_with = "nullable")]
    pub mode: String,
    /// Whether it is off until enabled.
    #[serde(default, deserialize_with = "nullable")]
    pub disabled_by_default: bool,
    /// Whether it is marked beta.
    #[serde(default, deserialize_with = "nullable")]
    pub beta: bool,
    /// Its icons.
    #[serde(default, deserialize_with = "nullable")]
    pub icons: Icons,
}

/// One extension in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    /// Its id, rewritten by [`post_process`].
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    /// Its name, which the rewritten id is built from.
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    /// Its title.
    #[serde(default, deserialize_with = "nullable")]
    pub title: String,
    /// What it does.
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    /// Who wrote it.
    #[serde(default, deserialize_with = "nullable")]
    pub author: Author,
    /// How many times it has been downloaded.
    #[serde(default, deserialize_with = "nullable")]
    pub download_count: i64,
    /// The API version it was built against.
    #[serde(default, deserialize_with = "nullable")]
    pub api_version: String,
    /// Its checksum.
    #[serde(default, deserialize_with = "nullable")]
    pub checksum: String,
    /// Whether the store marks it trending.
    #[serde(default, deserialize_with = "nullable")]
    pub trending: bool,
    /// Its icons.
    #[serde(default, deserialize_with = "nullable")]
    pub icons: Icons,
    /// Its categories.
    #[serde(default, deserialize_with = "nullable")]
    pub categories: Vec<Category>,
    /// The platforms it runs on; empty means all of them.
    #[serde(default, deserialize_with = "nullable")]
    pub platforms: Vec<String>,
    /// Its commands.
    #[serde(default, deserialize_with = "nullable")]
    pub commands: Vec<Command>,
    /// Where its source is.
    #[serde(default, deserialize_with = "nullable")]
    pub source_url: String,
    /// Where its README is.
    #[serde(default, deserialize_with = "nullable")]
    pub readme_url: String,
    /// Where its bundle is.
    #[serde(default, deserialize_with = "nullable")]
    pub download_url: String,
    /// When it was first published, as the API's ISO 8601 text.
    #[serde(default)]
    pub created_at: Option<String>,
    /// When it was last published, as the API's ISO 8601 text.
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Extension {
    /// What identifies the build the store serves now: the checksum, or the
    /// publication time when the store sent none.
    ///
    /// Kept beside an installed copy, it is what update detection compares.
    #[must_use]
    pub fn version_key(&self) -> String {
        if self.checksum.is_empty() {
            self.updated_at.clone().unwrap_or_default()
        } else {
            self.checksum.clone()
        }
    }

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
    #[serde(default, deserialize_with = "nullable")]
    pub page: u32,
    /// The page size.
    #[serde(default, deserialize_with = "nullable")]
    pub limit: u32,
    /// How many extensions there are in all.
    #[serde(default, deserialize_with = "nullable")]
    pub total: u32,
    /// How many pages that is.
    #[serde(default, deserialize_with = "nullable")]
    pub total_pages: u32,
    /// Whether there is a page after this one.
    #[serde(default, deserialize_with = "nullable")]
    pub has_next: bool,
    /// Whether there is one before it.
    #[serde(default, deserialize_with = "nullable")]
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
    #[serde(default, deserialize_with = "nullable")]
    pub extensions: Vec<Extension>,
    /// Where it sits.
    #[serde(default, deserialize_with = "nullable")]
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
    percent_encoding::utf8_percent_encode(text, crate::uri::UNRESERVED).to_string()
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

/// The extensions matching `query`, best first; every one, in the store's
/// order, for an empty query.
///
/// The C++ `FuzzySearchable<VicinaeStoreEntry>`: the title at full weight,
/// the author's name at half and the description at 0.3. The list is
/// fetched once and filtered locally as the user types, never searched on
/// the server — the store's own `search` endpoint is used by nothing.
#[must_use]
pub fn filter<'a>(extensions: &'a [Extension], query: &str) -> Vec<&'a Extension> {
    use compass_search::{Query, WeightedField, score_weighted};
    if query.trim().is_empty() {
        return extensions.iter().collect();
    }
    let parsed = Query::new(query);
    let mut scored: Vec<(u32, usize, &Extension)> = extensions
        .iter()
        .enumerate()
        .filter_map(|(position, extension)| {
            let matched = score_weighted(
                &[
                    WeightedField::new(&extension.title, 1.0),
                    WeightedField::new(&extension.author.name, 0.5),
                    WeightedField::new(&extension.description, 0.3),
                ],
                &parsed,
            );
            matched
                .accepted()
                .then_some((matched.score, position, extension))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .map(|(_, _, extension)| extension)
        .collect()
}

/// The extension `author` published as `name`, as the detail view finds it
/// in the full list.
#[must_use]
pub fn find<'a>(extensions: &'a [Extension], author: &str, name: &str) -> Option<&'a Extension> {
    extensions
        .iter()
        .find(|extension| extension.author.handle == author && extension.name == name)
}
