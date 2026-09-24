//! `recently-used.xbel`: the desktop's shared list of recent files.
//!
//! Ports the reading half of `xdgpp::BookmarkFile`
//! (`src/lib/xdgpp/xdgpp/bookmark/`) and `xdgpp::fromFileUri`
//! (`src/lib/xdgpp/xdgpp/uri/file-uri.cpp`), plus `listRecent`
//! (`src/server/src/services/files-service/linux/xbel-recent-files-provider.cpp`).
//!
//! # Reading only
//!
//! The C++ also *writes* this file — `addApplication`, `save`, an atomic
//! replace with owner-only permissions, the way GTK does. None of that is
//! here. A launcher that shows recent files does not have to register them,
//! and writing a file every desktop application also writes is a change with
//! its own failure modes; it can come with its own commit and its own tests.
//!
//! # `modified` is the timestamp that matters
//!
//! The C++ says why, and it is not obvious: "GTK refreshes `modified` on every
//! use and only sets `visited` when the item is first added, so `modified` is
//! authoritative and the others are fallbacks." Sorting by `visited` would put
//! the list in the order things were *first* opened.
//!
//! Timestamps are ISO 8601 UTC strings and are compared as strings, which the
//! C++ notes orders the same way chronologically. That holds only because they
//! are always UTC with the same shape; a local-time stamp would sort wrongly
//! in both engines.
//!
//! # One divergence: namespace prefixes
//!
//! pugixml matches element names literally, so the C++ looks for
//! `bookmark:private` and would miss a document that binds the same namespace
//! to a different prefix — showing a private bookmark as an ordinary one.
//! `roxmltree` resolves namespaces, so this matches the namespace URI instead.
//! The difference only appears for a document no current desktop writes, and
//! it errs towards hiding something private rather than revealing it.

use std::path::{Path, PathBuf};

/// The namespace the desktop-bookmark metadata lives in.
pub const BOOKMARK_NS: &str = "http://www.freedesktop.org/standards/desktop-bookmarks";
/// The namespace the MIME type lives in.
pub const MIME_NS: &str = "http://www.freedesktop.org/standards/shared-mime-info";
/// The file name under `$XDG_DATA_HOME`.
pub const RECENTLY_USED_FILE_NAME: &str = "recently-used.xbel";

/// An application that has opened a bookmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkApplication {
    /// The application's name.
    pub name: String,
    /// Its `Exec` line, as the file records it.
    pub exec: String,
    /// When it last opened this.
    pub modified: String,
    /// How many times.
    pub count: u32,
}

/// One recent item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bookmark {
    /// The `file://` URI.
    pub href: String,
    /// A title, when the writer gave one.
    pub title: String,
    /// A description.
    pub description: String,
    /// When it was added.
    pub added: String,
    /// When it was last used.
    pub modified: String,
    /// When it was first opened.
    pub visited: String,
    /// The MIME type the writer recorded.
    pub mime_type: Option<String>,
    /// Whether it is marked private.
    pub is_private: bool,
    /// Groups it belongs to.
    pub groups: Vec<String>,
    /// Applications that have opened it.
    pub applications: Vec<BookmarkApplication>,
}

impl Bookmark {
    /// The timestamp to sort by: `modified`, then `visited`, then `added`.
    #[must_use]
    pub fn last_used(&self) -> &str {
        if !self.modified.is_empty() {
            return &self.modified;
        }
        if !self.visited.is_empty() {
            return &self.visited;
        }
        &self.added
    }

    /// The path this bookmark points at, if it points at a local file.
    #[must_use]
    pub fn path(&self) -> Option<PathBuf> {
        from_file_uri(&self.href)
    }
}

/// Why a document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The XML did not parse.
    #[error("the bookmark file is not valid XML: {0}")]
    Xml(#[from] roxmltree::Error),
}

