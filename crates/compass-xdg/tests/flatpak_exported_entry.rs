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

/// The export as flatpak really lays it out: a symlink, not a file.
///
/// `flatpak-dir.c`'s `export_dir` builds the link target from
/// `symlink_prefix = ../app/<id>/current/active/export` (line 9123 at the time
/// of writing), so what lands in `exports/share/applications` is
///
///   com.google.Chrome.desktop -> ../../../app/com.google.Chrome/current/active/export/share/applications/com.google.Chrome.desktop
///
/// with `current` and `active` symlinks of their own. The entry a launcher
/// reads therefore lives in the **deploy** tree; the exports directory holds
/// only pointers into it.
///
/// That is the shape of #105. A sandbox granted the exports directory and not
/// the deploy tree sees a directory full of links that resolve to nothing --
/// and the failure is total and silent, because every Flatpak application on
/// the machine is exported exactly this way.
fn flatpak_install(root: &std::path::Path, app_id: &str, contents: &str) -> std::path::PathBuf {
    let deploy = root
        .join("app")
        .join(app_id)
        .join("current")
        .join("active")
        .join("export")
        .join("share")
        .join("applications");
    std::fs::create_dir_all(&deploy).expect("deploy tree");
    std::fs::write(deploy.join(format!("{app_id}.desktop")), contents).expect("write entry");

    let exports = root.join("exports").join("share").join("applications");
    std::fs::create_dir_all(&exports).expect("exports tree");
    std::os::unix::fs::symlink(
        std::path::Path::new("../../../app")
            .join(app_id)
            .join("current/active/export/share/applications")
            .join(format!("{app_id}.desktop")),
        exports.join(format!("{app_id}.desktop")),
    )
    .expect("export symlink");
    exports
}

#[test]
fn an_export_symlink_into_the_deploy_tree_is_indexed() {
    let root = tempfile::tempdir().expect("tempdir");
    let exports = flatpak_install(root.path(), "com.google.Chrome", CHROME);

    let scan = scan_desktop_files(&exports);
    assert_eq!(scan.files.len(), 1, "{:?}", scan.errors);
    assert_eq!(scan.files[0].id(), "com.google.Chrome.desktop");
    // Read through the link, because finding the name in the scan is not the
    // same as being able to open it.
    let read = std::fs::read_to_string(scan.files[0].path()).expect("the entry is readable");
    assert!(read.contains("Name=Google Chrome"));
}

#[test]
fn an_export_whose_deploy_tree_is_unreachable_is_the_reported_symptom() {
    let root = tempfile::tempdir().expect("tempdir");
    let exports = flatpak_install(root.path(), "com.google.Chrome", CHROME);

    // Exactly what our sandbox does to it: the exports directory is granted
    // and the deploy tree the links point into is not. Removing it here is the
    // same observation a bind mount makes -- the target is not there.
    std::fs::remove_dir_all(root.path().join("app")).expect("drop the deploy tree");

    let scan = scan_desktop_files(&exports);
    // The scan still *names* it: a dangling symlink is a directory entry like
    // any other, so the failure does not show up as an empty scan or an error.
    // It shows up one step later, when something tries to read it -- which is
    // why this was reported as "not in search" rather than as any kind of
    // diagnostic.
    assert_eq!(scan.files.len(), 1);
    assert!(
        std::fs::read_to_string(scan.files[0].path()).is_err(),
        "the entry should be unreadable, which is the bug"
    );
}
