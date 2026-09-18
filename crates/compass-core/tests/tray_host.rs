//! Which picture another application's tray icon draws, and what its menu says.
//!
//! Read off `TrayItem` and `TrayMenuItem`
//! (`src/server/src/services/tray-host/`).

use compass_core::image_url::ImageUrlType;
use compass_core::tray_host::{
    FALLBACK_ICON_NAME, ICON_EXTENSIONS, IconCandidate, Status, ToggleType, TrayItem, TrayMenuItem,
    best_icon,
};

/// Pretend PNG encoding.
fn encode(bytes: &[u8]) -> String {
    format!("data:image/png;base64,<{}>", bytes.len())
}

/// A candidate file.
fn candidate(path: &str, extension: &str, size: u64) -> IconCandidate {
    IconCandidate {
        path: path.to_owned(),
        extension: extension.to_owned(),
        size,
    }
}

/// An item with a bus name and a path.
fn item() -> TrayItem {
    TrayItem {
        bus_name: ":1.42".to_owned(),
        path: "/StatusNotifierItem".to_owned(),
        ..TrayItem::default()
    }
}

#[test]
fn an_items_key_is_its_bus_name_and_path_together() {
    // One application can export several items on one connection, so the bus
    // name alone would collapse them into one row.
    assert_eq!(item().key(), ":1.42/StatusNotifierItem");

    let mut second = item();
    second.path = "/StatusNotifierItem/2".to_owned();
    assert_ne!(item().key(), second.key());
}

#[test]
fn a_root_menu_path_means_no_menu() {
    // "/" is what an application sends to mean "none" without leaving the
    // field empty, and opening it gets an error.
    let mut with = item();
    with.menu_path = "/MenuBar".to_owned();
    assert!(with.has_menu());

    let mut root = item();
    root.menu_path = "/".to_owned();
    assert!(!root.has_menu());

    assert!(!item().has_menu(), "and an empty path is no menu either");
}

#[test]
fn an_item_with_nothing_usable_still_draws_something() {
    // A blank space in the tray reads as a bug in this program rather than in
    // the application that supplied nothing.
    let icon = item().icon(encode);
    assert_eq!(icon.kind, ImageUrlType::System);
    assert_eq!(icon.name, FALLBACK_ICON_NAME);
    assert_eq!(FALLBACK_ICON_NAME, "application-x-executable");
}

#[test]
fn a_resolved_path_is_preferred_over_everything() {
    // It is the only source certainly the icon this application meant: a theme
    // name can resolve to something else entirely under another theme.
    let mut with_everything = item();
    with_everything.icon_path = "/usr/share/icons/app.png".to_owned();
    with_everything.icon_pixmap = Some(vec![1, 2, 3]);
    with_everything.icon_name = "app".to_owned();

    let icon = with_everything.icon(encode);
    assert_eq!(icon.kind, ImageUrlType::Local);
    assert_eq!(icon.name, "/usr/share/icons/app.png");
}

#[test]
fn raw_pixels_beat_a_theme_name() {
    let mut pixels = item();
    pixels.icon_pixmap = Some(vec![1, 2, 3]);
    pixels.icon_name = "app".to_owned();

    let icon = pixels.icon(encode);
    assert_eq!(icon.kind, ImageUrlType::DataUri);
    assert!(icon.name.contains("image/png"));
}

#[test]
fn an_empty_pixmap_is_not_a_pixmap() {
    // An application that set the field but sent no bytes must fall through,
    // not produce a zero-byte image.
    let mut empty = item();
    empty.icon_pixmap = Some(Vec::new());
    empty.icon_name = "app".to_owned();

    let icon = empty.icon(encode);
    assert_eq!(icon.kind, ImageUrlType::System);
    assert_eq!(icon.name, "app");
}

#[test]
fn a_theme_name_is_used_when_there_is_nothing_else() {
    let mut named = item();
    named.icon_name = "firefox".to_owned();

    let icon = named.icon(encode);
    assert_eq!(icon.kind, ImageUrlType::System);
    assert_eq!(icon.name, "firefox");
}

