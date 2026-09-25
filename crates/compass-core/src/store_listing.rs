//! The two stores' extensions in one shape, and the detail page drawn from it.
//!
//! The C++ gives each store its own view host over one QML component
//! (`StoreListingView`, `StoreDetailView`), which reads the same properties
//! from both. This is that property set: whatever store an extension came
//! from, the launcher draws a [`Listing`].

use crate::raycast_store_view::Alert;

/// Which store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Store {
    /// The Vicinae extension store (`api.vicinae.com`).
    Vicinae,
    /// The Raycast store (`backend.raycast.com`).
    Raycast,
}

impl Store {
    /// Its name in a [`crate::store_bundle::Marker`].
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Vicinae => "vicinae",
            Self::Raycast => "raycast",
        }
    }

    /// The id an extension `name` from this store is installed under.
    #[must_use]
    pub fn extension_id(self, name: &str) -> String {
        match self {
            Self::Vicinae => format!("{}{name}", crate::extension_store::ID_PREFIX),
            Self::Raycast => format!("{}{name}", crate::raycast_store::ID_PREFIX),
        }
    }

    /// The search field's placeholder, `setSearchPlaceholderText`.
    #[must_use]
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Vicinae => "Browse Vicinae extensions",
            Self::Raycast => "Browse Raycast extensions",
        }
    }
}

/// Where "Report issue" goes, `Omnicast::GH_EXTENSIONS_CREATE_ISSUE`.
pub const REPORT_ISSUE_URL: &str = "https://github.com/vicinaehq/extensions/issues/new/choose";

/// The navigation title of either store's list.
pub const NAVIGATION_TITLE: &str = "Extension Store";

/// What a failed list fetch says, as a toast in the C++.
pub const FETCH_FAILED: &str = "Failed to fetch extensions";

/// What a failed search says.
pub const SEARCH_FAILED: &str = "Failed to search extensions";

/// One command, as the detail page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListingCommand {
    /// Its title.
    pub title: String,
    /// What it does.
    pub description: String,
}

/// One extension, from either store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    /// The id it installs under (`store.vicinae.<name>`, `store.raycast.<name>`).
    pub id: String,
    /// Its name in the store.
    pub name: String,
    /// Its author's handle.
    pub author: String,
    /// Its author's display name.
    pub author_name: String,
    /// Its author's avatar.
    pub author_avatar: Option<String>,
    /// Its title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// Its icon for a light theme.
    pub icon_light: Option<String>,
    /// Its icon for a dark theme.
    pub icon_dark: Option<String>,
    /// Its download count, formatted as the C++ formats it.
    pub downloads: String,
    /// The store's version key for the build it serves.
    pub version: String,
    /// When it was last published, as a date, when the store says.
    pub updated: Option<String>,
    /// Its categories' names.
    pub categories: Vec<String>,
    /// The platforms it names.
    pub platforms: Vec<String>,
    /// Its commands.
    pub commands: Vec<ListingCommand>,
    /// Who else worked on it (Raycast only).
    pub contributors: Vec<String>,
    /// Where its README is.
    pub readme_url: Option<String>,
    /// Where its source is.
    pub source_url: Option<String>,
    /// Its page on the store's website (Raycast only).
    pub store_url: Option<String>,
    /// Where its bundle is.
    pub download_url: String,
    /// Its screenshots (Raycast only; the Vicinae store has none, and its
    /// detail host answers `hasScreenshots` with false).
    pub screenshots: Vec<String>,
}

fn non_empty(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_owned())
}

