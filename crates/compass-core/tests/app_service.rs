//! Application lookups, read against
//! `src/server/src/services/app-service/app-service.cpp` and its XDG provider
//! (`.../xdg/xdg-app-database.cpp`).

mod support;

use compass_core::app_service::AppService;
use support::{builder, write};

/// An entry with a `StartupWMClass` and an optional desktop action.
fn entry(name: &str, wm_class: Option<&str>, action: Option<&str>) -> String {
    let mut text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n");
    if let Some(class) = wm_class {
        text.push_str(&format!("StartupWMClass={class}\n"));
    }
    if let Some(action) = action {
        text.push_str(&format!("Actions={action};\n"));
        text.push_str(&format!(
            "\n[Desktop Action {action}]\nName={action}\nExec={name} --{action}\n"
        ));
    }
    text
}

#[test]
fn an_id_is_found_with_or_without_the_desktop_suffix() {
    // `findById` tries the id, then `id + ".desktop"`.
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "firefox.desktop", &entry("Firefox", None, None));
    let index = builder().dir(dir.path()).build();
    let apps = AppService::new(&index);

    assert_eq!(
        apps.find_by_id("firefox.desktop").map(|a| a.desktop_id()),
        Some("firefox.desktop")
    );
    assert_eq!(
        apps.find_by_id("firefox").map(|a| a.desktop_id()),
        Some("firefox.desktop"),
        "the suffix is appended on a miss"
    );
    assert!(apps.find_by_id("chromium").is_none());
}

#[test]
fn a_window_class_finds_the_application_that_declares_it() {
    // `findByClass` -> `matchesWindowClass`, which normalises both sides.
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "org.gnome.Nautilus.desktop",
        &entry("Files", Some("org.gnome.Nautilus"), None),
    );
    write(dir.path(), "gimp.desktop", &entry("GIMP", None, None));
    let index = builder().dir(dir.path()).build();
    let apps = AppService::new(&index);

    assert_eq!(
        apps.find_by_class("org.gnome.nautilus").map(|a| a.name()),
        Some("Files"),
        "case-insensitive, via normalizeClass"
    );
    assert_eq!(
        apps.find_by_class("GIMP").map(|a| a.name()),
        Some("GIMP"),
        "an application with no StartupWMClass is matched on its desktop id"
    );
    assert!(apps.find_by_class("konsole").is_none());
}

#[test]
fn find_falls_through_from_the_id_to_the_window_class() {
    // `find` is `findById(target)` then `findByClass(target)`: a target only a
    // window class knows is still found, and one that is an id does not need
    // the second lookup.
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "code.desktop", &entry("VS Code", None, None));
    write(
        dir.path(),
        "org.gnome.Nautilus.desktop",
        &entry("Files", Some("nautilus-alt"), None),
    );
    let index = builder().dir(dir.path()).build();
    let apps = AppService::new(&index);

    assert_eq!(
        apps.find("code").map(|a| a.name()),
        Some("VS Code"),
        "found by id, with the .desktop suffix appended"
    );
    assert!(
        apps.find_by_id("nautilus-alt").is_none(),
        "nothing is called that"
    );
    assert_eq!(
        apps.find("nautilus-alt").map(|a| a.name()),
        Some("Files"),
        "so the class lookup answers instead"
    );
}

#[test]
fn a_lookup_never_answers_with_a_desktop_action() {
    // `m_apps` holds applications; an action lives inside one and is not
    // separately launchable, so neither lookup may return one.
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "firefox.desktop",
        &entry("Firefox", None, Some("new-window")),
    );
    let index = builder().dir(dir.path()).build();
    assert!(
        index.items().iter().any(|item| item.is_action()),
        "the fixture does index the action"
    );

    let apps = AppService::new(&index);
    assert_eq!(apps.list(false).len(), 1, "one application, not two");
    assert!(apps.list(false).iter().all(|item| !item.is_action()));
    assert!(
        apps.find_by_id("firefox.desktop")
            .is_some_and(|item| !item.is_action())
    );
}

#[test]
fn listing_alphabetically_ignores_case() {
    // `a->displayName().compare(b->displayName(), Qt::CaseInsensitive) < 0`.
    let dir = tempfile::tempdir().expect("tempdir");
    for (file, name) in [
        ("zed.desktop", "Zed"),
        ("ark.desktop", "ark"),
        ("blender.desktop", "Blender"),
    ] {
        write(dir.path(), file, &entry(name, None, None));
    }
    let index = builder().dir(dir.path()).build();
    let apps = AppService::new(&index);

    let sorted: Vec<&str> = apps.list(true).iter().map(|a| a.name()).collect();
    assert_eq!(
        sorted,
        ["ark", "Blender", "Zed"],
        "a lower-case 'ark' sorts first, not last"
    );

    let unsorted = apps.list(false);
    assert_eq!(unsorted.len(), 3, "and without the flag nothing is sorted");
}

#[test]
fn curating_openers_keeps_the_first_of_each_display_name() {
    // `findCuratedOpeners` dedupes on `displayName()`, not on id: a system
    // Firefox and a Flatpak Firefox are one choice to the user, and the list
    // arrives in preference order.
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "firefox.desktop", &entry("Firefox", None, None));
    write(
        dir.path(),
        "org.mozilla.firefox.desktop",
        &entry("Firefox", None, None),
    );
    write(dir.path(), "gimp.desktop", &entry("GIMP", None, None));
    let index = builder().dir(dir.path()).build();
    let apps = AppService::new(&index);

    let mut all = apps.list(false);
    all.sort_by_key(|item| item.desktop_id().to_owned());
    let curated = AppService::curated(all.clone());

    let names: Vec<String> = curated.iter().map(|a| a.display_name()).collect();
    assert_eq!(names, ["Firefox", "GIMP"]);
    assert_eq!(
        curated[0].desktop_id(),
        "firefox.desktop",
        "the first one wins"
    );
}
