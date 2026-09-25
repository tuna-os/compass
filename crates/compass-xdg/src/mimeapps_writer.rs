//! Making an application the default for some MIME types, in `mimeapps.list`.
//!
//! Ports `xdgpp::setDefaultApplication`
//! (`src/lib/xdgpp/xdgpp/mime/mime-apps-list-writer.cpp`), which Set Default
//! Browser uses. It does what `xdg-mime default` does: the application becomes
//! the `[Default Applications]` entry for each type *and* goes to the front of
//! its `[Added Associations]` list, and every other line of the file — comments,
//! other types, `[Removed Associations]` — is left exactly as it was.
//!
//! The file is edited as lines rather than parsed and re-serialised, because a
//! user's `mimeapps.list` is hand-edited as often as it is generated, and a
//! round trip through a parser would drop their comments and reorder their
//! groups. A key written twice in one group keeps its first position and loses
//! the duplicates, since a reader would otherwise disagree with itself about
//! which line wins.

use std::path::Path;

const DEFAULT_APPLICATIONS_GROUP: &str = "Default Applications";
const ADDED_ASSOCIATIONS_GROUP: &str = "Added Associations";

fn is_group_header(line: &str) -> bool {
    let entry = line.trim();
    entry.starts_with('[') && entry.ends_with(']')
}

fn key_of(line: &str) -> &str {
    let entry = line.trim();
    entry.split_once('=').map_or(entry, |(key, _)| key).trim()
}

fn value_of(line: &str) -> &str {
    line.trim()
        .split_once('=')
        .map_or("", |(_, value)| value.trim())
}

/// The body of `name` as a `[begin, end)` line range, appending an empty group
/// when the file has none. Blank lines before the next group are not part of
/// the body, so an insertion lands above them.
fn find_group(lines: &mut Vec<String>, name: &str) -> (usize, usize) {
    let header = format!("[{name}]");
    let Some(position) = lines.iter().position(|line| line.trim() == header) else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        return (lines.len(), lines.len());
    };
    let begin = position + 1;
    let mut end = begin;
    while end < lines.len() && !is_group_header(&lines[end]) {
        end += 1;
    }
    while end > begin && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    (begin, end)
}

fn set_key(
    lines: &mut Vec<String>,
    group: &str,
    key: &str,
    value: impl Fn(Option<&str>) -> String,
) {
    let (begin, end) = find_group(lines, group);
    let Some(first) = (begin..end).find(|&index| key_of(&lines[index]) == key) else {
        lines.insert(end, format!("{key}={}", value(None)));
        return;
    };
    let replaced = format!("{key}={}", value(Some(value_of(&lines[first]))));
    lines[first] = replaced;
    let mut index = first + 1;
    let mut end = end;
    while index < end {
        if key_of(&lines[index]) == key {
            lines.remove(index);
            end -= 1;
        } else {
            index += 1;
        }
    }
}

fn prepend_to_list(current: Option<&str>, app_id: &str) -> String {
    let mut ids: Vec<&str> = current
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    if !ids.contains(&app_id) {
        ids.insert(0, app_id);
    }
    ids.iter().map(|id| format!("{id};")).collect()
}

/// `existing` with `app_id` made the default for every type in `mimes`.
#[must_use]
pub fn with_default_application(existing: &str, mimes: &[&str], app_id: &str) -> String {
    let mut lines: Vec<String> = existing.lines().map(str::to_owned).collect();
    for mime in mimes {
        set_key(&mut lines, DEFAULT_APPLICATIONS_GROUP, mime, |_| {
            app_id.to_owned()
        });
        set_key(&mut lines, ADDED_ASSOCIATIONS_GROUP, mime, |current| {
            prepend_to_list(current, app_id)
        });
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Makes `app_id` the default for `mimes` in the `mimeapps.list` at `path`,
/// creating it (and its directory) when missing. The new contents go to a
/// temporary file beside it first and are renamed into place, so a failure
/// part-way leaves the old file whole.
///
/// # Errors
///
/// Whatever reading, writing or renaming the file reports.
pub fn set_default_application(path: &Path, mimes: &[&str], app_id: &str) -> std::io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    std::fs::write(
        &temporary,
        with_default_application(&existing, mimes, app_id),
    )?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mimeapps::Associations;

    // The three cases are `src/lib/xdgpp/tests/mime.cpp`'s, inputs and
    // expected files verbatim.

    #[test]
    fn a_missing_file_is_created_with_the_default_application() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("config/mimeapps.list");
        set_default_application(
            &path,
            &["x-scheme-handler/https", "text/html"],
            "firefox.desktop",
        )
        .expect("written");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "[Default Applications]\n\
             x-scheme-handler/https=firefox.desktop\n\
             text/html=firefox.desktop\n\
             \n\
             [Added Associations]\n\
             x-scheme-handler/https=firefox.desktop;\n\
             text/html=firefox.desktop;\n"
        );
    }

    #[test]
    fn an_existing_file_is_updated_and_everything_else_kept() {
        let existing = "# user config\n\
                        [Default Applications]\n\
                        text/html=chromium.desktop\n\
                        image/png=swayimg.desktop\n\
                        \n\
                        [Added Associations]\n\
                        text/html=chromium.desktop;firefox.desktop;\n\
                        image/png=swayimg.desktop;\n\
                        \n\
                        [Removed Associations]\n\
                        image/png=gimp.desktop;\n";
        let written = with_default_application(
            existing,
            &["text/html", "x-scheme-handler/https"],
            "firefox.desktop",
        );
        assert_eq!(
            written,
            "# user config\n\
             [Default Applications]\n\
             text/html=firefox.desktop\n\
             image/png=swayimg.desktop\n\
             x-scheme-handler/https=firefox.desktop\n\
             \n\
             [Added Associations]\n\
             text/html=chromium.desktop;firefox.desktop;\n\
             image/png=swayimg.desktop;\n\
             x-scheme-handler/https=firefox.desktop;\n\
             \n\
             [Removed Associations]\n\
             image/png=gimp.desktop;\n"
        );
        let read = Associations::parse(&written);
        assert_eq!(read.default_for("text/html"), ["firefox.desktop"]);
        assert_eq!(
            read.added_for("x-scheme-handler/https"),
            ["firefox.desktop"]
        );
    }

    #[test]
    fn duplicate_keys_are_dropped_and_the_first_position_kept() {
        let existing = "[Default Applications]\n\
                        text/html=chromium.desktop\n\
                        image/png=swayimg.desktop\n\
                        text/html=helium.desktop\n\
                        \n\
                        [Added Associations]\n\
                        text/html=chromium.desktop;\n\
                        text/html=helium.desktop\n";
        assert_eq!(
            with_default_application(existing, &["text/html"], "firefox.desktop"),
            "[Default Applications]\n\
             text/html=firefox.desktop\n\
             image/png=swayimg.desktop\n\
             \n\
             [Added Associations]\n\
             text/html=firefox.desktop;chromium.desktop;\n"
        );
    }

    #[test]
    fn an_application_already_associated_is_not_listed_twice() {
        let written = with_default_application(
            "[Added Associations]\ntext/html=a.desktop;firefox.desktop;\n",
            &["text/html"],
            "firefox.desktop",
        );
        assert!(
            written.contains("text/html=a.desktop;firefox.desktop;\n"),
            "{written}"
        );
    }
}
