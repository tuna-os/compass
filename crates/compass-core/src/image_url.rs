//! `icon://` — how every icon in the system is referred to.
//!
//! A port of `ImageURL` (`src/server/src/ui/image/url.cpp`), minus the
//! painting, the theme resolution and the platform icon lookups.
//!
//! # This is a wire format, not a widget
//!
//! An `ImageURL` is a *string* — `icon://<type>/<name>?fill=…&mask=…` — and
//! that string crosses every boundary in the system: it is what a root-search
//! row stores, what an extension gets handed back, what the alert model
//! carries. Anything that changes how it parses or how it prints silently
//! changes the meaning of data already on disk, so the round trip is what the
//! tests are about.

use std::fmt::Write as _;

/// What kind of thing an `ImageURL` points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageUrlType {
    /// Not an icon URL at all.
    #[default]
    Invalid,
    /// One of the built-in SVGs.
    Builtin,
    /// A website's favicon, by domain.
    Favicon,
    /// A themed system icon, by name.
    System,
    /// An `http`/`https` image. Both schemes land here.
    Http,
    /// A file on disk.
    Local,
    /// An emoji character.
    Emoji,
    /// A non-emoji glyph.
    Symbol,
    /// An inline `data:` image.
    DataUri,
    /// A macOS application bundle.
    MacBundle,
    /// The icon a file's type is shown with.
    FileIcon,
    /// A glyph rendered in a particular font.
    FontPreview,
    /// A Windows shell icon.
    WinShellIcon,
    /// A Windows stock icon.
    WinStockIcon,
}

/// The host names, in the C++ table's order.
///
/// The order matters twice over: parsing takes the first name that matches,
/// and *printing* takes the first entry whose type matches. So `https` parses
/// as [`ImageUrlType::Http`] but prints back as `http`, and
/// [`ImageUrlType::Builtin`] prints as `omnicast` rather than `builtin`.
/// Both are load-bearing for data already written down.
const ICON_TYPES: &[(&str, ImageUrlType)] = &[
    ("favicon", ImageUrlType::Favicon),
    ("omnicast", ImageUrlType::Builtin),
    ("builtin", ImageUrlType::Builtin),
    ("system", ImageUrlType::System),
    ("http", ImageUrlType::Http),
    ("https", ImageUrlType::Http),
    ("local", ImageUrlType::Local),
    ("bundle", ImageUrlType::MacBundle),
    ("win-shell", ImageUrlType::WinShellIcon),
    ("win-stock", ImageUrlType::WinStockIcon),
    ("file-icon", ImageUrlType::FileIcon),
    ("emoji", ImageUrlType::Emoji),
    ("symbol", ImageUrlType::Symbol),
    ("datauri", ImageUrlType::DataUri),
    ("font-preview", ImageUrlType::FontPreview),
];

/// The colour names an icon's `fill` or `bg_tint` can use, in table order.
///
/// Anything not on this list is taken as a literal colour, so a theme's
/// palette and a hex value share one field.
const COLOR_TINTS: &[(&str, &str)] = &[
    ("blue", "Blue"),
    ("green", "Green"),
    ("magenta", "Magenta"),
    ("purple", "Purple"),
    ("orange", "Orange"),
    ("red", "Red"),
    ("yellow", "Yellow"),
    ("cyan", "Cyan"),
    ("accent", "Accent"),
    ("primary-text", "Foreground"),
    ("secondary-text", "TextMuted"),
];

/// The scheme every icon URL uses.
pub const SCHEME: &str = "icon";

/// How an icon is clipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageMask {
    /// Not clipped.
    #[default]
    None,
    /// Clipped to a circle.
    Circle,
    /// Clipped to a rounded rectangle.
    RoundedRectangle,
}

impl ImageMask {
    /// The name this appears as in a URL's query.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Circle => Some("circle"),
            Self::RoundedRectangle => Some("roundedRectangle"),
        }
    }

    /// The mask `name` denotes; anything unrecognised is [`Self::None`].
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "circle" => Self::Circle,
            "roundedRectangle" => Self::RoundedRectangle,
            _ => Self::None,
        }
    }
}

