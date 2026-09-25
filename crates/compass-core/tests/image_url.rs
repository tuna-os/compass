//! How every icon in the system is written down and read back.
//!
//! Read off `ImageURL` (`src/server/src/ui/image/url.cpp`).

use compass_core::image_url::{
    ColorLike, ImageMask, ImageUrl, ImageUrlType, SCHEME, name_for_type, type_for_name,
};

/// Nothing exists on disk.
fn nothing_exists(_path: &str) -> bool {
    false
}

#[test]
fn the_scheme_is_icon() {
    assert_eq!(SCHEME, "icon");
}

#[test]
fn a_builtin_icon_round_trips() {
    let url = ImageUrl::builtin("trash");
    let printed = url.to_url();
    assert_eq!(ImageUrl::parse(&printed), url);
}

#[test]
fn a_builtin_prints_as_omnicast_not_builtin() {
    // nameForType takes the first table entry for the type, and "omnicast"
    // comes before "builtin". Data already on disk is written that way.
    assert_eq!(ImageUrl::builtin("trash").to_url(), "icon://omnicast/trash");
    assert_eq!(name_for_type(ImageUrlType::Builtin), "omnicast");
}

#[test]
fn both_builtin_spellings_parse() {
    assert_eq!(type_for_name("omnicast"), ImageUrlType::Builtin);
    assert_eq!(type_for_name("builtin"), ImageUrlType::Builtin);
    assert!(ImageUrl::parse("icon://builtin/trash").is_builtin());
    assert!(ImageUrl::parse("icon://omnicast/trash").is_builtin());
}

#[test]
fn https_parses_as_http_and_prints_back_as_http() {
    // Both schemes share one type, and the type prints as the first name.
    assert_eq!(type_for_name("https"), ImageUrlType::Http);
    assert_eq!(name_for_type(ImageUrlType::Http), "http");

    let parsed = ImageUrl::parse("icon://https/example.test%2Fa.png");
    assert_eq!(parsed.kind, ImageUrlType::Http);
    assert!(parsed.to_url().starts_with("icon://http/"));
}

#[test]
fn an_unknown_host_is_invalid_rather_than_an_error() {
    // The C++ default-constructs one and callers test isValid(), so invalid is
    // a value: a bad icon reference must not stop a row being drawn.
    let url = ImageUrl::parse("icon://nonsense/thing");
    assert!(!url.is_valid());
    assert_eq!(url.kind, ImageUrlType::Invalid);
}

#[test]
fn a_url_with_another_scheme_is_invalid() {
    assert!(!ImageUrl::parse("https://example.test/a.png").is_valid());
    assert!(!ImageUrl::parse("").is_valid());
    assert!(!ImageUrl::parse("trash").is_valid());
}

#[test]
fn the_scheme_is_checked_even_when_the_host_would_parse() {
    // "local" and "http" are both known hosts, so a URL whose *scheme* is
    // wrong can still have a host this would accept. Only the scheme check
    // rejects these, and without it a plain web URL would be read as an icon.
    assert!(!ImageUrl::parse("http://local/x").is_valid());
    assert!(!ImageUrl::parse("file://emoji/x").is_valid());

    // And a string with no scheme at all whose first segment happens to be a
    // type name: without the `icon://` requirement this would parse, so a
    // plain relative path like "local/icon.png" would become an icon URL.
    assert!(!ImageUrl::parse("local/icon.png").is_valid());
    assert!(!ImageUrl::parse("omnicast/trash").is_valid());
}

#[test]
fn an_explicitly_invalid_type_is_not_valid() {
    // Constructing one is how the C++ default-constructs a bad icon; valid has
    // to follow the type rather than being set unconditionally.
    let url = ImageUrl::new(ImageUrlType::Invalid, "whatever");
    assert!(!url.is_valid());
    assert!(ImageUrl::new(ImageUrlType::Local, "/x").is_valid());
}