impl Listing {
    /// A Vicinae store extension, post-processed.
    #[must_use]
    pub fn from_vicinae(extension: &crate::extension_store::Extension) -> Self {
        Self {
            id: extension.id.clone(),
            name: extension.name.clone(),
            author: extension.author.handle.clone(),
            author_name: extension.author.name.clone(),
            author_avatar: non_empty(&extension.author.avatar_url),
            title: extension.title.clone(),
            description: extension.description.clone(),
            icon_light: extension.icons.light.clone(),
            icon_dark: extension.icons.dark.clone(),
            downloads: crate::raycast_store_view::format_count(extension.download_count),
            version: extension.version_key(),
            // ISO 8601; the date is its first ten characters.
            updated: extension
                .updated_at
                .as_deref()
                .and_then(|at| at.get(..10))
                .map(str::to_owned),
            categories: extension
                .categories
                .iter()
                .map(|category| category.name.clone())
                .collect(),
            platforms: extension.platforms.clone(),
            commands: extension
                .commands
                .iter()
                .map(|command| ListingCommand {
                    title: command.title.clone(),
                    description: command.description.clone(),
                })
                .collect(),
            contributors: Vec::new(),
            readme_url: non_empty(&extension.readme_url),
            source_url: non_empty(&extension.source_url),
            store_url: None,
            download_url: extension.download_url.clone(),
            screenshots: Vec::new(),
        }
    }

    /// A Raycast store extension, post-processed; `updated` is its
    /// publication date, formatted by the caller (which has a calendar).
    #[must_use]
    pub fn from_raycast(
        extension: &crate::raycast_store::Extension,
        updated: Option<String>,
    ) -> Self {
        Self {
            id: extension.id.clone(),
            name: extension.name.clone(),
            author: extension.author.handle.clone(),
            author_name: extension.author.name.clone(),
            author_avatar: extension.author.avatar.clone(),
            title: extension.title.clone(),
            description: extension.description.clone(),
            icon_light: extension.icons.light.clone(),
            icon_dark: extension.icons.dark.clone(),
            downloads: crate::raycast_store_view::format_count(extension.download_count),
            version: extension.version_key(),
            updated,
            categories: extension.categories.clone(),
            platforms: extension.platforms.clone().unwrap_or_default(),
            commands: extension
                .commands
                .iter()
                .map(|command| ListingCommand {
                    title: command.title.clone(),
                    description: command.description.clone(),
                })
                .collect(),
            contributors: extension
                .contributors
                .iter()
                .map(|user| user.name.clone())
                .filter(|name| !name.is_empty())
                .collect(),
            readme_url: non_empty(&extension.readme_url),
            source_url: non_empty(&extension.source_url),
            store_url: non_empty(&extension.store_url),
            download_url: extension.download_url.clone(),
            screenshots: extension.screenshots(),
        }
    }
}

/// Where to fetch a README's Markdown from.
///
/// Both stores link a README's page on GitHub
/// (`github.com/<owner>/<repo>/tree/<ref>/<path>`), which is HTML. The same
/// file's raw text is at `raw.githubusercontent.com/<owner>/<repo>/<ref>/<path>`.
/// Any other URL is fetched as it is.
#[must_use]
pub fn readme_source_url(url: &str) -> String {
    for prefix in ["https://github.com/", "http://github.com/"] {
        let Some(rest) = url.strip_prefix(prefix) else {
            continue;
        };
        let mut parts = rest.splitn(4, '/');
        if let (Some(owner), Some(repo), Some(kind), Some(path)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
            && matches!(kind, "tree" | "blob")
        {
            return format!("https://raw.githubusercontent.com/{owner}/{repo}/{path}");
        }
    }
    url.to_owned()
}

/// A deeplink into a store extension's detail page:
/// `vicinae://extensions/<author>/<name>`, or its `raycast://` and
/// `com.raycast:` spellings, which open the Raycast store's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLink {
    /// Whether the Raycast store is meant.
    pub raycast: bool,
    /// The author's handle.
    pub author: String,
    /// The extension's name in the store.
    pub name: String,
}

/// The usage sentence the C++ answers a malformed extensions link with.
pub const EXTENSION_LINK_USAGE: &str = "Usage: vicinae://extensions/<author>/<extension-name>";