#[test]
fn an_item_needing_attention_uses_its_attention_icon() {
    let mut attention = item();
    attention.status = Status::NeedsAttention;
    attention.icon_path = "/icons/idle.png".to_owned();
    attention.attention_icon_path = "/icons/alert.png".to_owned();

    assert_eq!(attention.icon(encode).name, "/icons/alert.png");
}

#[test]
fn an_idle_item_never_uses_the_attention_icon() {
    let mut idle = item();
    idle.status = Status::Active;
    idle.icon_path = "/icons/idle.png".to_owned();
    idle.attention_icon_path = "/icons/alert.png".to_owned();

    assert_eq!(idle.icon(encode).name, "/icons/idle.png");
}

#[test]
fn each_attention_field_falls_back_on_its_own() {
    // An application supplying an attention *name* but no attention pixmap
    // still gets its ordinary pixmap. Treating the attention set as all or
    // nothing would blank icons that work today.
    let mut partial = item();
    partial.status = Status::NeedsAttention;
    partial.attention_icon_name = "alert".to_owned();
    partial.icon_pixmap = Some(vec![1, 2, 3]);

    let icon = partial.icon(encode);
    assert_eq!(
        icon.kind,
        ImageUrlType::DataUri,
        "the ordinary pixmap still outranks the attention name"
    );
}

#[test]
fn an_attention_status_with_no_attention_fields_uses_the_ordinary_ones() {
    let mut plain = item();
    plain.status = Status::NeedsAttention;
    plain.icon_name = "firefox".to_owned();

    assert_eq!(plain.icon(encode).name, "firefox");
}

#[test]
fn an_absolute_icon_name_is_already_a_path() {
    let mut absolute = item();
    absolute.icon_name = "/opt/app/icon.png".to_owned();
    absolute.icon_theme_path = "/usr/share/icons".to_owned();

    absolute.resolve_theme_icons(|_, _| panic!("no search is needed for an absolute name"));
    assert_eq!(absolute.icon_path, "/opt/app/icon.png");
}

#[test]
fn an_empty_name_is_not_searched_for() {
    // Most items have no attention icon, and walking a theme directory for a
    // name that is not there is the most expensive nothing this service could
    // do. The C++ guards it on findIconInThemePath's first line.
    let mut nameless = item();
    nameless.icon_theme_path = "/icons".to_owned();
    nameless.resolve_theme_icons(|_, _| panic!("an empty name must not be searched for"));
    assert_eq!(nameless.icon_path, "");
    assert_eq!(nameless.attention_icon_path, "");
}

#[test]
fn a_name_with_no_theme_path_is_not_searched_for_either() {
    let mut homeless = item();
    homeless.icon_name = "app".to_owned();
    homeless.resolve_theme_icons(|_, _| panic!("there is nowhere to search"));
    assert_eq!(homeless.icon_path, "");
}

#[test]
fn a_relative_icon_name_is_searched_for_in_the_nominated_directory() {
    let mut relative = item();
    relative.icon_name = "app".to_owned();
    relative.icon_theme_path = "/opt/app/icons".to_owned();

    relative.resolve_theme_icons(|directory, name| format!("{directory}/{name}.png"));
    assert_eq!(relative.icon_path, "/opt/app/icons/app.png");
}

#[test]
fn both_icon_names_are_resolved() {
    let mut both = item();
    both.icon_name = "idle".to_owned();
    both.attention_icon_name = "alert".to_owned();
    both.icon_theme_path = "/icons".to_owned();

    both.resolve_theme_icons(|directory, name| format!("{directory}/{name}.svg"));
    assert_eq!(both.icon_path, "/icons/idle.svg");
    assert_eq!(both.attention_icon_path, "/icons/alert.svg");
}

