//! `mimeapps.list`: which application opens which MIME type.
//!
//! Ports `xdgpp::MimeAppsList` (`src/lib/xdgpp/xdgpp/mime/`) and the resolution
//! `XdgAppDatabase::defaultForMime` / `findAssociations` performs over the
//! stack of those files
//! (`src/server/src/services/app-service/xdg/xdg-app-database.cpp`).
//!
//! # The order of the files is the whole answer
//!
//! A `mimeapps.list` is a desktop-entry file with three groups — `Default
//! Applications`, `Added Associations`, `Removed Associations` — and the same
//! MIME type appears in several of them across the system. Which application
//! opens a file is decided by *which file said so first*, so
//! [`search_paths_for`] is not a detail: it is the algorithm. It reproduces
//! `getMimeLikeConfigPaths("mimeapps.list")` exactly, including the
//! `$XDG_CURRENT_DESKTOP`-prefixed variants, which are how a desktop
//! environment overrides a user's default for itself.
//!
//! # What this does not do yet
//!
//! **No MIME-type inheritance.** `findAssociations` walks parent types
//! (`text/x-python` → `text/plain` → `application/octet-stream`) out of the
//! shared-mime-info database, which nothing in Rust reads yet. A lookup here
//! answers for the type it was given and no other, which is right for an exact
//! match and incomplete for a subtype. The gap is stated rather than papered
//! over, because a resolver that silently missed the parent would look like one
//! that had simply found no opener.
//!
//! **No target-to-MIME detection.** `mimeNameForTarget` turns a path or a URL
//! into a MIME type using the same database. A caller here supplies the type.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::locale::Locale;
use crate::reader::Reader;

/// The three groups of one `mimeapps.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Associations {
    /// `[Default Applications]`: the preferred openers, in order.
    pub default: Vec<(String, Vec<String>)>,
    /// `[Added Associations]`: further openers, in order.
    pub added: Vec<(String, Vec<String>)>,
    /// `[Removed Associations]`: openers that must not be offered.
    pub removed: Vec<(String, Vec<String>)>,
}

fn group_entries(reader: &Reader, name: &str) -> Vec<(String, Vec<String>)> {
    let Some(group) = reader.group(name) else {
        return Vec::new();
    };
    group
        .keys()
        .map(|key| (key.to_owned(), group.string_list(key)))
        .collect()
}

impl Associations {
    /// Parses one file's contents.
    #[must_use]
    pub fn parse(data: &str) -> Self {
        // The C locale on purpose: these keys are MIME types and the values are
        // desktop file ids. Neither is localized, and a `[Default
        // Applications]` group with a `text/plain[de]` key is not a thing.
        let reader = Reader::parse(data, Locale::default());
        Self {
            default: group_entries(&reader, "Default Applications"),
            added: group_entries(&reader, "Added Associations"),
            removed: group_entries(&reader, "Removed Associations"),
        }
    }

    /// Reads one file, or `None` if it is not there.
    ///
    /// A missing file is not an error: most of the paths in
    /// [`search_paths_for`] do not exist on any given machine.
    #[must_use]
    pub fn from_file(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .map(|data| Self::parse(&data))
    }

    fn lookup(entries: &[(String, Vec<String>)], mime: &str) -> Vec<String> {
        entries
            .iter()
            .find(|(key, _)| key == mime)
            .map(|(_, apps)| apps.clone())
            .unwrap_or_default()
    }

    /// The `Default Applications` entry for `mime`.
    #[must_use]
    pub fn default_for(&self, mime: &str) -> Vec<String> {
        Self::lookup(&self.default, mime)
    }

    /// The `Added Associations` entry for `mime`.
    #[must_use]
    pub fn added_for(&self, mime: &str) -> Vec<String> {
        Self::lookup(&self.added, mime)
    }

    /// The `Removed Associations` entry for `mime`.
    #[must_use]
    pub fn removed_for(&self, mime: &str) -> Vec<String> {
        Self::lookup(&self.removed, mime)
    }
}

