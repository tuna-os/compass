//! The launcher's visual vocabulary: one source of truth for both renderers.
//!
//! Two things draw this launcher. Iced draws the real one, on a compositor.
//! The design page under `tools/design/` draws a surrogate in a browser, so a
//! change can be looked at in a second instead of after a twenty-minute VM
//! run. They agree because they read the same numbers from here — the browser
//! page is served this module as JSON rather than having the palette retyped
//! into CSS, which is how a mock starts lying.
//!
//! What the surrogate proves is layout, colour and state. What it cannot prove
//! is that Iced paints the same thing, and the VM tier remains the only judge
//! of that.

use serde::Serialize;

/// What the desktop says it prefers, as `org.freedesktop.appearance`
/// `color-scheme` reports it over the XDG settings portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ColorScheme {
    /// The desktop has no opinion; the application picks.
    NoPreference,
    /// The desktop asks for dark.
    PreferDark,
    /// The desktop asks for light.
    PreferLight,
}

impl ColorScheme {
    /// Reads the portal's integer.
    ///
    /// **1 is dark and 2 is light**, which is the order the freedesktop
    /// specification gives and not the order anyone guesses. Getting it
    /// backwards produces a launcher that is dark for exactly the users who
    /// asked for light, so the mapping has a test of its own.
    ///
    /// An unrecognised value is `NoPreference` rather than an error: the
    /// specification reserves room to grow, and a desktop that reports
    /// something newer should get a readable launcher rather than none.
    #[must_use]
    pub const fn from_portal(value: u32) -> Self {
        match value {
            1 => Self::PreferDark,
            2 => Self::PreferLight,
            _ => Self::NoPreference,
        }
    }

    /// Which appearance to draw.
    ///
    /// No preference means light, because that is Adwaita's default and the
    /// launcher should look like the desktop it is sitting on rather than
    /// announce itself.
    #[must_use]
    pub const fn appearance(self) -> Appearance {
        match self {
            Self::PreferDark => Appearance::Dark,
            Self::NoPreference | Self::PreferLight => Appearance::Light,
        }
    }
}

/// Which of the two palettes is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Adwaita light.
    Light,
    /// Adwaita dark.
    Dark,
}

impl Appearance {
    /// Both, for the design page and for screenshot sweeps.
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    /// The name used in filenames and in the page's toggle.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// One colour, as 8-bit sRGB.
///
/// Stored as bytes rather than floats because these are transcribed from
/// GNOME's published Adwaita values, which are written as hex, and a float
/// round-trip makes them unrecognisable to anyone checking them against the
/// source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// A colour from its hex components.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#rrggbb`, for CSS and for reading in a diff.
    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// As an Iced colour.
    #[must_use]
    pub fn to_iced(self) -> iced::Color {
        iced::Color::from_rgb8(self.r, self.g, self.b)
    }
}

/// The colours one appearance draws with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Palette {
    /// The card's fill.
    pub surface: Rgb,
    /// The search field's fill, one step from the card.
    pub field: Rgb,
    /// Ordinary text.
    pub text: Rgb,
    /// Subtitles and headings.
    pub muted: Rgb,
    /// The selected row's fill.
    pub selection: Rgb,
    /// Text on the selected row.
    pub selection_text: Rgb,
    /// Hairlines: the card's edge, dividers.
    pub border: Rgb,
    /// The accent, for focus rings and shortcut chips.
    pub accent: Rgb,
    /// What is drawn behind the card, over the desktop.
    pub backdrop: Rgb,
    /// How opaque that backdrop is, 0–1.
    pub backdrop_alpha: f32,
}

/// Adwaita light.
///
/// Transcribed from GNOME's named palette: `@window_bg_color` #fafafa,
/// `@view_bg_color` #ffffff, `@accent_bg_color` #3584e4.
pub const LIGHT: Palette = Palette {
    surface: Rgb::new(0xfa, 0xfa, 0xfa),
    field: Rgb::new(0xff, 0xff, 0xff),
    text: Rgb::new(0x1e, 0x1e, 0x1e),
    muted: Rgb::new(0x5e, 0x5c, 0x64),
    selection: Rgb::new(0x35, 0x84, 0xe4),
    selection_text: Rgb::new(0xff, 0xff, 0xff),
    border: Rgb::new(0xd8, 0xd8, 0xd4),
    accent: Rgb::new(0x35, 0x84, 0xe4),
    backdrop: Rgb::new(0x00, 0x00, 0x00),
    backdrop_alpha: 0.25,
};

