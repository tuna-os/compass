//! `recently-used.xbel`: the desktop's shared list of recent files.
//!
//! Ports `xdgpp::BookmarkFile` (`src/lib/xdgpp/xdgpp/bookmark/`),
//! `xdgpp::toFileUri`/`fromFileUri` (`src/lib/xdgpp/xdgpp/uri/file-uri.cpp`),
//! and `listRecent` and `recordAccess`
//! (`src/server/src/services/files-service/linux/xbel-recent-files-provider.cpp`).
//!
//! # Writing
//!
//! [`record_access`] is `recordAccess`: read the file, add `compass` as an
//! application that opened the item (creating the bookmark, or bumping its
//! count), set its MIME type, and write the whole document back through a
//! temporary file with owner-only permissions, renamed into place, as GTK
//! and the C++ do. As in the C++, what is written is what the model holds:
//! a bookmark element's fields, its metadata and its applications; an
//! element another writer added that the model does not know is not kept.
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

/// The application name Compass records itself under (the C++ records
/// `vicinae`).
pub const APP_NAME: &str = "compass";

/// The `exec` recorded with it.
pub const APP_EXEC: &str = "'compass %u'";

/// A path as a `file://` URI: `toFileUri`'s escaping, which keeps the RFC
/// 3986 path characters and escapes every other byte as uppercase hex.
#[must_use]
pub fn to_file_uri(path: &Path) -> String {
    const KEEP: &[u8] = b"-._~/!$&'()*+,;=:@";
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `BookmarkFile::find`: the bookmark with `href`, or one whose URI names the
/// same path spelled differently.
fn find<'a>(bookmarks: &'a mut [Bookmark], href: &str) -> Option<&'a mut Bookmark> {
    if let Some(position) = bookmarks.iter().position(|b| b.href == href) {
        return bookmarks.get_mut(position);
    }
    let path = from_file_uri(href)?;
    bookmarks
        .iter_mut()
        .find(|b| from_file_uri(&b.href).as_ref() == Some(&path))
}

/// `BookmarkFile::addApplication`: `name` opened `href` at `timestamp`.
pub fn add_application(
    bookmarks: &mut Vec<Bookmark>,
    href: &str,
    name: &str,
    exec: &str,
    timestamp: &str,
) {
    if find(bookmarks, href).is_none() {
        bookmarks.push(Bookmark {
            href: href.to_owned(),
            added: timestamp.to_owned(),
            ..Bookmark::default()
        });
    }
    let Some(bookmark) = find(bookmarks, href) else {
        return;
    };
    bookmark.modified = timestamp.to_owned();
    bookmark.visited = timestamp.to_owned();
    match bookmark
        .applications
        .iter_mut()
        .find(|app| app.name == name)
    {
        Some(app) => {
            exec.clone_into(&mut app.exec);
            timestamp.clone_into(&mut app.modified);
            app.count += 1;
        }
        None => bookmark.applications.push(BookmarkApplication {
            name: name.to_owned(),
            exec: exec.to_owned(),
            modified: timestamp.to_owned(),
            count: 1,
        }),
    }
}

/// `BookmarkFile::setMimeType`.
pub fn set_mime_type(bookmarks: &mut [Bookmark], href: &str, mime: &str) {
    if let Some(bookmark) = find(bookmarks, href) {
        bookmark.mime_type = Some(mime.to_owned());
    }
}