#[test]
fn every_type_name_maps_back_to_a_type() {
    for name in [
        "favicon",
        "omnicast",
        "builtin",
        "system",
        "http",
        "https",
        "local",
        "bundle",
        "win-shell",
        "win-stock",
        "file-icon",
        "emoji",
        "symbol",
        "datauri",
        "font-preview",
    ] {
        assert_ne!(
            type_for_name(name),
            ImageUrlType::Invalid,
            "{name} should be a known host"
        );
    }
}

#[test]
fn every_type_prints_to_a_name_that_parses_back_to_it() {
    // The round trip has to close for every type, or an icon written by one
    // build is unreadable by the next.
    for kind in [
        ImageUrlType::Builtin,
        ImageUrlType::Favicon,
        ImageUrlType::System,
        ImageUrlType::Http,
        ImageUrlType::Local,
        ImageUrlType::Emoji,
        ImageUrlType::Symbol,
        ImageUrlType::DataUri,
        ImageUrlType::MacBundle,
        ImageUrlType::FileIcon,
        ImageUrlType::FontPreview,
        ImageUrlType::WinShellIcon,
        ImageUrlType::WinStockIcon,
    ] {
        let name = name_for_type(kind);
        assert!(!name.is_empty(), "{kind:?} prints to nothing");
        assert_eq!(type_for_name(name), kind, "{name} does not parse back");
    }
}

#[test]
fn the_query_items_are_printed_in_the_cpp_order() {
    // The URL is stored and compared as a string, so reordering would make two
    // equal icons look different.
    let url = ImageUrl::builtin("trash")
        .with_background_tint(ColorLike::parse("blue"))
        .with_fill(Some(ColorLike::parse("red")))
        .with_badge("star")
        .circle()
        .with_fallback(&ImageUrl::builtin("question-mark-circle"));

    let printed = url.to_url();
    let order: Vec<usize> = ["fallback=", "bg_tint=", "fill=", "badge=", "mask="]
        .iter()
        .map(|key| {
            printed
                .find(key)
                .unwrap_or_else(|| panic!("{key} in {printed}"))
        })
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{printed}");
}

#[test]
fn a_fully_decorated_icon_round_trips() {
    let url = ImageUrl::local("/home/ada/icon.png")
        .with_background_tint(ColorLike::parse("yellow"))
        .with_fill(Some(ColorLike::parse("red")))
        .with_badge("star")
        .circle();

    assert_eq!(ImageUrl::parse(&url.to_url()), url);
}

#[test]
fn a_palette_name_survives_as_a_palette_name() {
    let url = ImageUrl::parse("icon://omnicast/trash?fill=red");
    assert_eq!(url.fill, Some(ColorLike::Semantic("Red".to_owned())));
    assert_eq!(url.to_url(), "icon://omnicast/trash?fill=red");
}

#[test]
fn a_colour_that_is_not_a_palette_name_is_kept_literally() {
    // The two share one field, so a hex value must not be mistaken for a role
    // nor silently dropped.
    let url = ImageUrl::parse("icon://omnicast/trash?fill=%23ff00aa");
    assert_eq!(url.fill, Some(ColorLike::Literal("#ff00aa".to_owned())));
    assert_eq!(ImageUrl::parse(&url.to_url()).fill, url.fill);
}

#[test]
fn the_text_colour_names_are_the_cpp_ones() {
    // primary-text and secondary-text are not the names of the roles they map
    // to, so a rename in either direction would break existing URLs.
    assert_eq!(
        ColorLike::parse("primary-text"),
        ColorLike::Semantic("Foreground".to_owned())
    );
    assert_eq!(
        ColorLike::parse("secondary-text"),
        ColorLike::Semantic("TextMuted".to_owned())
    );
    assert_eq!(
        ColorLike::Semantic("TextMuted".to_owned()).serialize(),
        "secondary-text"
    );
}