/// Adwaita dark.
///
/// `@window_bg_color` #242424, `@view_bg_color` #1e1e1e, and the lighter
/// `@accent_bg_color` #3584e4 keeps its hue across both because GNOME's does.
pub const DARK: Palette = Palette {
    surface: Rgb::new(0x24, 0x24, 0x24),
    field: Rgb::new(0x1e, 0x1e, 0x1e),
    text: Rgb::new(0xff, 0xff, 0xff),
    muted: Rgb::new(0x9a, 0x99, 0x96),
    selection: Rgb::new(0x35, 0x84, 0xe4),
    selection_text: Rgb::new(0xff, 0xff, 0xff),
    border: Rgb::new(0x3d, 0x3d, 0x3d),
    accent: Rgb::new(0x62, 0xa0, 0xea),
    backdrop: Rgb::new(0x00, 0x00, 0x00),
    backdrop_alpha: 0.35,
};

/// The palette for an appearance.
#[must_use]
pub const fn palette(appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Light => LIGHT,
        Appearance::Dark => DARK,
    }
}

/// Sizes and spacing, in logical pixels.
///
/// A Search Light-shaped card: a fixed width rather than a fraction of the
/// screen, because a launcher that grows with the monitor is unreadable on a
/// wide one, and centred horizontally but held high vertically, because the
/// eye starts above the middle and the results grow downward.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Geometry {
    /// The card's width.
    pub card_width: u16,
    /// The tallest the card grows before the list scrolls.
    pub card_max_height: u16,
    /// The card's corner radius.
    pub card_radius: u16,
    /// How far down the screen the card's top sits, as a fraction of height.
    pub card_top_fraction: f32,
    /// Padding inside the card.
    pub card_padding: u16,
    /// The search field's height.
    pub field_height: u16,
    /// The search field's corner radius.
    pub field_radius: u16,
    /// A result row's height.
    pub row_height: u16,
    /// A result row's corner radius when selected.
    pub row_radius: u16,
    /// The gap between rows.
    pub row_spacing: u16,
    /// The icon's edge length.
    pub icon_size: u16,
    /// Title text size.
    pub title_size: u16,
    /// Subtitle text size.
    pub subtitle_size: u16,
    /// Section heading text size.
    pub heading_size: u16,
    /// Search field text size.
    pub query_size: u16,
}

/// The shipped geometry.
pub const GEOMETRY: Geometry = Geometry {
    card_width: 720,
    card_max_height: 560,
    card_radius: 16,
    card_top_fraction: 0.18,
    card_padding: 8,
    field_height: 56,
    field_radius: 12,
    row_height: 48,
    row_radius: 10,
    row_spacing: 2,
    icon_size: 32,
    title_size: 15,
    subtitle_size: 12,
    heading_size: 11,
    query_size: 20,
};

/// The font stack, most preferred first.
///
/// Cantarell is GNOME's interface font; the rest are what a non-GNOME desktop
/// running this is likely to have. The browser surrogate is given the same
/// list so that text metrics are close enough for layout decisions to carry.
pub const FONT_STACK: &[&str] = &["Cantarell", "Adwaita Sans", "Inter", "Cantarell Regular"];

/// An Iced theme for an appearance.
#[must_use]
pub fn theme(appearance: Appearance) -> iced::Theme {
    let p = palette(appearance);

    iced::Theme::custom(
        format!("Compass {}", appearance.name()),
        iced::theme::Palette {
            background: p.surface.to_iced(),
            text: p.text.to_iced(),
            primary: p.accent.to_iced(),
            success: p.accent.to_iced(),
            warning: p.accent.to_iced(),
            danger: iced::Color::from_rgb8(0xe0, 0x1b, 0x24),
        },
    )
}

/// How opaque the card is when `tint` is on (#86).
///
/// 0.82 rather than something lower: the launcher is text over an arbitrary
/// wallpaper, and legibility is not negotiable for the thing bound to
/// Super+Space. Enough to read as translucent over a busy background, opaque
/// enough that body text keeps its contrast over a bright one.
pub const TINT_ALPHA: f32 = 0.82;
