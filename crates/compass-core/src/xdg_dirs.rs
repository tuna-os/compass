//! The handful of XDG base-directory lookups the core needs.
//!
//! Everything here reads process environment variables. Nothing else in this crate does, so a
//! test that avoids these functions cannot accidentally touch the invoking user's home directory.

use std::path::{Path, PathBuf};

/// `/usr/local/share:/usr/share`, the specified fallback for an unset `$XDG_DATA_DIRS`.
pub const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The application directories, in precedence order.
///
/// DELEGATES rather than reimplementing, and that is the fix for a class of bug rather than a
/// style preference. This function used to be a character-for-character copy of
/// [`compass_xdg::application_dirs`], with `compass-xdg` owning the matching `icon_dirs`. So the
/// applications the launcher indexes and the icons it draws for them were resolved by two
/// separate copies of the same logic, and #95 -- a Flatpak sandbox exposing host data under
/// paths neither copy knew about -- had to be fixed in both or the launcher would have found
/// applications and then drawn none of their icons.
#[must_use]
pub fn application_dirs() -> Vec<PathBuf> {
    compass_xdg::application_dirs()
}

/// The extra data roots a Flatpak sandbox needs, with its inputs supplied explicitly.
///
/// Re-exported rather than having `vicinae` depend on `compass-xdg` directly: `compass-core` is
/// already the seam that crate sits behind, and `vicinae doctor` needs this to report the
/// directories the index really searches.
#[must_use]
pub fn sandbox_data_roots_for(in_flatpak: bool, home: Option<&Path>) -> Vec<PathBuf> {
    compass_xdg::sandbox_data_roots_for(in_flatpak, home)
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
#[must_use]
pub fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::data_dir(),
    }
}

/// `$XDG_STATE_HOME/vicinae`, falling back to `~/.local/state/vicinae`: the
/// C++'s `Omnicast::stateDir()`, where the log, the view memory and the
/// onboarding record live.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home_dir()?.join(".local/state"),
    };
    Some(state.join("vicinae"))
}

/// The home directory, or nothing when even the fallback cannot say.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    compass_xdg::home_dir()
}

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    compass_xdg::config_home()
}

/// `$XDG_CACHE_HOME`, falling back to `~/.cache`.
#[must_use]
pub fn cache_home() -> Option<PathBuf> {
    compass_xdg::cache_home()
}

