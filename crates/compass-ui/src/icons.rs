//! Resolving a desktop entry's `Icon=` to a file the renderer can draw (#85).
//!
//! Two things live here, and only one of them touches the disk.
//!
//! [`classify`] and [`IconArt`] are the decision: given an icon name and a way
//! to look names up, which file do we draw and with which widget. [`IconCache`]
//! is the memoisation around it, and it caches the *negative* answer too --
//! an application whose icon the theme cannot resolve is the common case on a
//! mixed desktop, and re-walking every theme directory for it on each keystroke
//! is the cost this exists to avoid.
//!
//! The lookup itself is injected rather than called, so the tests here run
//! against a closure and never read the invoking user's icon themes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which widget draws a resolved icon.
///
/// SVG and raster are separate widgets in Iced, so the distinction has to
/// survive out of resolution rather than being rediscovered at draw time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconArt {
    /// A PNG, XPM or other bitmap: `iced::widget::image`.
    Raster(PathBuf),
    /// An SVG: `iced::widget::svg`.
    Vector(PathBuf),
}

impl IconArt {
    /// The file this art came from.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            IconArt::Raster(path) | IconArt::Vector(path) => path,
        }
    }
}

/// Which widget a resolved file needs, by extension.
///
/// **PNG, JPEG and SVG only, and that is a decision rather than an
/// oversight.** PNG and SVG are what icon themes ship, and JPEG is what remote
/// images (avatars, mostly) are; the build enables exactly those decoders
/// (see `Cargo.toml`), so accepting another format here would resolve a file
/// the renderer then cannot draw -- an empty box, which is worse than the
/// initial the row falls back to. `.svgz` is gzipped SVG and Iced's `svg`
/// widget does not decompress it, so it is a miss for the same reason.
#[must_use]
pub fn classify(path: &Path) -> Option<IconArt> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Some(IconArt::Vector(path.to_path_buf())),
        "png" | "jpg" | "jpeg" => Some(IconArt::Raster(path.to_path_buf())),
        _ => None,
    }
}

/// Resolve one `Icon=` value.
///
/// The key may be an absolute path rather than a theme name -- the desktop
/// entry specification allows it, and Flatpak-exported entries written by hand
/// sometimes use it. An absolute path is taken as given and **not** offered to
/// the theme lookup, which would search for a name containing slashes and find
/// nothing.
pub fn resolve(icon: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<IconArt> {
    if icon.is_empty() {
        return None;
    }

    let candidate = Path::new(icon);
    if candidate.is_absolute() {
        return classify(candidate);
    }

    classify(&find(icon)?)
}

/// Resolved icons, by `Icon=` value.
///
/// Holds the misses as well as the hits: see the module docs.
#[derive(Debug, Default)]
pub struct IconCache {
    entries: HashMap<String, Option<IconArt>>,
}

impl IconCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The art for `icon`, resolving and remembering it on first ask.
    pub fn get(&mut self, icon: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<&IconArt> {
        if !self.entries.contains_key(icon) {
            self.entries.insert(icon.to_owned(), resolve(icon, find));
        }
        self.entries.get(icon).and_then(Option::as_ref)
    }

    /// The art for `icon` if it has already been resolved.
    ///
    /// Immutable, because `view` is. Resolution walks theme directories, which
    /// is disk work and does not belong in a draw; [`IconCache::warm`] does it
    /// from `update`, for the rows about to be drawn, and this reads the
    /// result. A name that was never warmed reads as absent and the row falls
    /// back to its initial, which is also what a genuine miss looks like --
    /// the two are indistinguishable on purpose, so a warming bug degrades to
    /// today's appearance rather than to an empty row.
    #[must_use]
    pub fn cached(&self, icon: &str) -> Option<&IconArt> {
        self.entries.get(icon).and_then(Option::as_ref)
    }

    /// Resolve every name in `icons` that is not already known.
    ///
    /// Called with the icons of the rows about to be drawn, so the cost is
    /// bounded by what is on screen rather than by the size of the index.
    pub fn warm<'a>(
        &mut self,
        icons: impl IntoIterator<Item = &'a str>,
        find: &dyn Fn(&str) -> Option<PathBuf>,
    ) {
        for icon in icons {
            self.get(icon, find);
        }
    }

    /// How many names have been resolved, hit or miss. For tests and `doctor`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been resolved yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What a row's icon slot draws, decided in `update` and read by `view`.
///
/// Ports the part of `ImageURL` a builtin list row uses: a themed or
/// application icon keeps its own colours; a builtin icon is single-colour and
/// drawn in `fill` (the row's text colour when `None`), on a rounded `tile`
/// when the C++ sets `setBackgroundTint`, with a `badge` in the corner when it
/// sets `setBadge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Glyph {
    /// A file from an icon theme or an application.
    Art(IconArt),
    /// One of the builtin icons, by name.
    Builtin {
        /// The builtin icon's name, e.g. `copy-clipboard`.
        name: String,
        /// Its colour; `None` is the row's text colour.
        fill: Option<crate::design::Rgb>,
        /// The tile behind it.
        tile: Option<crate::design::Rgb>,
        /// A second builtin drawn small in the bottom-right corner.
        badge: Option<&'static str>,
    },
}