/// Reads every bookmark in an XBEL document.
///
/// # Errors
///
/// [`Error::Xml`] if the document does not parse. A document with no
/// bookmarks is not an error — an empty recent list is an ordinary state.
pub fn parse(xml: &str) -> Result<Vec<Bookmark>, Error> {
    let document = roxmltree::Document::parse(xml)?;
    let mut bookmarks = Vec::new();

    for node in document
        .descendants()
        .filter(|n| n.has_tag_name("bookmark"))
    {
        // `<bookmark:application>` also has the local name `application`, but
        // a `<bookmark>` element is in no namespace; this keeps the two apart.
        if node.tag_name().namespace().is_some() {
            continue;
        }

        let text_of = |name: &str| {
            node.children()
                .find(|child| child.has_tag_name(name))
                .and_then(|child| child.text())
                .unwrap_or_default()
                .to_owned()
        };

        let metadata = node
            .children()
            .find(|child| child.has_tag_name("info"))
            .and_then(|info| info.children().find(|child| child.has_tag_name("metadata")));

        let mut bookmark = Bookmark {
            href: node.attribute("href").unwrap_or_default().to_owned(),
            title: text_of("title"),
            description: text_of("desc"),
            added: node.attribute("added").unwrap_or_default().to_owned(),
            modified: node.attribute("modified").unwrap_or_default().to_owned(),
            visited: node.attribute("visited").unwrap_or_default().to_owned(),
            ..Bookmark::default()
        };

        if let Some(metadata) = metadata {
            bookmark.is_private = metadata
                .children()
                .any(|child| child.has_tag_name((BOOKMARK_NS, "private")));

            bookmark.mime_type = metadata
                .children()
                .find(|child| child.has_tag_name((MIME_NS, "mime-type")))
                .and_then(|node| node.attribute("type"))
                .map(ToOwned::to_owned);

            if let Some(groups) = metadata
                .children()
                .find(|child| child.has_tag_name((BOOKMARK_NS, "groups")))
            {
                bookmark.groups = groups
                    .children()
                    .filter(|child| child.has_tag_name((BOOKMARK_NS, "group")))
                    .filter_map(|child| child.text())
                    .map(ToOwned::to_owned)
                    .collect();
            }

            if let Some(applications) = metadata
                .children()
                .find(|child| child.has_tag_name((BOOKMARK_NS, "applications")))
            {
                bookmark.applications = applications
                    .children()
                    .filter(|child| child.has_tag_name((BOOKMARK_NS, "application")))
                    .map(|child| BookmarkApplication {
                        name: child.attribute("name").unwrap_or_default().to_owned(),
                        exec: child.attribute("exec").unwrap_or_default().to_owned(),
                        modified: child.attribute("modified").unwrap_or_default().to_owned(),
                        count: child
                            .attribute("count")
                            .and_then(|count| count.parse().ok())
                            .unwrap_or(0),
                    })
                    .collect();
            }
        }

        bookmarks.push(bookmark);
    }

    Ok(bookmarks)
}

/// The path a `file://` URI names.
///
/// `None` for another scheme, a host that is neither empty nor `localhost`, a
/// truncated escape, and — as the C++ does — a `%00`, because a NUL cannot be
/// in a path.
#[must_use]
pub fn from_file_uri(uri: &str) -> Option<PathBuf> {
    const SCHEME: &str = "file://";

    let rest = uri.strip_prefix(SCHEME)?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !host.is_empty() && host != "localhost" {
        return None;
    }

    let encoded = &rest[slash..];
    // Strict, unlike `percent_decode_str`: a malformed escape or an encoded
    // NUL is refused rather than passed through.
    let bytes = encoded.as_bytes();
    for (index, _) in encoded.match_indices('%') {
        let hex = bytes.get(index + 1..index + 3)?;
        if !hex.iter().all(u8::is_ascii_hexdigit) || hex == b"00" {
            return None;
        }
    }
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?;
    Some(PathBuf::from(decoded.into_owned()))
}

/// `$XDG_DATA_HOME/recently-used.xbel`.
#[must_use]
pub fn recently_used_path() -> Option<PathBuf> {
    crate::xdg_dirs::data_home().map(|home| home.join(RECENTLY_USED_FILE_NAME))
}