/// Reads an extensions deeplink, as `IpcCommandHandler` does for the
/// `extensions` command: two path segments, percent-decoded. `None` for a
/// URL that is not an extensions link at all; `Some(Err(..))` for one with
/// the wrong number of segments.
#[must_use]
pub fn parse_extension_link(url: &str) -> Option<Result<ExtensionLink, &'static str>> {
    let (scheme, rest) = url.split_once(':')?;
    let raycast = match scheme {
        "vicinae" => false,
        "raycast" | "com.raycast" => true,
        _ => return None,
    };
    let rest = rest.trim_start_matches('/');
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let path = rest.strip_prefix("extensions")?;
    if !(path.is_empty() || path.starts_with('/')) {
        return None;
    }
    let segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            percent_encoding::percent_decode_str(segment)
                .decode_utf8_lossy()
                .into_owned()
        })
        .collect();
    let [author, name] = segments.as_slice() else {
        return Some(Err(EXTENSION_LINK_USAGE));
    };
    Some(Ok(ExtensionLink {
        raycast,
        author: author.clone(),
        name: name.clone(),
    }))
}

/// The largest README fetched, in bytes.
pub const MAX_README_BYTES: u64 = 512 * 1024;

/// The detail page's Markdown: `StoreDetailView`'s header, metadata,
/// compatibility banner and command list, then the README when one was
/// fetched.
#[must_use]
pub fn detail_markdown(listing: &Listing, alert: Option<&Alert>, readme: Option<&str>) -> String {
    let mut out = format!("# {}\n\n", listing.title);
    if !listing.description.trim().is_empty() {
        out.push_str(&format!("{}\n\n", listing.description.trim()));
    }

    if let Some(alert) = alert {
        out.push_str(&format!("> **{}**\n", alert.message));
        for note in &alert.notes {
            out.push_str(&format!(">\n> - {note}\n"));
        }
        out.push('\n');
    }

    let mut facts = Vec::new();
    if !listing.author_name.is_empty() {
        facts.push(format!("**Author** {}", listing.author_name));
    }
    facts.push(format!("**Downloads** {}", listing.downloads));
    if let Some(updated) = &listing.updated {
        facts.push(format!("**Updated** {updated}"));
    }
    if !listing.categories.is_empty() {
        facts.push(format!("**Categories** {}", listing.categories.join(", ")));
    }
    if !listing.platforms.is_empty() {
        facts.push(format!("**Platforms** {}", listing.platforms.join(", ")));
    }
    out.push_str(&facts.join(" · "));
    out.push_str("\n\n");

    if !listing.commands.is_empty() {
        out.push_str(&format!("## Commands ({})\n\n", listing.commands.len()));
        for command in &listing.commands {
            if command.description.trim().is_empty() {
                out.push_str(&format!("- **{}**\n", command.title));
            } else {
                out.push_str(&format!(
                    "- **{}** — {}\n",
                    command.title,
                    command.description.trim()
                ));
            }
        }
        out.push('\n');
    }

    if !listing.contributors.is_empty() {
        out.push_str(&format!(
            "## Contributors\n\n{}\n\n",
            listing.contributors.join(", ")
        ));
    }

    if let Some(readme) = readme.map(str::trim).filter(|text| !text.is_empty()) {
        out.push_str("---\n\n");
        out.push_str(readme);
        out.push('\n');
    }
    out
}