impl Glyph {
    /// A plain builtin, in the row's text colour.
    #[must_use]
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin {
            name: name.into(),
            fill: None,
            tile: None,
            badge: None,
        }
    }
}

/// The C++ Vicinae dark theme's accents (`ThemeFile::vicinaeDark`), which a
/// command tile is drawn from before [`tile_tone`] evens them out.
const TILE_ACCENTS: [(compass_core::commands::Tile, crate::design::Rgb); 8] = {
    use crate::design::Rgb;
    use compass_core::commands::Tile;
    [
        (Tile::Red, Rgb::new(0xb9, 0x54, 0x3b)),
        (Tile::Orange, Rgb::new(0xf0, 0x88, 0x3e)),
        (Tile::Yellow, Rgb::new(0xc9, 0xa7, 0x6e)),
        (Tile::Green, Rgb::new(0x3a, 0x9c, 0x61)),
        (Tile::Cyan, Rgb::new(0x6a, 0x8a, 0x7c)),
        (Tile::Blue, Rgb::new(0x2f, 0x6f, 0xed)),
        (Tile::Purple, Rgb::new(0xbc, 0x8c, 0xff)),
        (Tile::Gray, Rgb::new(128, 132, 138)),
    ]
};

/// The colour a command's tile is filled with, `accent` being the palette's.
#[must_use]
pub fn tile_color(
    tile: compass_core::commands::Tile,
    accent: crate::design::Rgb,
) -> crate::design::Rgb {
    let base = TILE_ACCENTS
        .iter()
        .find(|(candidate, _)| *candidate == tile)
        .map_or(accent, |(_, rgb)| *rgb);
    tile_tone(base)
}

fn to_core(rgb: crate::design::Rgb) -> compass_core::contrast::Rgb {
    compass_core::contrast::Rgb::new(rgb.r, rgb.g, rgb.b)
}

fn from_core(rgb: compass_core::contrast::Rgb) -> crate::design::Rgb {
    crate::design::Rgb::new(rgb.r, rgb.g, rgb.b)
}

/// `clampTileTone`: every tile pulled into one tonal band (saturation
/// 0.55–0.8 unless grey, lightness 0.42–0.52), so a row of them reads as a
/// family and always carries a light glyph.
#[must_use]
pub fn tile_tone(color: crate::design::Rgb) -> crate::design::Rgb {
    use compass_core::contrast::{Hsl, from_hsl, to_hsl};
    let hsl = to_hsl(to_core(color));
    // Qt's HSL is on 0..=255; the C++'s float bounds scaled onto it.
    let saturation = if hsl.saturation > 12 {
        hsl.saturation.clamp(140, 204)
    } else {
        hsl.saturation
    };
    from_core(from_hsl(Hsl {
        hue: Some(hsl.hue.unwrap_or(0)),
        saturation,
        lightness: hsl.lightness.clamp(107, 133),
    }))
}

/// The glyph colour on a tile: `getTonalContrastColor(tile, 5, 0.1)`.
#[must_use]
pub fn on_tile(tile: crate::design::Rgb) -> crate::design::Rgb {
    from_core(compass_core::contrast::tonal_contrast_color_with_amount(
        to_core(tile),
        5.0,
        0.1,
    ))
}

/// A builtin command's glyph: its icon on its C++ tile, with its badge.
#[must_use]
pub fn command_glyph(
    command: &compass_core::commands::BuiltinCommand,
    accent: crate::design::Rgb,
) -> Glyph {
    let tile = tile_color(command.kind.tile(), accent);
    Glyph::Builtin {
        name: command.icon.to_owned(),
        fill: Some(on_tile(tile)),
        tile: Some(tile),
        badge: command.kind.badge(),
    }
}