/// `$XDG_DATA_DIRS`, falling back to [`DEFAULT_DATA_DIRS`].
#[must_use]
pub fn data_dirs() -> Vec<PathBuf> {
    let raw = match std::env::var("XDG_DATA_DIRS") {
        Ok(value) if !value.is_empty() => value,
        _ => DEFAULT_DATA_DIRS.to_owned(),
    };

    raw.split(':')
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The desktop names `OnlyShowIn`/`NotShowIn` are matched against, e.g. `["GNOME"]`.
///
/// Reads `$XDG_CURRENT_DESKTOP`, and falls back to `$XDG_SESSION_DESKTOP` and then
/// `$DESKTOP_SESSION` when it is unset. See [`desktops_from`] for why.
#[must_use]
pub fn current_desktops() -> Vec<String> {
    let read = |name: &str| std::env::var(name).ok();
    desktops_from(
        read("XDG_CURRENT_DESKTOP").as_deref(),
        read("XDG_SESSION_DESKTOP").as_deref(),
        read("DESKTOP_SESSION").as_deref(),
    )
}

/// The desktop names, from the three variables that carry them.
///
/// # Why there is a fallback at all
///
/// `$XDG_CURRENT_DESKTOP` is the specified source and the only one the C++ engine reads
/// (`xdgpp::currentDesktop`). Inside our Flatpak it can be unset -- `doctor` reports it that way
/// on the VM tier -- and an unset value is not neutral: an entry marked `OnlyShowIn=GNOME` is
/// then hidden **on GNOME**, which is the opposite of what the key asks for (#97). `NotShowIn`
/// fails the same way in the other direction, admitting entries meant to be excluded.
///
/// So when the specified variable says nothing, two conventional ones are consulted. Both are set
/// by systemd/logind and the display manager, and neither is a guess about what the desktop *is*
/// -- they are the session's own record of what it launched.
///
/// # Why an inferred name is emitted in two casings
///
/// The specification compares these names **case-sensitively**, and `matches_desktop` does
/// exactly that, byte for byte as the C++ does. But `$XDG_SESSION_DESKTOP` is conventionally
/// lowercase (`gnome`) while entries are written against the registered name (`GNOME`), so a
/// fallback that passed the raw value through would find nothing and quietly change no behaviour
/// at all.
///
/// Rather than loosen the comparison -- which would diverge from the C++ everywhere, for the sake
/// of a case that only arises here -- an INFERRED name is contributed in both its own casing and
/// ASCII uppercase. `$XDG_CURRENT_DESKTOP` is authoritative and is passed through untouched: when
/// the session states its identity there is nothing to guess at, and uppercasing e.g.
/// `X-Cinnamon` would break a name that was already correct.
#[must_use]
pub fn desktops_from(
    current: Option<&str>,
    session: Option<&str>,
    desktop_session: Option<&str>,
) -> Vec<String> {
    let split = |raw: &str| -> Vec<String> {
        raw.split(':')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    };

    // The authoritative source, verbatim.
    let authoritative = current.map(split).unwrap_or_default();
    if !authoritative.is_empty() {
        return authoritative;
    }

    // Inferred, in both casings. `first()` rather than a merge of the two: they name the same
    // session, and a session that set only the second is not also running the first.
    let inferred = [session, desktop_session]
        .into_iter()
        .flatten()
        .map(split)
        .find(|names| !names.is_empty())
        .unwrap_or_default();

    let mut out = Vec::with_capacity(inferred.len() * 2);
    for name in inferred {
        let upper = name.to_ascii_uppercase();
        if upper != name {
            out.push(upper);
        }
        out.push(name);
    }
    out
}

/// The directories in `$PATH`.
#[must_use]
pub fn exec_search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::desktops_from;

    #[test]
    fn the_specified_variable_wins_and_is_passed_through_verbatim() {
        assert_eq!(
            desktops_from(Some("ubuntu:GNOME"), Some("gnome"), Some("gnome")),
            ["ubuntu", "GNOME"]
        );
        // Not uppercased: a session that states its identity is not guessing, and
        // `X-Cinnamon` is already the registered spelling.
        assert_eq!(
            desktops_from(Some("X-Cinnamon"), None, None),
            ["X-Cinnamon"]
        );
    }

    #[test]
    fn an_unset_specified_variable_falls_back_to_the_session() {
        // This is #97: inside the Flatpak the first is unset, and without a fallback an
        // `OnlyShowIn=GNOME` entry is hidden on GNOME.
        let names = desktops_from(None, Some("gnome"), None);
        assert!(names.contains(&"GNOME".to_owned()), "{names:?}");
        assert!(names.contains(&"gnome".to_owned()), "{names:?}");
    }

    #[test]
    fn an_inferred_name_is_offered_in_both_casings() {
        // `matches_desktop` compares case-sensitively, byte for byte as the C++ does.
        // `XDG_SESSION_DESKTOP` is conventionally lowercase and entries are written against the
        // registered name, so passing the raw value alone would match nothing and the fallback
        // would change no behaviour whatsoever.
        assert_eq!(desktops_from(None, Some("gnome"), None), ["GNOME", "gnome"]);
        // Already uppercase: contributed once, not twice.
        assert_eq!(desktops_from(None, Some("GNOME"), None), ["GNOME"]);
    }

    #[test]
    fn desktop_session_is_the_last_resort() {
        assert_eq!(
            desktops_from(None, None, Some("plasma")),
            ["PLASMA", "plasma"]
        );
    }

    #[test]
    fn the_first_source_that_says_anything_wins_outright() {
        // Not a merge. The two variables name the same session, so a session that set only the
        // second is not also running the first, and concatenating them would claim both.
        assert_eq!(
            desktops_from(None, Some("gnome"), Some("plasma")),
            ["GNOME", "gnome"]
        );
    }

    #[test]
    fn empty_and_whitespace_values_are_not_a_desktop() {
        assert!(desktops_from(Some(""), None, None).is_empty());
        assert!(desktops_from(Some("  "), None, None).is_empty());
        // An empty authoritative value falls through rather than winning with nothing.
        assert_eq!(
            desktops_from(Some(""), Some("gnome"), None),
            ["GNOME", "gnome"]
        );
        assert!(desktops_from(None, None, None).is_empty());
    }

    #[test]
    fn separators_and_padding_are_handled_like_the_specified_form() {
        assert_eq!(desktops_from(Some("a::b"), None, None), ["a", "b"]);
        assert_eq!(
            desktops_from(Some(" GNOME : Unity "), None, None),
            ["GNOME", "Unity"]
        );
    }
}
