//! User theme files: the TOML themes Set Theme offers besides the built-in
//! ones.
//!
//! Ports `ThemeDatabase` and `ThemeFile` (`src/server/src/theme/`): which
//! directories are read (`$XDG_DATA_HOME/vicinae/themes`, then each
//! `$XDG_DATA_DIRS/vicinae/themes`), which file wins when two share an id
//! (the first found), how a file is read (`[meta]` with `name`, `description`
//! and `variant`, colours under `[colors.*]` as `#hex`, a `colors.<key>`
//! reference, or a table with `name` and `opacity`/`lighter`/`darker`), and
//! how a colour a file leaves out is found: derived from the file's other
//! colours as `deriveSemantic` does, else inherited from its `inherits`
//! theme, else from the built-in dark or light base.
//!
//! The launcher's palette has nine slots, so only the colours that feed them
//! (and the eight swatches a row shows) are resolved; the rest of the C++'s
//! semantic set is read and kept but not derived.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, 255 opaque.
    pub a: u8,
}

impl Rgba {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parses Qt's hex forms: `#rgb`, `#rrggbb`, `#aarrggbb`, with or
    /// without the `#` (`parseColorName` retries with one).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|n| n * 17);
        match hex.len() {
            3 => Some(Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?)),
            6 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Self {
                a: byte(0)?,
                r: byte(2)?,
                g: byte(4)?,
                b: byte(6)?,
            }),
            _ => None,
        }
    }

    /// `a` towards `b` by `t`, as `ThemeFile::mix`.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }

    /// This colour drawn over `background` at its own alpha: what a
    /// translucent colour looks like on the card.
    #[must_use]
    pub fn over(self, background: Self) -> Self {
        let a = f32::from(self.a) / 255.0;
        let blend = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1.0 - a)).round() as u8;
        Self::rgb(
            blend(self.r, background.r),
            blend(self.g, background.g),
            blend(self.b, background.b),
        )
    }

    /// `QColor::lighter(factor)`: the HSV value scaled by `factor / 100`,
    /// with what overflows taken from the saturation.
    #[must_use]
    pub fn lighter(self, factor: i32) -> Self {
        if factor <= 0 {
            return self;
        }
        if factor < 100 {
            return self.darker(10_000 / factor);
        }
        let (h, s, v) = self.hsv();
        let mut v = v * factor as f32 / 100.0;
        let mut s = s;
        if v > 1.0 {
            s = (s - (v - 1.0)).max(0.0);
            v = 1.0;
        }
        Self::from_hsv(h, s, v, self.a)
    }

    /// `QColor::darker(factor)`: the HSV value divided by `factor / 100`.
    #[must_use]
    pub fn darker(self, factor: i32) -> Self {
        if factor <= 0 {
            return self;
        }
        if factor < 100 {
            return self.lighter(10_000 / factor);
        }
        let (h, s, v) = self.hsv();
        Self::from_hsv(h, s, v * 100.0 / factor as f32, self.a)
    }

    fn hsv(self) -> (f32, f32, f32) {
        let (r, g, b) = (
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let s = if max == 0.0 { 0.0 } else { delta / max };
        (h, s, max)
    }

    fn from_hsv(h: f32, s: f32, v: f32, a: u8) -> Self {
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = v - c;
        let (r, g, b) = match (h / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let byte = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self {
            r: byte(r),
            g: byte(g),
            b: byte(b),
            a,
        }
    }
}

/// A colour as a file states it.
#[derive(Debug, Clone, PartialEq)]
enum ColorSpec {
    /// A colour.
    Literal(Rgba),
    /// Another key's colour, adjusted.
    Ref {
        key: String,
        opacity: Option<f64>,
        lighter: Option<i32>,
        darker: Option<i32>,
    },
}

/// One theme file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeFile {
    /// The file name without its extension; what the configuration stores.
    pub id: String,
    /// `meta.name`.
    pub name: String,
    /// `meta.description`.
    pub description: String,
    /// `meta.variant` is anything but `light`.
    pub dark: bool,
    /// `meta.inherits`, else `vicinae-dark` or `vicinae-light`.
    pub inherits: String,
    /// `meta.icon`, relative to the file's folder unless absolute.
    pub icon: Option<PathBuf>,
    /// Where it was read from.
    pub path: PathBuf,
    /// What was wrong with it without making it unreadable.
    pub diagnostics: Vec<String>,
    colors: BTreeMap<String, ColorSpec>,
}