/// The MIME type a file is shown as, and the two theme names its icon is
/// looked up by: `QMimeType::iconName` and `genericIconName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIconNames {
    /// The MIME type, e.g. `image/png`.
    pub mime: String,
    /// The specific icon, e.g. `image-png`.
    pub icon: String,
    /// The generic one, e.g. `image-x-generic`.
    pub generic: String,
}

/// [`FileIconNames`] for a MIME type, by the shared-mime-info defaults Qt
/// falls back on: the type with `/` made `-`, and the media type with
/// `-x-generic`. A directory's generic icon is `folder`, as shared-mime-info
/// declares for `inode/directory`.
#[must_use]
pub fn names_for_mime(mime: &str) -> FileIconNames {
    let icon = mime.replace('/', "-");
    let generic = if mime == "inode/directory" {
        "folder".to_owned()
    } else {
        let media = mime.split('/').next().unwrap_or(mime);
        format!("{media}-x-generic")
    };
    FileIconNames {
        mime: mime.to_owned(),
        icon,
        generic,
    }
}

/// The MIME type of the file at `path`: `inode/directory` for a directory,
/// else by its extension through `mime_guess`, else
/// `application/octet-stream`.
#[must_use]
pub fn mime_for_path(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_owned();
    }
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

/// `renderFileIcon`: the theme's icon for the file's MIME type, else its
/// generic icon, else the builtin `folder` or `blank-document` in the text
/// colour.
pub fn file_glyph(path: &Path, find: &dyn Fn(&str) -> Option<PathBuf>) -> Glyph {
    let names = names_for_mime(&mime_for_path(path));
    if let Some(art) = resolve(&names.icon, find).or_else(|| resolve(&names.generic, find)) {
        return Glyph::Art(art);
    }
    Glyph::builtin(if names.mime == "inode/directory" {
        "folder"
    } else {
        "blank-document"
    })
}

/// A clipboard row's builtin, by what was copied (`iconForEntry`).
#[must_use]
pub fn clipboard_glyph(kind: crate::backend::ClipboardRowKind) -> Glyph {
    use crate::backend::ClipboardRowKind;
    Glyph::builtin(match kind {
        ClipboardRowKind::Image => "image",
        ClipboardRowKind::Link => "link",
        ClipboardRowKind::Text => "text",
        ClipboardRowKind::File => "folder",
        ClipboardRowKind::Unknown => compass_core::builtin_icon::UNKNOWN,
    })
}

/// The pickers' mark on the current default: `CheckCircle` filled green.
#[must_use]
pub fn default_mark() -> Glyph {
    Glyph::Builtin {
        name: "check-circle".to_owned(),
        fill: Some(TILE_ACCENTS[3].1),
        tile: None,
        badge: None,
    }
}

/// File glyphs by path, warmed from `update` as [`IconCache`] is.
#[derive(Debug, Default)]
pub struct FileGlyphCache {
    entries: HashMap<String, Glyph>,
}