/// Where `mimeapps.list` files are looked for, in the order they are consulted.
///
/// Reproduces `getMimeLikeConfigPaths`: for each of `$XDG_CONFIG_HOME`, each
/// `$XDG_CONFIG_DIRS`, `$XDG_DATA_HOME/applications` and each
/// `$XDG_DATA_DIRS/applications`, the desktop-prefixed names first (lower-cased
/// `$XDG_CURRENT_DESKTOP` entries, e.g. `gnome-mimeapps.list`) and then the
/// bare name.
///
/// Takes its inputs rather than reading the environment so that it can be
/// tested; [`search_paths`] is the version that reads it.
#[must_use]
pub fn search_paths_for(
    config_home: Option<&Path>,
    config_dirs: &[PathBuf],
    data_home: Option<&Path>,
    data_dirs: &[PathBuf],
    desktops: &[String],
) -> Vec<PathBuf> {
    const FILE: &str = "mimeapps.list";
    let prefixed: Vec<String> = desktops
        .iter()
        .map(|desktop| format!("{}-{FILE}", desktop.to_lowercase()))
        .collect();

    let mut paths = Vec::new();
    let mut push_all = |dir: &Path| {
        for name in &prefixed {
            paths.push(dir.join(name));
        }
        paths.push(dir.join(FILE));
    };

    if let Some(home) = config_home {
        push_all(home);
    }
    for dir in config_dirs {
        push_all(dir);
    }
    if let Some(home) = data_home {
        push_all(&home.join("applications"));
    }
    for dir in data_dirs {
        push_all(&dir.join("applications"));
    }
    paths
}

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::config_dir(),
    }
}