/// The built-in dark base's id.
pub const VICINAE_DARK: &str = "vicinae-dark";
/// The built-in light base's id.
pub const VICINAE_LIGHT: &str = "vicinae-light";

/// The directories themes are read from, in the order a clash is decided:
/// the user's, then each system data directory's (`dataSearchPaths`).
#[must_use]
pub fn search_dirs(data_home: Option<&Path>, data_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let user = data_home.map(|home| home.join("vicinae").join("themes"));
    let mut out: Vec<PathBuf> = Vec::with_capacity(1 + data_dirs.len());
    out.extend(user.clone());
    for dir in data_dirs {
        let path = dir.join("vicinae").join("themes");
        if Some(&path) != user.as_ref() && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The directories for this session's environment.
#[must_use]
pub fn default_search_dirs() -> Vec<PathBuf> {
    search_dirs(
        crate::xdg_dirs::data_home().as_deref(),
        &crate::xdg_dirs::data_dirs(),
    )
}

/// Reads every `.toml` file directly in `dirs`, in order; a file whose id an
/// earlier one (or a built-in base) already has is skipped, and one that
/// does not parse is logged and skipped, as `ThemeDatabase::scan`.
#[must_use]
pub fn scan(dirs: &[PathBuf]) -> Vec<ThemeFile> {
    let mut themes: Vec<ThemeFile> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match parse(&path, &text) {
                Ok(theme) => {
                    if theme.id == VICINAE_DARK
                        || theme.id == VICINAE_LIGHT
                        || themes.iter().any(|t| t.id == theme.id)
                    {
                        continue;
                    }
                    for diagnostic in &theme.diagnostics {
                        tracing::warn!(theme = %theme.id, "{diagnostic}");
                    }
                    themes.push(theme);
                }
                Err(error) => {
                    tracing::error!(path = %path.display(), %error, "failed to parse theme file");
                }
            }
        }
    }
    themes
}