/// `BookmarkFile::toString`: the document, two-space indented.
#[must_use]
pub fn serialize(bookmarks: &[Bookmark]) -> String {
    use xmlwriter::{Options, XmlWriter};
    let mut xml = XmlWriter::new(Options::default());
    xml.write_declaration();
    xml.start_element("xbel");
    xml.write_attribute("version", "1.0");
    xml.write_attribute("xmlns:bookmark", BOOKMARK_NS);
    xml.write_attribute("xmlns:mime", MIME_NS);
    for bookmark in bookmarks {
        xml.start_element("bookmark");
        xml.write_attribute("href", &bookmark.href);
        xml.write_attribute("added", &bookmark.added);
        xml.write_attribute("modified", &bookmark.modified);
        xml.write_attribute("visited", &bookmark.visited);
        for (name, value) in [("title", &bookmark.title), ("desc", &bookmark.description)] {
            if !value.is_empty() {
                xml.start_element(name);
                xml.write_text(value);
                xml.end_element();
            }
        }
        xml.start_element("info");
        xml.start_element("metadata");
        xml.write_attribute("owner", "http://freedesktop.org");
        if let Some(mime) = &bookmark.mime_type {
            xml.start_element("mime:mime-type");
            xml.write_attribute("type", mime);
            xml.end_element();
        }
        if !bookmark.groups.is_empty() {
            xml.start_element("bookmark:groups");
            for group in &bookmark.groups {
                xml.start_element("bookmark:group");
                xml.write_text(group);
                xml.end_element();
            }
            xml.end_element();
        }
        if !bookmark.applications.is_empty() {
            xml.start_element("bookmark:applications");
            for app in &bookmark.applications {
                xml.start_element("bookmark:application");
                xml.write_attribute("name", &app.name);
                xml.write_attribute("exec", &app.exec);
                xml.write_attribute("modified", &app.modified);
                xml.write_attribute("count", &app.count);
                xml.end_element();
            }
            xml.end_element();
        }
        if bookmark.is_private {
            xml.start_element("bookmark:private");
            xml.end_element();
        }
        xml.end_element();
        xml.end_element();
        xml.end_element();
    }
    xml.end_document()
}

/// Why an access could not be recorded.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// The existing file did not parse; it is left alone.
    #[error(transparent)]
    Parse(#[from] Error),
    /// Reading or writing failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `recordAccess`: records that Compass opened `file` (of type `mime`) at
/// `timestamp` (ISO 8601 UTC) in the XBEL file at `xbel`.
///
/// # Errors
///
/// [`RecordError::Parse`] when the existing file is not XBEL (it is then not
/// touched), [`RecordError::Io`] when it cannot be read or written.
pub fn record_access(
    xbel: &Path,
    file: &Path,
    mime: &str,
    timestamp: &str,
) -> Result<(), RecordError> {
    let mut bookmarks = match std::fs::read_to_string(xbel) {
        Ok(text) => parse(&text)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let href = to_file_uri(file);
    add_application(&mut bookmarks, &href, APP_NAME, APP_EXEC, timestamp);
    set_mime_type(&mut bookmarks, &href, mime);
    save(xbel, &serialize(&bookmarks))?;
    Ok(())
}

/// `BookmarkFile::save`: a temporary file beside it, owner-only, renamed into
/// place.
fn save(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
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

    #[test]
    fn recording_an_access_adds_then_bumps_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let xbel = dir.path().join("share/recently-used.xbel");
        let file = std::path::Path::new("/home/a/My Notes/résumé.pdf");
        record_access(
            &xbel,
            file,
            "application/pdf",
            "2026-09-24T10:00:00.000001Z",
        )
        .unwrap();
        let text = std::fs::read_to_string(&xbel).unwrap();
        let bookmarks = parse(&text).unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(
            bookmarks[0].href,
            "file:///home/a/My%20Notes/r%C3%A9sum%C3%A9.pdf"
        );
        assert_eq!(bookmarks[0].path().as_deref(), Some(file));
        assert_eq!(bookmarks[0].mime_type.as_deref(), Some("application/pdf"));
        assert_eq!(bookmarks[0].applications[0].name, APP_NAME);
        assert_eq!(bookmarks[0].applications[0].count, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&xbel).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        std::fs::write(&xbel, SAMPLE).unwrap();
        record_access(
            &xbel,
            std::path::Path::new("/tmp/test.mp4"),
            "video/mp4",
            "2027-01-01T00:00:00.000000Z",
        )
        .unwrap();
        let bookmarks = parse(&std::fs::read_to_string(&xbel).unwrap()).unwrap();
        let sample = parse(SAMPLE).unwrap();
        assert_eq!(
            bookmarks.len(),
            sample.len(),
            "an existing bookmark is reused"
        );
        let video = bookmarks
            .iter()
            .find(|b| b.href == "file:///tmp/test.mp4")
            .unwrap();
        assert_eq!(video.modified, "2027-01-01T00:00:00.000000Z");
        assert_eq!(video.added, sample[0].added, "added is kept");
        assert!(video.applications.iter().any(|a| a.name == APP_NAME));

        std::fs::write(&xbel, "not xml <").unwrap();
        assert!(matches!(
            record_access(&xbel, file, "x/y", "t"),
            Err(RecordError::Parse(_))
        ));
        assert_eq!(std::fs::read_to_string(&xbel).unwrap(), "not xml <");
    }

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