/// The compatibility banner for a Raycast extension, on a platform that has
/// a compatibility sheet; `None` elsewhere, and for the Vicinae store.
#[must_use]
pub fn raycast_alert(
    compat: &crate::raycast_store::CompatMap,
    name: &str,
    platform: &str,
) -> Option<Alert> {
    crate::raycast_store::has_compat_sheet(platform)
        .then(|| crate::raycast_store_view::build_alert(compat.get(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extensions_link_names_the_store_author_and_extension() {
        assert_eq!(
            parse_extension_link("vicinae://extensions/zoë/clock"),
            Some(Ok(ExtensionLink {
                raycast: false,
                author: "zoë".into(),
                name: "clock".into()
            }))
        );
        assert_eq!(
            parse_extension_link("raycast://extensions/thomas/spotify-player?x=1"),
            Some(Ok(ExtensionLink {
                raycast: true,
                author: "thomas".into(),
                name: "spotify-player".into()
            }))
        );
        assert_eq!(
            parse_extension_link("com.raycast:/extensions/a%20b/c")
                .and_then(Result::ok)
                .map(|link| link.author),
            Some("a b".into())
        );
        assert_eq!(
            parse_extension_link("vicinae://extensions/x"),
            Some(Err(EXTENSION_LINK_USAGE))
        );
        assert_eq!(parse_extension_link("raycast://oauth?code=c"), None);
        assert_eq!(parse_extension_link("vicinae://extensionsfoo/a/b"), None);
        assert_eq!(parse_extension_link("https://extensions/a/b"), None);
    }

    #[test]
    fn a_github_readme_page_is_fetched_as_raw_text() {
        assert_eq!(
            readme_source_url(
                "https://github.com/vicinaehq/extensions/tree/main/extensions/bluetooth/README.md"
            ),
            "https://raw.githubusercontent.com/vicinaehq/extensions/main/extensions/bluetooth/README.md"
        );
        assert_eq!(
            readme_source_url(
                "https://github.com/raycast/extensions/blob/8490f28/extensions/x/README.md"
            ),
            "https://raw.githubusercontent.com/raycast/extensions/8490f28/extensions/x/README.md"
        );
        assert_eq!(
            readme_source_url("http://127.0.0.1:9/readme.md"),
            "http://127.0.0.1:9/readme.md"
        );
        assert_eq!(
            readme_source_url("https://github.com/owner"),
            "https://github.com/owner"
        );
    }

    #[test]
    fn the_detail_names_everything_the_qml_view_shows() {
        let listing = Listing {
            title: "Clock".into(),
            description: "Shows the time".into(),
            author_name: "Zoë".into(),
            downloads: "1.1K".into(),
            updated: Some("2026-07-02".into()),
            categories: vec!["System".into()],
            platforms: vec!["linux".into()],
            commands: vec![
                ListingCommand {
                    title: "Show".into(),
                    description: "Show it".into(),
                },
                ListingCommand {
                    title: "Hide".into(),
                    description: String::new(),
                },
            ],
            ..Listing::default()
        };
        let alert = crate::raycast_store_view::build_alert(None);
        let text = detail_markdown(&listing, Some(&alert), Some("## Usage\n\nPress it."));
        assert!(text.starts_with("# Clock\n\nShows the time\n\n> **No compatibility data"));
        assert!(text.contains("**Author** Zoë · **Downloads** 1.1K · **Updated** 2026-07-02"));
        assert!(text.contains("## Commands (2)\n\n- **Show** — Show it\n- **Hide**\n"));
        assert!(text.ends_with("---\n\n## Usage\n\nPress it.\n"));
        assert!(!detail_markdown(&listing, None, Some("  ")).contains("---"));
    }

    #[test]
    fn the_banner_exists_only_where_there_is_a_sheet() {
        let compat = crate::raycast_store::CompatMap::new();
        assert!(raycast_alert(&compat, "x", "linux").is_some());
        assert!(raycast_alert(&compat, "x", "macos").is_none());
    }

    #[test]
    fn a_raycast_listing_derives_its_screenshots_and_id() {
        let mut extension = crate::raycast_store::Extension {
            name: "spotify-player".into(),
            metadata_count: 2,
            readme_assets_path: "https://example.com/assets/".into(),
            download_count: 479_906,
            ..crate::raycast_store::Extension::default()
        };
        crate::raycast_store::post_process_extension(&mut extension);
        let listing = Listing::from_raycast(&extension, None);
        assert_eq!(listing.id, "store.raycast.spotify-player");
        assert_eq!(listing.downloads, "480K");
        assert_eq!(
            listing.screenshots,
            [
                "https://example.com/assets/metadata/spotify-player-1.png",
                "https://example.com/assets/metadata/spotify-player-2.png"
            ]
        );
        assert_eq!(Store::Raycast.extension_id("x"), "store.raycast.x");
        assert_eq!(Store::Vicinae.extension_id("x"), "store.vicinae.x");
    }
}