/// Reads one theme file.
///
/// # Errors
///
/// The sentence `ThemeFile::fromFile` gives: TOML that does not parse, a
/// missing `[meta]` table or one of its three strings, or a circular
/// reference.
pub fn parse(path: &Path, text: &str) -> Result<ThemeFile, String> {
    let table: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
    let meta = table
        .get("meta")
        .and_then(toml::Value::as_table)
        .ok_or("a [meta] table is required")?;
    let string = |key: &str| meta.get(key).and_then(toml::Value::as_str);
    let name = string("name").ok_or("meta.name must be a string")?;
    let description = string("description").ok_or("meta.description must be a string")?;
    let variant = string("variant").ok_or(r#"meta.variant must be a string ("light" | "dark")"#)?;
    let dark = variant != "light";
    let id = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let inherits = string("inherits").map_or_else(
        || if dark { VICINAE_DARK } else { VICINAE_LIGHT }.to_owned(),
        str::to_owned,
    );
    let icon = string("icon").map(|icon| {
        if icon.starts_with('/') {
            PathBuf::from(icon)
        } else {
            path.parent().unwrap_or(Path::new("")).join(icon)
        }
    });
    let mut colors = BTreeMap::new();
    let mut diagnostics = Vec::new();
    if let Some(root) = table.get("colors").and_then(toml::Value::as_table) {
        collect_colors(root, "", &mut colors, &mut diagnostics);
    }
    for key in colors.keys() {
        if circular(&colors, key) {
            return Err(format!("Detected circular binding for key {key}"));
        }
    }
    Ok(ThemeFile {
        id,
        name: name.to_owned(),
        description: description.to_owned(),
        dark,
        inherits,
        icon,
        path: path.to_path_buf(),
        diagnostics,
        colors,
    })
}

fn collect_colors(
    table: &toml::Table,
    root: &str,
    colors: &mut BTreeMap<String, ColorSpec>,
    diagnostics: &mut Vec<String>,
) {
    for (key, value) in table {
        let path = if root.is_empty() {
            key.clone()
        } else {
            format!("{root}.{key}")
        };
        match value {
            toml::Value::String(text) => match color_name(text) {
                Some(spec) => {
                    colors.insert(path, spec);
                }
                None => diagnostics.push(format!(
                    "colors.{path} is not a valid color: {text} is not a valid color name or reference"
                )),
            },
            toml::Value::Table(inner) if inner.contains_key("name") => {
                let Some(mut spec) = inner
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .and_then(color_name)
                else {
                    diagnostics.push(format!("colors.{path} is not a valid color"));
                    continue;
                };
                let number = |key: &str| {
                    inner.get(key).and_then(|v| {
                        v.as_float()
                            .or_else(|| v.as_integer().map(|n| n as f64))
                    })
                };
                let opacity = number("opacity");
                let lighter = number("lighter").map(|n| n as i32);
                let darker = number("darker").map(|n| n as i32);
                spec = match spec {
                    ColorSpec::Literal(mut color) => {
                        if let Some(opacity) = opacity {
                            color.a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                        if let Some(lighter) = lighter {
                            color = color.lighter(lighter);
                        }
                        if let Some(darker) = darker {
                            color = color.darker(darker);
                        }
                        ColorSpec::Literal(color)
                    }
                    ColorSpec::Ref { key, .. } => ColorSpec::Ref {
                        key,
                        opacity,
                        lighter,
                        darker,
                    },
                };
                colors.insert(path, spec);
            }
            toml::Value::Table(inner) => collect_colors(inner, &path, colors, diagnostics),
            _ => diagnostics.push(format!("unused config key colors.{path}")),
        }
    }
}

fn color_name(text: &str) -> Option<ColorSpec> {
    if let Some(key) = text.strip_prefix("colors.") {
        return Some(ColorSpec::Ref {
            key: key.to_owned(),
            opacity: None,
            lighter: None,
            darker: None,
        });
    }
    Rgba::parse(text).map(ColorSpec::Literal)
}

fn circular(colors: &BTreeMap<String, ColorSpec>, start: &str) -> bool {
    let mut seen = vec![start.to_owned()];
    let mut key = start.to_owned();
    while let Some(ColorSpec::Ref { key: next, .. }) = colors.get(&key) {
        if next == &key {
            // A key referring to itself means "the inherited value".
            return false;
        }
        if seen.contains(next) {
            return true;
        }
        seen.push(next.clone());
        key = next.clone();
    }
    false
}

/// The colours the launcher draws with, resolved for one theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// Whether the theme is dark.
    pub dark: bool,
    /// `core.background`: the card.
    pub background: Rgba,
    /// `input.background`: the search field.
    pub input_background: Rgba,
    /// `text.default`.
    pub text: Rgba,
    /// `text.muted`, drawn over the background.
    pub muted: Rgba,
    /// `list.item.selection.background`.
    pub selection: Rgba,
    /// `list.item.selection.foreground`.
    pub selection_text: Rgba,
    /// `main_window.border`.
    pub border: Rgba,
    /// `core.accent`.
    pub accent: Rgba,
    /// The row's eight swatches, in `theme_picker::PALETTE_ORDER`.
    pub swatches: [Rgba; 8],
}

/// Resolves `theme` against the other `themes` it may inherit from.
#[must_use]
pub fn resolve(theme: &ThemeFile, themes: &[ThemeFile]) -> Resolved {
    let resolver = Resolver { themes };
    let get = |key: &str| resolver.resolve(theme, key, 0);
    let background = get("core.background");
    let muted = get("text.muted").over(background);
    Resolved {
        dark: theme.dark,
        background,
        input_background: get("input.background").over(background),
        text: get("text.default").over(background),
        muted,
        selection: get("list.item.selection.background").over(background),
        selection_text: get("list.item.selection.foreground").over(background),
        border: get("main_window.border").over(background),
        accent: get("core.accent").over(background),
        swatches: [
            get("accents.red"),
            get("accents.blue"),
            get("accents.cyan"),
            get("accents.green"),
            get("accents.magenta"),
            get("accents.orange"),
            get("core.foreground"),
            muted,
        ],
    }
}

/// `ThemeFile::vicinaeDark` and `vicinaeLight`, as keys.
fn base(dark: bool) -> &'static [(&'static str, Rgba)] {
    const DARK: &[(&str, Rgba)] = &[
        ("core.background", Rgba::rgb(0x0f, 0x10, 0x14)),
        ("core.secondary_background", Rgba::rgb(0x15, 0x16, 0x1b)),
        (
            "list.item.selection.background",
            Rgba::rgb(0x27, 0x28, 0x31),
        ),
        ("grid.item.background", Rgba::rgb(0x1b, 0x1c, 0x22)),
        ("core.foreground", Rgba::rgb(0xe7, 0xe5, 0xe4)),
        ("core.border", Rgba::rgb(0x37, 0x38, 0x42)),
        ("core.accent", Rgba::rgb(0xb8, 0x94, 0x4e)),
        ("core.accent_foreground", Rgba::rgb(0x0f, 0x10, 0x14)),
        ("accents.red", Rgba::rgb(0xb9, 0x54, 0x3b)),
        ("accents.orange", Rgba::rgb(0xf0, 0x88, 0x3e)),
        ("accents.yellow", Rgba::rgb(0xc9, 0xa7, 0x6e)),
        ("accents.green", Rgba::rgb(0x3a, 0x9c, 0x61)),
        ("accents.cyan", Rgba::rgb(0x6a, 0x8a, 0x7c)),
        ("accents.blue", Rgba::rgb(0x2f, 0x6f, 0xed)),
        ("accents.magenta", Rgba::rgb(0xbc, 0x8c, 0xff)),
        ("accents.purple", Rgba::rgb(0xbc, 0x8c, 0xff)),
    ];
    const LIGHT: &[(&str, Rgba)] = &[
        ("core.background", Rgba::rgb(0xfa, 0xf8, 0xf4)),
        ("core.secondary_background", Rgba::rgb(0xf0, 0xec, 0xe5)),
        (
            "list.item.selection.background",
            Rgba::rgb(0xca, 0xc0, 0xaa),
        ),
        ("core.foreground", Rgba::rgb(0x1c, 0x19, 0x17)),
        ("core.border", Rgba::rgb(0x82, 0x80, 0x7a)),
        ("grid.item.background", Rgba::rgb(0xe6, 0xe1, 0xd5)),
        ("core.accent", Rgba::rgb(0x8a, 0x6d, 0x35)),
        ("core.accent_foreground", Rgba::rgb(0xfa, 0xf8, 0xf4)),
        ("accents.red", Rgba::rgb(0xb9, 0x54, 0x3b)),
        ("accents.orange", Rgba::rgb(0xc9, 0x7a, 0x30)),
        ("accents.yellow", Rgba::rgb(0x9a, 0x7b, 0x3f)),
        ("accents.green", Rgba::rgb(0x2d, 0x7a, 0x4d)),
        ("accents.cyan", Rgba::rgb(0x44, 0x63, 0x5a)),
        ("accents.blue", Rgba::rgb(0x1f, 0x6f, 0xeb)),
        ("accents.magenta", Rgba::rgb(0x8b, 0x6e, 0xbf)),
        ("accents.purple", Rgba::rgb(0x8b, 0x6e, 0xbf)),
    ];
    if dark { DARK } else { LIGHT }
}