impl FileGlyphCache {
    /// Resolve every path in `paths` that is not already known.
    pub fn warm<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a str>,
        find: &dyn Fn(&str) -> Option<PathBuf>,
    ) {
        for path in paths {
            if !self.entries.contains_key(path) {
                self.entries
                    .insert(path.to_owned(), file_glyph(Path::new(path), find));
            }
        }
    }

    /// The glyph for `path`, if it has been warmed.
    #[must_use]
    pub fn cached(&self, path: &str) -> Option<&Glyph> {
        self.entries.get(path)
    }

    /// How many paths have been resolved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been resolved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A theme lookup with a fixed set of answers.
    fn theme(answers: &[(&str, &str)]) -> impl Fn(&str) -> Option<PathBuf> {
        let answers: Vec<(String, PathBuf)> = answers
            .iter()
            .map(|(name, path)| ((*name).to_owned(), PathBuf::from(*path)))
            .collect();
        move |name: &str| {
            answers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, path)| path.clone())
        }
    }

    #[test]
    fn an_svg_and_a_png_need_different_widgets() {
        assert_eq!(
            classify(Path::new("/i/firefox.svg")),
            Some(IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert_eq!(
            classify(Path::new("/i/firefox.png")),
            Some(IconArt::Raster(PathBuf::from("/i/firefox.png")))
        );
    }

    #[test]
    fn the_extension_is_matched_without_regard_to_case() {
        assert_eq!(
            classify(Path::new("/i/App.SVG")),
            Some(IconArt::Vector(PathBuf::from("/i/App.SVG")))
        );
        assert_eq!(
            classify(Path::new("/i/App.PNG")),
            Some(IconArt::Raster(PathBuf::from("/i/App.PNG")))
        );
    }

    #[test]
    fn a_format_neither_widget_can_draw_resolves_to_nothing() {
        // `.svgz` is gzipped SVG: the svg widget does not decompress it and the
        // image widget cannot read it, so the row falls back to the initial
        // rather than drawing an empty box.
        assert_eq!(classify(Path::new("/i/app.svgz")), None);
        assert_eq!(classify(Path::new("/i/app.icns")), None);
        assert_eq!(classify(Path::new("/i/app")), None);
        // Formats the `image` crate can decode but this build ships no decoder
        // for. Accepting one would draw an empty box.
        assert_eq!(classify(Path::new("/i/app.webp")), None);
        assert_eq!(classify(Path::new("/i/app.xpm")), None);
    }

    #[test]
    fn an_absolute_icon_value_is_taken_as_given() {
        // The desktop entry specification allows `Icon=` to be a path. Handing
        // it to the theme lookup would search for a name containing slashes.
        let find = theme(&[]);
        assert_eq!(
            resolve("/opt/thing/icon.png", &find),
            Some(IconArt::Raster(PathBuf::from("/opt/thing/icon.png")))
        );
    }

    #[test]
    fn a_name_goes_through_the_theme_lookup() {
        let find = theme(&[("firefox", "/usr/share/icons/h/firefox.svg")]);
        assert_eq!(
            resolve("firefox", &find),
            Some(IconArt::Vector(PathBuf::from(
                "/usr/share/icons/h/firefox.svg"
            )))
        );
    }

    #[test]
    fn a_name_the_theme_does_not_have_resolves_to_nothing() {
        let find = theme(&[]);
        assert_eq!(resolve("no-such-app", &find), None);
    }

    #[test]
    fn an_empty_icon_value_is_not_a_lookup() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };
        assert_eq!(resolve("", &find), None);
        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    #[test]
    fn the_cache_resolves_a_name_once() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            Some(PathBuf::from("/i/firefox.png"))
        };

        let mut cache = IconCache::new();
        for _ in 0..5 {
            assert!(cache.get("firefox", &find).is_some());
        }
        assert_eq!(*asked.borrow(), ["firefox"]);
    }

    #[test]
    fn the_cache_remembers_a_miss_too() {
        // The point of the cache. An application whose icon the theme cannot
        // resolve is the common case on a mixed desktop, and re-walking every
        // theme directory for it on each keystroke is what this avoids.
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };

        let mut cache = IconCache::new();
        for _ in 0..5 {
            assert!(cache.get("no-such-app", &find).is_none());
        }
        assert_eq!(*asked.borrow(), ["no-such-app"]);
        assert_eq!(cache.len(), 1, "the miss was not remembered");
    }

    #[test]
    fn warming_resolves_the_names_it_is_given_and_nothing_else() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            Some(PathBuf::from(format!("/i/{name}.png")))
        };

        let mut cache = IconCache::new();
        cache.warm(["a", "b"], &find);
        assert_eq!(*asked.borrow(), ["a", "b"]);

        // The cost is bounded by what is on screen: warming the same rows
        // again asks nothing.
        cache.warm(["a", "b"], &find);
        assert_eq!(*asked.borrow(), ["a", "b"]);
    }

    #[test]
    fn a_name_that_was_never_warmed_reads_as_absent() {
        // Not an error and not a panic: the row falls back to its initial, the
        // same as for a name the theme genuinely does not have.
        let find = theme(&[("firefox", "/i/firefox.png")]);
        let mut cache = IconCache::new();
        cache.warm(["firefox"], &find);
        assert!(cache.cached("firefox").is_some());
        assert!(cache.cached("thunderbird").is_none());
    }

    #[test]
    fn different_names_are_cached_separately() {
        let find = theme(&[("a", "/i/a.png"), ("b", "/i/b.svg")]);
        let mut cache = IconCache::new();
        assert_eq!(
            cache.get("a", &find).map(IconArt::path),
            Some(Path::new("/i/a.png"))
        );
        assert_eq!(
            cache.get("b", &find).map(IconArt::path),
            Some(Path::new("/i/b.svg"))
        );
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_file_takes_its_mime_icon_then_the_generic_one_then_a_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("photo.png");
        let notes = dir.path().join("notes.txt");
        let blob = dir.path().join("blob.qqqq");
        for file in [&png, &notes, &blob] {
            std::fs::write(file, b"x").unwrap();
        }
        let find = theme(&[
            ("image-png", "/i/image-png.svg"),
            ("text-x-generic", "/i/text-x-generic.png"),
            ("folder", "/i/folder.svg"),
        ]);
        assert_eq!(
            file_glyph(&png, &find),
            Glyph::Art(IconArt::Vector(PathBuf::from("/i/image-png.svg")))
        );
        assert_eq!(
            file_glyph(&notes, &find),
            Glyph::Art(IconArt::Raster(PathBuf::from("/i/text-x-generic.png"))),
            "text/plain has no text-plain icon here, so the generic one"
        );
        assert_eq!(
            file_glyph(dir.path(), &find),
            Glyph::Art(IconArt::Vector(PathBuf::from("/i/folder.svg"))),
            "a directory's generic icon is folder"
        );
        assert_eq!(file_glyph(&blob, &find), Glyph::builtin("blank-document"));
        assert_eq!(
            file_glyph(dir.path(), &theme(&[])),
            Glyph::builtin("folder")
        );
    }

    #[test]
    fn mime_icon_names_follow_the_shared_mime_info_defaults() {
        let names = names_for_mime("application/pdf");
        assert_eq!(names.icon, "application-pdf");
        assert_eq!(names.generic, "application-x-generic");
        assert_eq!(names_for_mime("inode/directory").generic, "folder");
    }

    #[test]
    fn file_glyphs_are_resolved_once_per_path() {
        let asked = RefCell::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(name.to_owned());
            None
        };
        let mut cache = FileGlyphCache::default();
        cache.warm(["/nowhere/a.png", "/nowhere/a.png"], &find);
        cache.warm(["/nowhere/a.png"], &find);
        assert_eq!(*asked.borrow(), ["image-png", "image-x-generic"]);
        assert_eq!(
            cache.cached("/nowhere/a.png"),
            Some(&Glyph::builtin("blank-document"))
        );
        assert!(cache.cached("/nowhere/b.png").is_none());
    }

    #[test]
    fn a_command_is_drawn_on_its_tile_with_a_light_glyph() {
        use crate::design::Rgb;
        let accent = Rgb::new(0x35, 0x84, 0xe4);
        let clipboard = compass_core::commands::by_id("commands:clipboard-history").unwrap();
        let Glyph::Builtin {
            name,
            fill,
            tile,
            badge,
        } = command_glyph(clipboard, accent)
        else {
            panic!("a builtin command draws a builtin");
        };
        assert_eq!(name, "copy-clipboard");
        assert_eq!(badge, None);
        let tile = tile.expect("clipboard history has a red tile");
        assert!(tile.r > tile.g && tile.r > tile.b, "red: {tile:?}");
        let hsl = compass_core::contrast::to_hsl(to_core(tile));
        assert!((107..=133).contains(&hsl.lightness), "{hsl:?}");
        let fill = fill.expect("a tile carries a contrasting glyph");
        assert!(
            compass_core::contrast::contrast_ratio(to_core(tile), to_core(fill)) >= 4.5,
            "{fill:?} on {tile:?}"
        );
        let create = compass_core::commands::by_id("commands:create-snippet").unwrap();
        assert!(matches!(
            command_glyph(create, accent),
            Glyph::Builtin {
                badge: Some("plus"),
                ..
            }
        ));
    }

    #[test]
    fn a_grey_tile_stays_grey_and_accent_follows_the_palette() {
        use crate::design::Rgb;
        let grey = tile_color(compass_core::commands::Tile::Gray, Rgb::new(0, 0, 0));
        assert!(grey.r.abs_diff(grey.b) < 16, "{grey:?}");
        let accent = Rgb::new(0x20, 0xa0, 0x20);
        let tile = tile_color(compass_core::commands::Tile::Accent, accent);
        assert!(tile.g > tile.r && tile.g > tile.b, "{tile:?}");
    }

    #[test]
    fn clipboard_rows_and_the_default_mark_use_the_cpps_builtins() {
        use crate::backend::ClipboardRowKind;
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Image),
            Glyph::builtin("image")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Link),
            Glyph::builtin("link")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Text),
            Glyph::builtin("text")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::File),
            Glyph::builtin("folder")
        );
        assert_eq!(
            clipboard_glyph(ClipboardRowKind::Unknown),
            Glyph::builtin("question-mark-circle")
        );
        let Glyph::Builtin { name, fill, .. } = default_mark() else {
            panic!("the mark is a builtin");
        };
        assert_eq!(name, "check-circle");
        let fill = fill.unwrap();
        assert!(fill.g > fill.r && fill.g > fill.b, "green: {fill:?}");
    }
}