/// `$XDG_CONFIG_DIRS`, falling back to `/etc/xdg`.
#[must_use]
pub fn config_dirs() -> Vec<PathBuf> {
    let raw = match std::env::var("XDG_CONFIG_DIRS") {
        Ok(value) if !value.is_empty() => value,
        _ => "/etc/xdg".to_owned(),
    };
    raw.split(':')
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// [`search_paths_for`], from this process's environment.
#[must_use]
pub fn search_paths() -> Vec<PathBuf> {
    search_paths_for(
        config_home().as_deref(),
        &config_dirs(),
        crate::xdg_dirs::data_home().as_deref(),
        &crate::xdg_dirs::data_dirs(),
        &crate::xdg_dirs::current_desktops(),
    )
}

/// The MIME type a target names, as `mimeNameForTarget` computes it.
///
/// Three steps, in the C++'s order:
///
/// 1. a URL with a scheme is `x-scheme-handler/<scheme>` — this is how
///    `https://…` reaches a browser, and it is the case that needs no
///    database at all;
/// 2. a string that is already a MIME type is itself;
/// 3. anything else is looked up as a file.
///
/// # Where this diverges, and it is visible
///
/// Step 2 asks `QMimeDatabase::mimeTypeForName(...).isValid()`, which means
/// *registered on this machine*; here it is the syntactic shape `type/subtype`
/// under a known top-level type. A made-up `text/bar` is therefore taken as a
/// MIME type here and as a filename by the C++ — and both then find no
/// opener, because nothing claims it either way.
///
/// Step 3 classifies a file by its extension through `mime_guess`, and a
/// directory as `inode/directory`; shared-mime-info's content sniffing is not
/// read, so an extensionless file is `application/octet-stream`, which is
/// what `QMimeDatabase` answers for a file it cannot sniff either.
#[must_use]
pub fn target_mime(target: &str) -> String {
    if let Some(scheme) = url_scheme(target) {
        return format!("x-scheme-handler/{scheme}");
    }
    if is_mime_name(target) {
        return target.to_owned();
    }
    file_mime(std::path::Path::new(target))
}

/// The MIME type of a file: `inode/directory` for a directory, else what its
/// extension says, else `application/octet-stream`.
#[must_use]
pub fn file_mime(path: &std::path::Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_owned();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned()
}

/// The top-level media types a MIME name can start with: the IANA registry's,
/// plus the freedesktop pseudo-types openers are registered for.
const MIME_TOP_LEVEL: &[&str] = &[
    "application",
    "audio",
    "chemical",
    "font",
    "image",
    "inode",
    "message",
    "model",
    "multipart",
    "text",
    "video",
    "x-content",
    "x-scheme-handler",
];

/// Whether `text` has the shape of a MIME name, `type/subtype`, with a known
/// top-level type. The C++ asks whether the name is registered on this
/// machine; the shape with a known top level is the closest a check can come
/// without shared-mime-info, and it keeps a relative path such as
/// `notes/today.md` a path.
fn is_mime_name(text: &str) -> bool {
    let Some((top, sub)) = text.split_once('/') else {
        return false;
    };
    MIME_TOP_LEVEL.contains(&top)
        && !sub.is_empty()
        && sub
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$&^_.+-".contains(c))
}

/// The scheme of `target`, if it has one.
///
/// RFC 3986: `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`. A bare path
/// has none, and neither does `text/plain` — there is no colon.
fn url_scheme(target: &str) -> Option<&str> {
    let colon = target.find(':')?;
    let scheme = &target[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    chars
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        .then_some(scheme)
}

/// The `mimeapps.list` files that exist, in consultation order.
#[derive(Debug, Clone, Default)]
pub struct Lists {
    files: Vec<(PathBuf, Associations)>,
}

impl Lists {
    /// Reads every file in `paths` that exists.
    #[must_use]
    pub fn load(paths: &[PathBuf]) -> Self {
        Self {
            files: paths
                .iter()
                .filter_map(|path| Associations::from_file(path).map(|list| (path.clone(), list)))
                .collect(),
        }
    }

    /// From this process's environment.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::load(&search_paths())
    }

    /// The files that were found, in order.
    #[must_use]
    pub fn files(&self) -> &[(PathBuf, Associations)] {
        &self.files
    }

    /// The preferred opener for `mime`, as `defaultForMime` chooses it.
    ///
    /// `usable` says whether a desktop file id names an application this
    /// machine can actually run; the C++ asks `appMap` plus `isExecutable`, and
    /// this crate has no application index, so the caller supplies the answer.
    ///
    /// Falls back to the first of [`Lists::openers_for`], which is what the C++ does
    /// when no `Default Applications` entry names something runnable.
    #[must_use]
    pub fn default_for(&self, mime: &str, usable: &impl Fn(&str) -> bool) -> Option<String> {
        for (_, list) in &self.files {
            for id in list.default_for(mime) {
                if usable(&id) {
                    return Some(id);
                }
            }
        }
        self.openers_for(mime, usable).into_iter().next()
    }

    /// The preferred opener for whatever `target` names.
    ///
    /// `findDefaultOpener` is exactly `defaultForMime(mimeNameForTarget(t))`.
    #[must_use]
    pub fn default_for_target(
        &self,
        target: &str,
        usable: &impl Fn(&str) -> bool,
    ) -> Option<String> {
        self.default_for(&target_mime(target), usable)
    }

    /// Every application offered for `mime`, best first.
    ///
    /// The C++ order, reproduced: every file's `Default Applications` entry
    /// first (one per file, the first runnable one), then a second pass over
    /// the files taking `Added Associations` — with anything a *previous* file
    /// removed left out, and a file's own `Removed Associations` applying to
    /// the files after it.
    ///
    /// Associations that come from a desktop file's own `MimeType=` are not
    /// here: they need the application index, and the caller that has one adds
    /// them in the same pass.
    #[must_use]
    pub fn openers_for(&self, mime: &str, usable: &impl Fn(&str) -> bool) -> Vec<String> {
        let mut openers = Vec::new();
        let mut seen = BTreeSet::new();

        for (_, list) in &self.files {
            for id in list.default_for(mime) {
                if usable(&id) {
                    if seen.insert(id.clone()) {
                        openers.push(id);
                    }
                    break;
                }
            }
        }

        let mut removed = BTreeSet::new();
        for (_, list) in &self.files {
            for id in list.added_for(mime) {
                if removed.contains(&id) || seen.contains(&id) {
                    continue;
                }
                seen.insert(id.clone());
                if usable(&id) {
                    openers.push(id);
                }
            }
            for id in list.removed_for(mime) {
                removed.insert(id);
            }
        }

        openers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_groups_are_read() {
        let list = Associations::parse(
            "[Default Applications]\n\
             text/plain=gedit.desktop;kate.desktop;\n\
             [Added Associations]\n\
             text/plain=vim.desktop;\n\
             [Removed Associations]\n\
             text/plain=emacs.desktop;\n",
        );
        assert_eq!(
            list.default_for("text/plain"),
            vec!["gedit.desktop".to_owned(), "kate.desktop".to_owned()]
        );
        assert_eq!(list.added_for("text/plain"), vec!["vim.desktop".to_owned()]);
        assert_eq!(
            list.removed_for("text/plain"),
            vec!["emacs.desktop".to_owned()]
        );
        assert!(list.default_for("image/png").is_empty());
    }

    #[test]
    fn a_group_that_is_not_there_is_empty_rather_than_an_error() {
        let list = Associations::parse("[Default Applications]\ntext/plain=gedit.desktop;\n");
        assert!(list.added_for("text/plain").is_empty());
        assert!(list.removed_for("text/plain").is_empty());
    }

    #[test]
    fn the_search_order_is_the_one_the_cpp_builds() {
        // `getMimeLikeConfigPaths`: config home, config dirs, data home's
        // applications/, then each data dir's applications/ -- and within each,
        // the desktop-prefixed names before the bare one.
        let paths = search_paths_for(
            Some(Path::new("/home/u/.config")),
            &[PathBuf::from("/etc/xdg")],
            Some(Path::new("/home/u/.local/share")),
            &[PathBuf::from("/usr/share")],
            &["GNOME".to_owned()],
        );

        let expected: Vec<PathBuf> = [
            "/home/u/.config/gnome-mimeapps.list",
            "/home/u/.config/mimeapps.list",
            "/etc/xdg/gnome-mimeapps.list",
            "/etc/xdg/mimeapps.list",
            "/home/u/.local/share/applications/gnome-mimeapps.list",
            "/home/u/.local/share/applications/mimeapps.list",
            "/usr/share/applications/gnome-mimeapps.list",
            "/usr/share/applications/mimeapps.list",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn the_desktop_prefix_is_lower_cased_and_every_desktop_gets_one() {
        // `$XDG_CURRENT_DESKTOP` is `GNOME` or `ubuntu:GNOME`; the file names
        // are lower case. A port that kept the case would look in
        // `GNOME-mimeapps.list` and find nothing, silently.
        let paths = search_paths_for(
            Some(Path::new("/c")),
            &[],
            None,
            &[],
            &["ubuntu".to_owned(), "GNOME".to_owned()],
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/c/ubuntu-mimeapps.list"),
                PathBuf::from("/c/gnome-mimeapps.list"),
                PathBuf::from("/c/mimeapps.list"),
            ]
        );
    }

    /// A stack of lists, without touching the filesystem.
    fn lists(files: &[&str]) -> Lists {
        Lists {
            files: files
                .iter()
                .enumerate()
                .map(|(index, data)| {
                    (
                        PathBuf::from(format!("/{index}")),
                        Associations::parse(data),
                    )
                })
                .collect(),
        }
    }

    fn anything() -> impl Fn(&str) -> bool {
        |_| true
    }

    #[test]
    fn the_first_file_that_names_a_usable_default_wins() {
        let stack = lists(&[
            "[Default Applications]\ntext/plain=first.desktop;\n",
            "[Default Applications]\ntext/plain=second.desktop;\n",
        ]);
        assert_eq!(
            stack.default_for("text/plain", &anything()),
            Some("first.desktop".to_owned())
        );
    }

    #[test]
    fn a_default_that_is_not_installed_is_passed_over() {
        // `appMap.find(appId) != appMap.end() && isExecutable(...)`. A default
        // naming an application that was uninstalled must not win, or the
        // machine has no opener for the type at all.
        let stack = lists(&["[Default Applications]\ntext/plain=gone.desktop;here.desktop;\n"]);
        let installed = |id: &str| id == "here.desktop";
        assert_eq!(
            stack.default_for("text/plain", &installed),
            Some("here.desktop".to_owned())
        );
    }

    #[test]
    fn with_no_default_the_first_added_association_is_used() {
        let stack = lists(&["[Added Associations]\ntext/plain=vim.desktop;nano.desktop;\n"]);
        assert_eq!(
            stack.default_for("text/plain", &anything()),
            Some("vim.desktop".to_owned())
        );
    }

    #[test]
    fn a_removal_hides_an_application_a_later_file_adds() {
        // The rule that makes the order matter: a user's own file removing an
        // association must beat a system file adding it.
        let stack = lists(&[
            "[Removed Associations]\ntext/plain=vim.desktop;\n",
            "[Added Associations]\ntext/plain=vim.desktop;nano.desktop;\n",
        ]);
        assert_eq!(
            stack.openers_for("text/plain", &anything()),
            vec!["nano.desktop".to_owned()],
            "a removed association was offered anyway"
        );
    }

    #[test]
    fn a_removal_does_not_reach_backwards() {
        // `removed` is filled as the loop walks the files, so a removal in a
        // later file does not undo an addition an earlier one made. That is
        // the C++'s behaviour, and it is what makes the user's file win.
        let stack = lists(&[
            "[Added Associations]\ntext/plain=vim.desktop;\n",
            "[Removed Associations]\ntext/plain=vim.desktop;\n",
        ]);
        assert_eq!(
            stack.openers_for("text/plain", &anything()),
            vec!["vim.desktop".to_owned()]
        );
    }

    #[test]
    fn each_application_is_offered_once() {
        let stack = lists(&[
            "[Default Applications]\ntext/plain=gedit.desktop;\n",
            "[Added Associations]\ntext/plain=gedit.desktop;vim.desktop;\n",
        ]);
        assert_eq!(
            stack.openers_for("text/plain", &anything()),
            vec!["gedit.desktop".to_owned(), "vim.desktop".to_owned()]
        );
    }

    #[test]
    fn nothing_is_offered_for_a_type_nobody_claims() {
        let stack = lists(&["[Default Applications]\ntext/plain=gedit.desktop;\n"]);
        assert!(stack.openers_for("image/png", &anything()).is_empty());
        assert_eq!(stack.default_for("image/png", &anything()), None);
    }

    #[test]
    fn a_url_resolves_to_a_scheme_handler() {
        // The case that needs no MIME database, and the one that matters most:
        // it is how a link reaches a browser.
        assert_eq!(target_mime("https://example.com"), "x-scheme-handler/https");
        assert_eq!(target_mime("mailto:a@b.c"), "x-scheme-handler/mailto");
        assert_eq!(
            target_mime("vicinae+deep://x"),
            "x-scheme-handler/vicinae+deep"
        );
    }

    #[test]
    fn a_path_and_a_mime_type_are_left_alone() {
        // Neither has a scheme. A MIME type is already the answer; a path is
        // classified by its extension, or as a directory.
        assert_eq!(target_mime("/home/u/notes.txt"), "text/plain");
        assert_eq!(target_mime("/home/u/report.pdf"), "application/pdf");
        assert_eq!(target_mime("text/plain"), "text/plain");
        assert_eq!(
            target_mime("application/vnd.oasis.opendocument.text"),
            "application/vnd.oasis.opendocument.text"
        );
        assert_eq!(target_mime("notes.txt"), "text/plain");
        assert_eq!(target_mime("notes/today.md"), "text/markdown");
        assert_eq!(target_mime("/home/u/Makefile"), "application/octet-stream");
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            target_mime(&dir.path().to_string_lossy()),
            "inode/directory"
        );
    }

    #[test]
    fn something_that_only_looks_like_a_scheme_is_not_one() {
        // RFC 3986 says a scheme starts with a letter. A Windows-style path or
        // a time of day must not become `x-scheme-handler/c`.
        assert_eq!(target_mime("12:30"), "application/octet-stream");
        assert_eq!(target_mime(":/x"), "application/octet-stream");
        assert_eq!(
            target_mime("a b:c"),
            "application/octet-stream",
            "a space is not scheme syntax"
        );
    }

    #[test]
    fn a_target_resolves_through_the_same_stack_as_a_mime_type() {
        let stack = lists(&["[Default Applications]\nx-scheme-handler/https=firefox.desktop;\n"]);
        assert_eq!(
            stack.default_for_target("https://example.com", &anything()),
            Some("firefox.desktop".to_owned())
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_skipped_rather_than_failing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let present = dir.path().join("mimeapps.list");
        std::fs::write(&present, "[Default Applications]\ntext/plain=a.desktop;\n").expect("write");

        let stack = Lists::load(&[dir.path().join("missing.list"), present]);
        assert_eq!(stack.files().len(), 1);
        assert_eq!(
            stack.default_for("text/plain", &anything()),
            Some("a.desktop".to_owned())
        );
    }
}