struct Resolver<'a> {
    themes: &'a [ThemeFile],
}

/// Past this depth a chain of references or parents is taken as broken.
const MAX_DEPTH: u32 = 32;

impl Resolver<'_> {
    fn parent(&self, theme: &ThemeFile) -> Option<&ThemeFile> {
        self.themes
            .iter()
            .find(|t| t.id == theme.inherits && t.id != theme.id)
    }

    /// `ThemeFile::resolve(SemanticColor)`: the file's own value, else what
    /// it derives, else its parent's.
    fn resolve(&self, theme: &ThemeFile, key: &str, depth: u32) -> Rgba {
        if depth > MAX_DEPTH {
            return Rgba::rgb(0, 0, 0);
        }
        if let Some(spec) = theme.colors.get(key) {
            return match spec {
                ColorSpec::Literal(color) => *color,
                ColorSpec::Ref {
                    key: target,
                    opacity,
                    lighter,
                    darker,
                } => {
                    let mut color = if target == key {
                        self.inherit(theme, key, depth + 1)
                    } else {
                        self.resolve(theme, target, depth + 1)
                    };
                    if let Some(opacity) = opacity {
                        let background = self.resolve(theme, "core.background", depth + 1);
                        color = Rgba {
                            a: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
                            ..color
                        }
                        .over(background);
                    }
                    if let Some(darker) = darker {
                        color = color.darker((darker + 100).max(0));
                    }
                    if let Some(lighter) = lighter {
                        color = color.lighter((lighter + 100).max(0));
                    }
                    color
                }
            };
        }
        if let Some(color) = self.derive(theme, key, depth) {
            return color;
        }
        self.inherit(theme, key, depth + 1)
    }

    fn inherit(&self, theme: &ThemeFile, key: &str, depth: u32) -> Rgba {
        if let Some(parent) = self.parent(theme) {
            return self.resolve(parent, key, depth);
        }
        base(theme.dark)
            .iter()
            .find(|(name, _)| *name == key)
            .map_or_else(|| self.derive_base(theme.dark, key, depth), |(_, c)| *c)
    }

    /// What the built-in base derives for a key it does not list.
    fn derive_base(&self, dark: bool, key: &str, depth: u32) -> Rgba {
        let base = ThemeFile {
            id: if dark { VICINAE_DARK } else { VICINAE_LIGHT }.to_owned(),
            name: String::new(),
            description: String::new(),
            dark,
            inherits: String::new(),
            icon: None,
            path: PathBuf::new(),
            diagnostics: Vec::new(),
            colors: self::base(dark)
                .iter()
                .map(|(name, color)| ((*name).to_owned(), ColorSpec::Literal(*color)))
                .collect(),
        };
        self.derive(&base, key, depth + 1)
            .unwrap_or(Rgba::rgb(0, 0, 0))
    }

    /// The part of `deriveSemantic` the launcher's palette reads.
    fn derive(&self, theme: &ThemeFile, key: &str, depth: u32) -> Option<Rgba> {
        let get = |key: &str| self.resolve(theme, key, depth + 1);
        Some(match key {
            "text.default" | "list.item.selection.foreground" => get("core.foreground"),
            "text.muted" => Rgba {
                a: (0.7_f32 * 255.0).round() as u8,
                ..get("text.default")
            },
            "list.item.selection.background" => get("core.secondary_background"),
            "grid.item.background" => get("core.secondary_background"),
            "input.background" => get("grid.item.background"),
            "core.accent" => get("accents.blue"),
            "core.accent_foreground" => Rgba::rgb(0xff, 0xff, 0xff),
            "main_window.border" => {
                let color = get("core.border");
                if theme.dark {
                    color.lighter(110)
                } else {
                    color.darker(110)
                }
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOCHA: &str = r##"
[meta]
version = 1
name = "Catppuccin Mocha"
description = "The Original"
variant = "dark"
icon = "icons/catppuccin.png"

[colors.core]
background = "#1E1E2E"
foreground = "#CDD6F4"
secondary_background = "#181825"
border = "#424265"
accent = "#89B4FA"

[colors.accents]
blue = "#89B4FA"
red = "#F38BA8"

[colors.list.item.selection]
background = "#41425d"
secondary_background = "#383A4A"

[colors.text]
muted = { name = "colors.core.foreground", opacity = 0.5 }
"##;

    #[test]
    fn a_theme_file_is_read_and_resolved_with_its_derivations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catppuccin-mocha.toml");
        let theme = parse(&path, MOCHA).unwrap();
        assert_eq!(theme.id, "catppuccin-mocha");
        assert_eq!(theme.name, "Catppuccin Mocha");
        assert!(theme.dark);
        assert_eq!(theme.inherits, VICINAE_DARK);
        assert_eq!(theme.icon, Some(dir.path().join("icons/catppuccin.png")));
        let resolved = resolve(&theme, &[]);
        assert_eq!(resolved.background, Rgba::rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(resolved.text, Rgba::rgb(0xcd, 0xd6, 0xf4));
        assert_eq!(resolved.selection, Rgba::rgb(0x41, 0x42, 0x5d));
        assert_eq!(resolved.selection_text, resolved.text, "derived");
        assert_eq!(
            resolved.input_background,
            Rgba::rgb(0x18, 0x18, 0x25),
            "grid, then the secondary background"
        );
        assert_eq!(
            resolved.muted,
            Rgba::rgb(0xcd, 0xd6, 0xf4).mix(Rgba::rgb(0x1e, 0x1e, 0x2e), 0.5)
        );
        assert_eq!(resolved.swatches[0], Rgba::rgb(0xf3, 0x8b, 0xa8));
        assert_eq!(
            resolved.swatches[2],
            Rgba::rgb(0x6a, 0x8a, 0x7c),
            "cyan comes from the dark base"
        );
    }

    #[test]
    fn a_child_inherits_from_its_parent_and_bad_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let parent = parse(&dir.path().join("mocha.toml"), MOCHA).unwrap();
        let child = parse(
            &dir.path().join("child.toml"),
            "[meta]\nname = \"C\"\ndescription = \"\"\nvariant = \"dark\"\ninherits = \"mocha\"\n[colors.core]\nbackground = \"000\"\n",
        )
        .unwrap();
        let resolved = resolve(&child, &[parent.clone(), child.clone()]);
        assert_eq!(resolved.background, Rgba::rgb(0, 0, 0));
        assert_eq!(resolved.text, Rgba::rgb(0xcd, 0xd6, 0xf4), "from mocha");

        assert!(parse(&dir.path().join("x.toml"), "[colors]").is_err());
        assert!(
            parse(
                &dir.path().join("x.toml"),
                "[meta]\nname = \"x\"\ndescription = \"\"\nvariant = \"light\"\n[colors.core]\nbackground = \"colors.core.foreground\"\nforeground = \"colors.core.background\"\n",
            )
            .unwrap_err()
            .contains("circular")
        );
        let light = parse(
            &dir.path().join("l.toml"),
            "[meta]\nname = \"l\"\ndescription = \"\"\nvariant = \"light\"\n[colors.core]\nbackground = \"nope\"\n",
        )
        .unwrap();
        assert!(!light.dark);
        assert_eq!(light.diagnostics.len(), 1);
        assert_eq!(
            resolve(&light, &[]).background,
            Rgba::rgb(0xfa, 0xf8, 0xf4),
            "the light base"
        );
    }

    #[test]
    fn the_first_directory_wins_and_the_bases_cannot_be_replaced() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("mocha.toml"), MOCHA).unwrap();
        std::fs::write(
            system.path().join("mocha.toml"),
            MOCHA.replace("Catppuccin Mocha", "System Mocha"),
        )
        .unwrap();
        std::fs::write(system.path().join("vicinae-dark.toml"), MOCHA).unwrap();
        std::fs::write(system.path().join("notes.txt"), "x").unwrap();
        let themes = scan(&[user.path().to_path_buf(), system.path().to_path_buf()]);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "Catppuccin Mocha");

        let dirs = search_dirs(
            Some(Path::new("/home/a/.local/share")),
            &[
                PathBuf::from("/usr/share"),
                PathBuf::from("/home/a/.local/share"),
            ],
        );
        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/a/.local/share/vicinae/themes"),
                PathBuf::from("/usr/share/vicinae/themes"),
            ]
        );
    }

    #[test]
    fn lighter_and_darker_follow_qt() {
        let grey = Rgba::rgb(100, 100, 100);
        assert_eq!(grey.lighter(150), Rgba::rgb(150, 150, 150));
        assert_eq!(grey.darker(200), Rgba::rgb(50, 50, 50));
        assert_eq!(Rgba::parse("#80ff0000").unwrap().a, 0x80);
        assert_eq!(Rgba::parse("fff"), Some(Rgba::rgb(255, 255, 255)));
        assert_eq!(Rgba::parse("red"), None);
    }
}
