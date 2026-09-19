//! A Flatpak-exported desktop entry, as one really ships.
//!
//! Issue #105 reports that Flatpak applications do not appear in search.
//! These pin the two things about such an entry that could plausibly have
//! dropped it before it ever reached the index, so that if the cause turns out
//! to be elsewhere, nobody re-investigates this ground.

use compass_xdg::scan::scan_desktop_files;

/// `com.google.Chrome`'s exported entry, copied from a real export.
///
/// The `Exec` line is the part worth having: Flatpak rewrites it to a
/// `flatpak run` invocation and wraps the URI placeholder in its own
/// `@@u ... @@` file-forwarding markers. A parser that treated `@@` as
/// malformed, or that required `Exec` to be a bare program, would drop every
/// Flatpak application on the machine and nothing else -- which is exactly the
/// shape of the reported symptom.
const CHROME: &str = "\
[Desktop Entry]
Version=1.0
Name=Google Chrome
GenericName=Web Browser
Comment=Access the Internet
Exec=/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=/app/bin/chrome \
--file-forwarding com.google.Chrome @@u %U @@
StartupNotify=true
Terminal=false
Icon=com.google.Chrome
Type=Application
Categories=Network;WebBrowser;
MimeType=application/pdf;text/html;
StartupWMClass=Google-chrome
X-Flatpak=com.google.Chrome
";

#[test]
fn a_flatpak_exported_entry_scans_parses_and_shows() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("com.google.Chrome.desktop"), CHROME).expect("write");

    let scan = scan_desktop_files(dir.path());
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.files.len(), 1);

    let parsed = scan.files[0].parse().expect("a Flatpak export must parse");
    let entry = parsed.entry();

    assert_eq!(entry.name(), "Google Chrome");
    assert!(!entry.no_display(), "nothing hides it");
    assert!(!entry.hidden());
    assert!(
        entry.exec().is_some_and(|exec| exec.contains("@@u")),
        "the file-forwarding markers survive into Exec rather than being rejected"
    );
}

#[test]
fn it_shows_with_no_desktop_names_at_all() {
    // Inside our own Flatpak `XDG_CURRENT_DESKTOP` is unset (#97), so the list
    // of desktop names the entry is matched against is empty. An entry with no
    // `OnlyShowIn` must survive that -- if it did not, every application would
    // vanish inside the sandbox rather than just the GNOME-only ones.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("com.google.Chrome.desktop"), CHROME).expect("write");

    let scan = scan_desktop_files(dir.path());
    let parsed = scan.files[0].parse().expect("parses");

    assert!(parsed.entry().should_show(Vec::<String>::new()));
    assert!(parsed.entry().should_show(["GNOME"]));
}