#[test]
fn both_masks_round_trip_and_anything_else_is_no_mask() {
    for mask in [ImageMask::Circle, ImageMask::RoundedRectangle] {
        let url = ImageUrl::builtin("trash").with_mask(mask);
        assert_eq!(ImageUrl::parse(&url.to_url()).mask, mask);
    }
    assert_eq!(ImageMask::from_name("triangle"), ImageMask::None);
    assert_eq!(ImageMask::None.name(), None, "no mask prints nothing");
}

#[test]
fn an_icon_with_no_decorations_has_no_query_at_all() {
    assert_eq!(ImageUrl::builtin("trash").to_url(), "icon://omnicast/trash");
    assert!(!ImageUrl::builtin("trash").to_url().contains('?'));
}

#[test]
fn a_fallback_is_stored_as_the_other_icons_url() {
    // A chain of fallbacks nests rather than branching, which is what lets one
    // string carry the whole chain.
    let url = ImageUrl::builtin("trash").with_fallback(&ImageUrl::builtin("question-mark-circle"));
    assert_eq!(
        url.fallback.as_deref(),
        Some("icon://omnicast/question-mark-circle")
    );
    assert_eq!(ImageUrl::parse(&url.to_url()).fallback, url.fallback);
}

#[test]
fn a_nested_fallback_survives_the_round_trip() {
    let inner = ImageUrl::builtin("a").with_fallback(&ImageUrl::builtin("b"));
    let outer = ImageUrl::builtin("c").with_fallback(&inner);
    let reparsed = ImageUrl::parse(&outer.to_url());

    let middle = ImageUrl::parse(reparsed.fallback.as_deref().expect("one level"));
    assert_eq!(middle.name, "a");
    assert_eq!(
        ImageUrl::parse(middle.fallback.as_deref().expect("two levels")).name,
        "b"
    );
}

#[test]
fn a_local_path_survives_its_slashes() {
    let url = ImageUrl::local("/home/ada/Pictures/wallpaper.png");
    assert_eq!(
        ImageUrl::parse(&url.to_url()).name,
        "/home/ada/Pictures/wallpaper.png"
    );
}

#[test]
fn an_empty_query_value_is_ignored() {
    // The C++ tests `!value.isEmpty()` before using each one, so an empty
    // fill= is no fill rather than a literal empty colour.
    let url = ImageUrl::parse("icon://omnicast/trash?fill=&badge=");
    assert_eq!(url.fill, None);
    assert_eq!(url.badge, None);
    assert!(url.is_valid());
}

#[test]
fn an_unknown_query_key_is_ignored_rather_than_invalidating_the_url() {
    // A newer build's URL must still draw on an older one.
    let url = ImageUrl::parse("icon://omnicast/trash?something=new&fill=red");
    assert!(url.is_valid());
    assert_eq!(url.fill, Some(ColorLike::Semantic("Red".to_owned())));
}

#[test]
fn a_themed_sibling_is_used_when_it_exists() {
    let themed = ImageUrl::themed_local_path("/icons/logo.svg", false, |path| {
        path == "/icons/logo@dark.svg"
    });
    assert_eq!(themed, "/icons/logo@dark.svg");
}

#[test]
fn a_light_theme_looks_for_the_light_sibling() {
    let themed = ImageUrl::themed_local_path("/icons/logo.svg", true, |path| {
        path == "/icons/logo@light.svg"
    });
    assert_eq!(themed, "/icons/logo@light.svg");
}

#[test]
fn a_path_with_no_themed_sibling_is_left_alone() {
    assert_eq!(
        ImageUrl::themed_local_path("/icons/logo.svg", false, nothing_exists),
        "/icons/logo.svg"
    );
}

