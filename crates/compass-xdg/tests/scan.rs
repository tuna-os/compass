use std::path::Path;

use compass_xdg::{Locale, ParseOptions, desktop_file_id, scan_desktop_files};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn desktop_ids_flatten_subdirectories() {
    let root = Path::new("/usr/share/applications");
    assert_eq!(
        desktop_file_id(root, Path::new("/usr/share/applications/konsole.desktop")).as_deref(),
        Some("konsole.desktop"),
    );
    assert_eq!(
        desktop_file_id(
            root,
            Path::new("/usr/share/applications/kde4/konsole.desktop")
        )
        .as_deref(),
        Some("kde4-konsole.desktop"),
    );
    assert_eq!(
        desktop_file_id(root, Path::new("/elsewhere/konsole.desktop")),
        None,
    );
}

#[test]
fn scanning_is_recursive_sorted_and_limited_to_desktop_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "z.desktop",
        "[Desktop Entry]\nName=Zed\nExec=zed\n",
    );
    write(
        dir.path(),
        "kde4/a.desktop",
        "[Desktop Entry]\nName=Alpha\nExec=alpha\n",
    );
    write(dir.path(), "ignored.txt", "not a desktop entry");

    let scan = scan_desktop_files(dir.path());
    let ids: Vec<_> = scan.files.iter().map(|file| file.id()).collect();

    assert_eq!(ids, ["kde4-a.desktop", "z.desktop"]);
    assert!(scan.errors.is_empty());
}

#[test]
fn a_scanned_entry_has_a_non_optional_id() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "nested/example.desktop",
        "[Desktop Entry]\nName=Example\nExec=example\n",
    );

    let scan = scan_desktop_files(dir.path());
    let entry = scan.files[0]
        .parse_with(&ParseOptions {
            locale: Some(Locale::parse("C")),
            ..Default::default()
        })
        .unwrap();

    assert_eq!(entry.id(), "nested-example.desktop");
    assert_eq!(entry.name(), "Example");
    assert_eq!(entry.path(), Some(scan.files[0].path()));
}

#[test]
fn a_missing_action_name_remains_missing() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "action.desktop",
        "[Desktop Entry]\nName=Example\nExec=example\nActions=broken;\n\n[Desktop Action broken]\nExec=example --broken\n",
    );

    let scan = scan_desktop_files(dir.path());
    let entry = scan.files[0].parse().unwrap();

    assert_eq!(entry.actions()[0].name(), None);
}
