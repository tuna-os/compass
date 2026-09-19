//! An `OnlyShowIn=GNOME` entry, indexed on a session that says so three different ways.
//!
//! #97: inside the Flatpak `$XDG_CURRENT_DESKTOP` can be unset, and an unset value is not
//! neutral -- `OnlyShowIn=GNOME` then hides the entry **on GNOME**, which is the opposite of
//! what the key asks for. The count it costs on the first target is small (one application),
//! but the direction is wrong and the number differs per image.
//!
//! These drive `AppIndex` rather than the matcher, because the defect was never in the matcher:
//! it compares correctly against whatever it is given, and what it was given was nothing.

use compass_core::apps::AppIndex;
use compass_core::xdg_dirs::desktops_from;

const GNOME_ONLY: &str = "\
[Desktop Entry]
Type=Application
Name=Gnome Only Thing
Exec=/bin/true
OnlyShowIn=GNOME;
";

const EVERYWHERE: &str = "\
[Desktop Entry]
Type=Application
Name=Everywhere Thing
Exec=/bin/true
";

const NOT_GNOME: &str = "\
[Desktop Entry]
Type=Application
Name=Not On Gnome
Exec=/bin/true
NotShowIn=GNOME;
";

fn corpus(dir: &std::path::Path) {
    for (file, body) in [
        ("gnome-only.desktop", GNOME_ONLY),
        ("everywhere.desktop", EVERYWHERE),
        ("not-gnome.desktop", NOT_GNOME),
    ] {
        std::fs::write(dir.join(file), body).expect("write entry");
    }
}

fn names(dir: &std::path::Path, desktops: Vec<String>) -> Vec<String> {
    let index = AppIndex::builder().dir(dir).desktops(desktops).build();
    let mut found: Vec<String> = index
        .items()
        .iter()
        .map(|item| item.name().to_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn a_gnome_only_entry_is_indexed_when_the_session_is_named_the_specified_way() {
    let dir = tempfile::tempdir().expect("tempdir");
    corpus(dir.path());
    assert_eq!(
        names(dir.path(), desktops_from(Some("GNOME"), None, None)),
        ["Everywhere Thing", "Gnome Only Thing"]
    );
}

#[test]
fn and_when_only_the_session_variable_names_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    corpus(dir.path());
    // XDG_CURRENT_DESKTOP unset, XDG_SESSION_DESKTOP=gnome -- the shape #97 reports inside the
    // Flatpak. Before the fallback this returned only "Everywhere Thing".
    assert_eq!(
        names(dir.path(), desktops_from(None, Some("gnome"), None)),
        ["Everywhere Thing", "Gnome Only Thing"]
    );
}

#[test]
fn and_when_only_the_legacy_variable_names_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    corpus(dir.path());
    assert_eq!(
        names(dir.path(), desktops_from(None, None, Some("gnome"))),
        ["Everywhere Thing", "Gnome Only Thing"]
    );
}

#[test]
fn not_show_in_is_honoured_by_the_same_fallback() {
    let dir = tempfile::tempdir().expect("tempdir");
    corpus(dir.path());
    // The other direction of the same defect: with no desktop identified, an entry that asks
    // NOT to be shown on GNOME is shown anyway.
    let inferred = names(dir.path(), desktops_from(None, Some("gnome"), None));
    assert!(
        !inferred.contains(&"Not On Gnome".to_owned()),
        "{inferred:?}"
    );

    let blind = names(dir.path(), desktops_from(None, None, None));
    assert!(
        blind.contains(&"Not On Gnome".to_owned()),
        "with no desktop identified it is admitted, which is what the fallback exists to fix: \
         {blind:?}"
    );
}

#[test]
fn an_unidentifiable_desktop_still_behaves_as_the_cpp_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    corpus(dir.path());
    // Deliberately unchanged. `DesktopEntry::matches_desktop` excludes OnlyShowIn entries when
    // nothing is known, byte for byte as `DesktopEntry::matchesCurrentDesktop` does in the C++
    // (src/lib/xdgpp/xdgpp/desktop-entry/entry.cpp). The fix here is that "nothing is known" is
    // now rare, not that the rule for it changed -- loosening the rule would be a divergence in
    // every environment for the sake of one.
    assert_eq!(
        names(dir.path(), desktops_from(None, None, None)),
        ["Everywhere Thing", "Not On Gnome"]
    );
}