/// The most recently used paths, newest first.
///
/// Reproduces `listRecent`: sort by [`Bookmark::last_used`] descending
/// (stably), then skip private bookmarks and ones whose file is gone, then
/// take `limit`. `exists` answers whether a path is there, so a caller can
/// test this without a filesystem; `keep` is the category filter, and
/// returning `true` keeps everything.
///
/// A `limit` of zero yields nothing, as the C++'s `<= 0` check does.
#[must_use]
pub fn recent_paths(
    bookmarks: &[Bookmark],
    limit: usize,
    exists: &impl Fn(&Path) -> bool,
    keep: &impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    if limit == 0 {
        return Vec::new();
    }

    let mut ordered: Vec<&Bookmark> = bookmarks.iter().collect();
    // Stable, and descending, exactly as `std::ranges::stable_sort` with
    // `std::ranges::greater` does: two items used at the same moment keep the
    // order the file had.
    ordered.sort_by(|a, b| b.last_used().cmp(a.last_used()));

    let mut out = Vec::new();
    for bookmark in ordered {
        if out.len() >= limit {
            break;
        }
        if bookmark.is_private {
            continue;
        }
        let Some(path) = bookmark.path() else {
            continue;
        };
        if !exists(&path) || !keep(&path) {
            continue;
        }
        out.push(path);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample from `src/lib/xdgpp/tests/bookmark.cpp`, verbatim.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
  <bookmark href="file:///tmp/test.mp4" added="2025-11-14T21:04:28.221405Z" modified="2026-08-07T08:42:13.260987Z" visited="2025-11-14T21:04:28.221406Z">
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="video/mp4"/>
        <bookmark:applications>
          <bookmark:application name="xdg-desktop-portal-gtk" exec="&apos;xdg-desktop-portal-gtk %u&apos;" modified="2026-08-07T08:42:13.260986Z" count="24"/>
          <bookmark:application name="org.gnome.Nautilus" exec="&apos;mpv -- %U&apos;" modified="2026-04-02T12:11:18.113646Z" count="1"/>
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>
  <bookmark href="file:///home/user/secret.txt" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z" visited="2026-01-01T00:00:00Z">
    <title>Secret</title>
    <desc>Do not share</desc>
    <info>
      <metadata owner="http://freedesktop.org">
        <bookmark:groups>
          <bookmark:group>Work</bookmark:group>
          <bookmark:group>Notes</bookmark:group>
        </bookmark:groups>
        <bookmark:private/>
      </metadata>
    </info>
  </bookmark>
</xbel>
"#;

    fn sample() -> Vec<Bookmark> {
        parse(SAMPLE).expect("the C++ test's own sample parses")
    }

    #[test]
    fn the_cpp_tests_sample_parses_into_what_it_describes() {
        let bookmarks = sample();
        assert_eq!(bookmarks.len(), 2);

        let video = &bookmarks[0];
        assert_eq!(video.href, "file:///tmp/test.mp4");
        assert_eq!(video.mime_type.as_deref(), Some("video/mp4"));
        assert!(!video.is_private);
        assert_eq!(video.applications.len(), 2);
        assert_eq!(video.applications[0].name, "xdg-desktop-portal-gtk");
        assert_eq!(video.applications[0].count, 24);
        assert_eq!(
            video.applications[0].exec, "'xdg-desktop-portal-gtk %u'",
            "the &apos; entities decode"
        );

        let secret = &bookmarks[1];
        assert_eq!(secret.title, "Secret");
        assert_eq!(secret.description, "Do not share");
        assert!(secret.is_private, "bookmark:private is present");
        assert_eq!(secret.groups, vec!["Work".to_owned(), "Notes".to_owned()]);
    }

    #[test]
    fn modified_is_the_timestamp_that_orders_the_list() {
        // The C++ comment: GTK refreshes `modified` on every use and sets
        // `visited` only when the item is added. Sorting by `visited` would
        // order by when things were *first* opened.
        let bookmark = Bookmark {
            added: "2020".to_owned(),
            visited: "2021".to_owned(),
            modified: "2022".to_owned(),
            ..Bookmark::default()
        };
        assert_eq!(bookmark.last_used(), "2022");

        let no_modified = Bookmark {
            modified: String::new(),
            ..bookmark.clone()
        };
        assert_eq!(no_modified.last_used(), "2021");

        let only_added = Bookmark {
            modified: String::new(),
            visited: String::new(),
            ..bookmark
        };
        assert_eq!(only_added.last_used(), "2020");
    }

    #[test]
    fn a_file_uri_decodes_to_a_path() {
        assert_eq!(
            from_file_uri("file:///tmp/test.mp4"),
            Some(PathBuf::from("/tmp/test.mp4"))
        );
        assert_eq!(
            from_file_uri("file:///home/u/My%20Documents/a%2Bb.txt"),
            Some(PathBuf::from("/home/u/My Documents/a+b.txt"))
        );
        assert_eq!(
            from_file_uri("file://localhost/etc/hosts"),
            Some(PathBuf::from("/etc/hosts")),
            "localhost is the one host that means this machine"
        );
    }

    #[test]
    fn anything_that_is_not_a_local_file_is_refused() {
        assert_eq!(from_file_uri("https://example.com/a"), None);
        assert_eq!(from_file_uri("file://elsewhere/etc/hosts"), None);
        assert_eq!(from_file_uri("file://"), None, "no path at all");
        assert_eq!(from_file_uri("file:///tmp/a%"), None, "a truncated escape");
        assert_eq!(from_file_uri("file:///tmp/a%zz"), None, "not hex");
        assert_eq!(
            from_file_uri("file:///tmp/a%00b"),
            None,
            "a NUL cannot be in a path, and the C++ refuses it too"
        );
    }

    #[test]
    fn the_recent_list_is_newest_first_and_skips_the_private_one() {
        let bookmarks = sample();
        let paths = recent_paths(&bookmarks, 10, &|_| true, &|_| true);
        assert_eq!(
            paths,
            vec![PathBuf::from("/tmp/test.mp4")],
            "the secret is private, and the video is the only other entry"
        );
    }

    #[test]
    fn a_file_that_is_gone_is_skipped_rather_than_listed() {
        // `fs::exists` in the C++. A recent list full of deleted files is
        // worse than a short one.
        let bookmarks = sample();
        assert!(recent_paths(&bookmarks, 10, &|_| false, &|_| true).is_empty());
    }

    #[test]
    fn the_category_filter_applies_after_the_sort() {
        let bookmarks = sample();
        let only_text = recent_paths(&bookmarks, 10, &|_| true, &|path| {
            path.extension().is_some_and(|ext| ext == "txt")
        });
        assert!(
            only_text.is_empty(),
            "the only .txt is private, so filtering cannot resurrect it"
        );
    }

    #[test]
    fn a_zero_limit_yields_nothing() {
        assert!(recent_paths(&sample(), 0, &|_| true, &|_| true).is_empty());
    }

    #[test]
    fn the_newest_comes_first_however_the_file_is_ordered() {
        let older = Bookmark {
            href: "file:///a".to_owned(),
            modified: "2020-01-01T00:00:00Z".to_owned(),
            ..Bookmark::default()
        };
        let newer = Bookmark {
            href: "file:///b".to_owned(),
            modified: "2026-01-01T00:00:00Z".to_owned(),
            ..Bookmark::default()
        };

        let paths = recent_paths(&[older, newer], 10, &|_| true, &|_| true);
        assert_eq!(
            paths,
            vec![PathBuf::from("/b"), PathBuf::from("/a")],
            "the file listed them oldest first"
        );
    }

    #[test]
    fn a_document_with_no_bookmarks_is_not_an_error() {
        assert!(
            parse(r#"<?xml version="1.0"?><xbel version="1.0"/>"#)
                .expect("an empty document is valid")
                .is_empty()
        );
    }

    #[test]
    fn a_malformed_document_is_an_error_rather_than_an_empty_list() {
        // An empty list and an unreadable file are different facts, and a
        // caller may want to say so.
        assert!(parse("<xbel><bookmark></xbel>").is_err());
    }
}