/// A colour, either named by the theme or given literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorLike {
    /// One of the theme's roles.
    Semantic(String),
    /// A literal colour, as written.
    Literal(String),
}

impl ColorLike {
    /// The colour `text` denotes: a palette name if it is one, else a literal.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        COLOR_TINTS
            .iter()
            .find(|(name, _)| *name == text)
            .map_or_else(
                || Self::Literal(text.to_owned()),
                |(_, semantic)| Self::Semantic((*semantic).to_owned()),
            )
    }

    /// How this is written into a URL.
    #[must_use]
    pub fn serialize(&self) -> String {
        match self {
            Self::Semantic(role) => COLOR_TINTS
                .iter()
                .find(|(_, name)| *name == role)
                .map_or_else(|| role.clone(), |(name, _)| (*name).to_owned()),
            Self::Literal(text) => text.clone(),
        }
    }
}

/// The type `name` denotes.
#[must_use]
pub fn type_for_name(name: &str) -> ImageUrlType {
    ICON_TYPES
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map_or(ImageUrlType::Invalid, |(_, kind)| *kind)
}

/// The name `kind` is written as: the *first* table entry for it.
#[must_use]
pub fn name_for_type(kind: ImageUrlType) -> &'static str {
    ICON_TYPES
        .iter()
        .find(|(_, candidate)| *candidate == kind)
        .map_or("", |(name, _)| *name)
}

/// One icon reference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImageUrl {
    /// What kind of icon it is.
    pub kind: ImageUrlType,
    /// Whether it parsed.
    valid: bool,
    /// The type-specific name, path or URL.
    pub name: String,
    /// A colour behind it.
    pub background_tint: Option<ColorLike>,
    /// A colour to recolour it with.
    pub fill: Option<ColorLike>,
    /// How it is clipped.
    pub mask: ImageMask,
    /// Another icon URL to use if this one cannot be drawn.
    pub fallback: Option<String>,
    /// A built-in icon's name, drawn small in the corner.
    pub badge: Option<String>,
}

impl ImageUrl {
    /// An icon of `kind` called `name`.
    #[must_use]
    pub fn new(kind: ImageUrlType, name: impl Into<String>) -> Self {
        Self {
            kind,
            valid: kind != ImageUrlType::Invalid,
            name: name.into(),
            ..Self::default()
        }
    }