#[test]
fn an_svg_wins_immediately() {
    // Resolution-independent, so there is nothing to compare it against and
    // the search can stop.
    let found = vec![
        candidate("/icons/big.png", "png", 100_000),
        candidate("/icons/app.svg", "svg", 900),
        candidate("/icons/bigger.png", "png", 200_000),
    ];
    assert_eq!(best_icon(&found).expect("one").path, "/icons/app.svg");
}

#[test]
fn among_bitmaps_the_biggest_file_wins() {
    // File size as a proxy for resolution: crude, and right often enough,
    // since a 256px PNG is almost always a larger file than a 22px one.
    let found = vec![
        candidate("/icons/22.png", "png", 900),
        candidate("/icons/256.png", "png", 40_000),
        candidate("/icons/48.png", "png", 3_000),
    ];
    assert_eq!(best_icon(&found).expect("one").path, "/icons/256.png");
}

#[test]
fn two_files_of_equal_size_resolve_to_the_first() {
    // Strictly greater, so the answer does not depend on the order the
    // directory happened to be read in.
    let found = vec![
        candidate("/icons/a.png", "png", 5_000),
        candidate("/icons/b.png", "png", 5_000),
    ];
    assert_eq!(best_icon(&found).expect("one").path, "/icons/a.png");
}

#[test]
fn only_the_three_known_extensions_are_considered() {
    assert_eq!(ICON_EXTENSIONS, &["svg", "png", "xpm"]);
    let found = vec![
        candidate("/icons/app.ico", "ico", 100_000),
        candidate("/icons/app.xpm", "xpm", 500),
    ];
    assert_eq!(best_icon(&found).expect("one").path, "/icons/app.xpm");
}

#[test]
fn nothing_usable_found_is_nothing() {
    assert_eq!(best_icon(&[]), None);
    assert_eq!(best_icon(&[candidate("/icons/a.ico", "ico", 9)]), None);
}

#[test]
fn a_menu_items_defaults_are_enabled_and_visible() {
    let entry = TrayMenuItem::new();
    assert!(entry.enabled);
    assert!(entry.visible);
    assert!(!entry.separator);
    assert_eq!(entry.toggle_type, ToggleType::None);
    assert_eq!(entry.toggle_state, -1, "indeterminate until told otherwise");
}

#[test]
fn a_mnemonic_underscore_is_stripped() {
    // Showing the raw label would put stray underscores through the menu.
    let mut entry = TrayMenuItem::new();
    entry.label = "_File".to_owned();
    assert_eq!(entry.plain_label(), "File");

    entry.label = "Save _As".to_owned();
    assert_eq!(entry.plain_label(), "Save As");
}

#[test]
fn a_doubled_underscore_is_a_real_one() {
    // Stripping both would eat the underscores in a filename.
    let mut entry = TrayMenuItem::new();
    entry.label = "my__file".to_owned();
    assert_eq!(entry.plain_label(), "my_file");
}

#[test]
fn a_trailing_underscore_disappears() {
    let mut entry = TrayMenuItem::new();
    entry.label = "Quit_".to_owned();
    assert_eq!(entry.plain_label(), "Quit");
}

#[test]
fn three_underscores_are_one_real_one_and_a_mnemonic() {
    // The doubling is consumed first, so what is left is a single mnemonic
    // marker before whatever follows.
    let mut entry = TrayMenuItem::new();
    entry.label = "a___b".to_owned();
    assert_eq!(entry.plain_label(), "a_b");
}

#[test]
fn a_label_with_no_underscores_is_unchanged() {
    let mut entry = TrayMenuItem::new();
    entry.label = "Preferences".to_owned();
    assert_eq!(entry.plain_label(), "Preferences");
}

#[test]
fn a_label_keeps_its_non_ascii_characters() {
    // Iterated by character, not by byte, so an accented label is not cut in
    // half by the underscore search.
    let mut entry = TrayMenuItem::new();
    entry.label = "_Préférences".to_owned();
    assert_eq!(entry.plain_label(), "Préférences");
}