#[test]
fn the_suffix_goes_before_the_first_dot_of_the_basename() {
    // indexOf from the basename's start, not lastIndexOf: "a.tar.gz" becomes
    // "a@dark.tar.gz", which is what a themed-asset convention expects.
    let themed = ImageUrl::themed_local_path("/icons/a.tar.gz", false, |_| true);
    assert_eq!(themed, "/icons/a@dark.tar.gz");
}

#[test]
fn a_dot_in_a_directory_name_is_not_mistaken_for_an_extension() {
    // The search starts after the last separator, so ~/.local/share/logo has
    // no extension despite the dot above it.
    let themed = ImageUrl::themed_local_path("/home/ada/.local/share/logo", false, |_| true);
    assert_eq!(themed, "/home/ada/.local/share/logo@dark");
}

#[test]
fn a_path_with_no_extension_gets_the_suffix_on_the_end() {
    assert_eq!(
        ImageUrl::themed_local_path("/icons/logo", false, |_| true),
        "/icons/logo@dark"
    );
}

#[test]
fn an_invalid_icon_is_not_builtin() {
    let url = ImageUrl::default();
    assert!(!url.is_valid());
    assert!(!url.is_builtin());
}

/// A disk with one builtin icon, one file, one asset and one theme icon.
struct Probe;

impl compass_core::image_url::SourceLookup for Probe {
    fn builtin(&self, name: &str) -> bool {
        name == "star"
    }
    fn file(&self, path: &str) -> bool {
        path == "/opt/logo.png"
    }
    fn asset(&self, relative: &str) -> Option<String> {
        (relative == "icon").then(|| "/ext/assets/icon.png".to_owned())
    }
    fn themed(&self, name: &str) -> bool {
        name == "firefox"
    }
}

#[test]
fn a_bare_source_is_read_as_image_url_reads_one() {
    let read = |source: &str| {
        let url = ImageUrl::from_source(source, &Probe);
        (url.kind, url.name)
    };
    assert_eq!(read("🔥"), (ImageUrlType::Emoji, "🔥".into()));
    let symbol = compass_core::glyph::glyphs()
        .iter()
        .find(|g| g.kind == compass_core::glyph::Kind::Symbol)
        .expect("a symbol in the table")
        .character;
    assert_eq!(read(symbol), (ImageUrlType::Symbol, symbol.into()));
    assert_eq!(read("star"), (ImageUrlType::Builtin, "star".into()));
    assert_eq!(
        ImageUrl::from_source("star", &Probe).fill,
        Some(ColorLike::Semantic("Foreground".into())),
        "a builtin is filled in the text colour"
    );
    assert_eq!(
        read("/opt/logo.png"),
        (ImageUrlType::Local, "/opt/logo.png".into())
    );
    assert_eq!(
        read("icon"),
        (ImageUrlType::Local, "/ext/assets/icon.png".into())
    );
    assert_eq!(read("firefox"), (ImageUrlType::System, "firefox".into()));
    assert_eq!(
        read("file:///tmp/a%20b.png"),
        (ImageUrlType::Local, "/tmp/a b.png".into())
    );
    assert_eq!(
        read("https://example.com/a.png"),
        (ImageUrlType::Http, "https://example.com/a.png".into())
    );
    assert_eq!(read("data:image/png;base64,AA").0, ImageUrlType::DataUri);
    assert_eq!(
        read("icon://omnicast/link?fill=red").0,
        ImageUrlType::Builtin,
        "an icon URL is itself"
    );
    assert!(!ImageUrl::from_source("nothing-at-all", &Probe).is_valid());
}

#[test]
fn a_remote_images_own_query_survives_the_round_trip() {
    let url = ImageUrl::http("https://example.com/a.png?size=64#top");
    let printed = url.to_url();
    assert!(!printed.contains("a.png?size"), "{printed}");
    assert_eq!(ImageUrl::parse(&printed).name, url.name);
    assert_eq!(
        ImageUrl::new(ImageUrlType::Emoji, "🎉").to_url(),
        "icon://emoji/🎉",
        "Unicode is printed as written"
    );
}