    /// A built-in icon.
    #[must_use]
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::new(ImageUrlType::Builtin, name)
    }

    /// A file on disk.
    #[must_use]
    pub fn local(path: impl Into<String>) -> Self {
        Self::new(ImageUrlType::Local, path)
    }

    /// A remote image.
    #[must_use]
    pub fn http(url: impl Into<String>) -> Self {
        Self::new(ImageUrlType::Http, url)
    }

    /// Whether this parsed as an icon URL.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    /// Whether this names a built-in icon.
    #[must_use]
    pub fn is_builtin(&self) -> bool {
        self.kind == ImageUrlType::Builtin
    }

    /// Recolour it.
    #[must_use]
    pub fn with_fill(mut self, fill: Option<ColorLike>) -> Self {
        self.fill = fill;
        self
    }

    /// Put a colour behind it.
    #[must_use]
    pub fn with_background_tint(mut self, tint: ColorLike) -> Self {
        self.background_tint = Some(tint);
        self
    }

    /// Clip it.
    #[must_use]
    pub fn with_mask(mut self, mask: ImageMask) -> Self {
        self.mask = mask;
        self
    }

    /// Clip it to a circle.
    #[must_use]
    pub fn circle(self) -> Self {
        self.with_mask(ImageMask::Circle)
    }

    /// Fall back to `fallback` when this cannot be drawn.
    ///
    /// Stored as the fallback's *string*, as the C++ does, so a chain of
    /// fallbacks nests rather than branching.
    #[must_use]
    pub fn with_fallback(mut self, fallback: &Self) -> Self {
        self.fallback = Some(fallback.to_url());
        self
    }

    /// Badge it with a built-in icon.
    #[must_use]
    pub fn with_badge(mut self, builtin_name: impl Into<String>) -> Self {
        self.badge = Some(builtin_name.into());
        self
    }

    /// Print this as a URL.
    ///
    /// The query items go in the C++'s order — fallback, bg_tint, fill, badge,
    /// mask — because the string is compared and stored as a string, and
    /// reordering would make two equal icons look different.
    #[must_use]
    pub fn to_url(&self) -> String {
        let mut url = format!("{SCHEME}://{}/{}", name_for_type(self.kind), self.name);
        let mut query: Vec<String> = Vec::new();

        if let Some(fallback) = &self.fallback {
            query.push(format!("fallback={}", encode_query_component(fallback)));
        }
        if let Some(tint) = &self.background_tint {
            query.push(format!(
                "bg_tint={}",
                encode_query_component(&tint.serialize())
            ));
        }
        if let Some(fill) = &self.fill {
            query.push(format!(
                "fill={}",
                encode_query_component(&fill.serialize())
            ));
        }
        if let Some(badge) = &self.badge {
            query.push(format!("badge={}", encode_query_component(badge)));
        }
        if let Some(mask) = self.mask.name() {
            query.push(format!("mask={mask}"));
        }

        if !query.is_empty() {
            let _ = write!(url, "?{}", query.join("&"));
        }
        url
    }

    /// Parse a URL.
    ///
    /// Anything whose scheme is not `icon`, or whose host is not a known type,
    /// is invalid — and invalid is a *value*, not an error, because the C++
    /// default-constructs one and callers test `isValid()`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let Some(rest) = text.strip_prefix(&format!("{SCHEME}://")) else {
            return Self::default();
        };

        let (host, after_host) = rest.split_once('/').unwrap_or((rest, ""));
        let kind = type_for_name(host);
        if kind == ImageUrlType::Invalid {
            return Self::default();
        }

        let (path, query) = after_host.split_once('?').unwrap_or((after_host, ""));

        let mut url = Self {
            kind,
            valid: true,
            name: decode_query_component(path),
            ..Self::default()
        };

        for item in query.split('&').filter(|item| !item.is_empty()) {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            let value = decode_query_component(value);
            if value.is_empty() {
                continue;
            }
            match key {
                "bg_tint" => url.background_tint = Some(ColorLike::parse(&value)),
                "fill" => url.fill = Some(ColorLike::parse(&value)),
                "fallback" => url.fallback = Some(value),
                "badge" => url.badge = Some(value),
                "mask" => url.mask = ImageMask::from_name(&value),
                _ => {}
            }
        }

        url
    }

    /// The local path's themed sibling, where one exists.
    ///
    /// `icon.svg` becomes `icon@dark.svg` under a dark theme. The suffix goes
    /// before the *first* dot after the last separator, so `a.tar.gz` becomes
    /// `a@dark.tar.gz` rather than `a.tar@dark.gz` — and a path with no dot at
    /// all just gets the suffix on the end.
    #[must_use]
    pub fn themed_local_path(path: &str, is_light: bool, exists: impl Fn(&str) -> bool) -> String {
        let suffix = if is_light { "@light" } else { "@dark" };
        let base_start = path.rfind('/').map_or(0, |index| index + 1);
        let themed = match path[base_start..].find('.') {
            None => format!("{path}{suffix}"),
            Some(offset) => {
                let dot = base_start + offset;
                format!("{}{suffix}{}", &path[..dot], &path[dot..])
            }
        };

        if exists(&themed) {
            themed
        } else {
            path.to_owned()
        }
    }
}

/// Percent-encode a query component.
fn encode_query_component(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, crate::uri::QUERY_VALUE).to_string()
}

/// Undo [`encode_query_component`], leniently: a malformed escape is kept as
/// written and invalid UTF-8 is replaced.
fn decode_query_component(text: &str) -> String {
    percent_encoding::percent_decode_str(text)
        .decode_utf8_lossy()
        .into_owned()
}
