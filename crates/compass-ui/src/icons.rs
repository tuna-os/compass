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
/// **PNG and SVG only, and that is a decision rather than an oversight.** Those
/// are what icon themes ship; the build enables exactly those two decoders
/// (see `Cargo.toml`), so accepting a third format here would resolve a file
/// the renderer then cannot draw -- an empty box, which is worse than the
/// initial the row falls back to. `.svgz` is gzipped SVG and Iced's `svg`
/// widget does not decompress it, so it is a miss for the same reason.
#[must_use]
pub fn classify(path: &Path) -> Option<IconArt> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Some(IconArt::Vector(path.to_path_buf())),
        "png" => Some(IconArt::Raster(path.to_path_buf())),
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
        assert_eq!(classify(Path::new("/i/app.jpg")), None);
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
}
