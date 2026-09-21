//! Curated color themes for #153.
//!
//! Each entry is a full [`Palette`] so the browser surrogate can render it
//! from `states.json` without retyping colours. `system` is not a palette —
//! it means "follow the portal appearance with Adwaita colours".

use crate::design::{Appearance, Palette, Rgb};

/// A named theme from the #153 assortment.
///
/// The string forms are what `launcher.appearance.theme` persists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Follow the desktop (Adwaita light/dark).
    #[default]
    System,
    /// Catppuccin Mocha (dark) / Latte (light).
    Catppuccin,
    /// Dracula — dark only, light falls back to a light variant.
    Dracula,
    /// Nord — dark and light.
    Nord,
    /// Gruvbox — dark and light.
    Gruvbox,
    /// Tokyo Night — dark (Storm) / light.
    TokyoNight,
    /// Solarized — dark / light.
    Solarized,
}

impl Theme {
    /// All curated names, for the picker and for `states.json`.
    pub const ALL: [Self; 7] = [
        Self::System,
        Self::Catppuccin,
        Self::Dracula,
        Self::Nord,
        Self::Gruvbox,
        Self::TokyoNight,
        Self::Solarized,
    ];

    /// The persisted spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Catppuccin => "catppuccin",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
            Self::TokyoNight => "tokyo-night",
            Self::Solarized => "solarized",
        }
    }

    /// Parse the persisted spelling, case-insensitive.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "catppuccin" | "catppuccin-mocha" | "catppuccin-latte" => Some(Self::Catppuccin),
            "dracula" => Some(Self::Dracula),
            "nord" => Some(Self::Nord),
            "gruvbox" => Some(Self::Gruvbox),
            "tokyo-night" | "tokyonight" | "tokyo_night" => Some(Self::TokyoNight),
            "solarized" => Some(Self::Solarized),
            _ => None,
        }
    }

    /// Palette for this theme at the given appearance.
    ///
    /// System returns the Adwaita palette; the others return curated palettes
    /// with readable contrast for selection, focus, muted and error states.
    #[must_use]
    pub fn palette(self, appearance: Appearance) -> Palette {
        match self {
            Self::System => crate::design::palette(appearance),
            Self::Catppuccin => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x1e, 0x1e, 0x2e), // Mocha base
                    field: Rgb::new(0x31, 0x32, 0x44),   // surface0
                    text: Rgb::new(0xcd, 0xd6, 0xf4),
                    muted: Rgb::new(0xa6, 0xad, 0xc8), // subtext0
                    selection: Rgb::new(0x89, 0xb4, 0xfa), // blue
                    selection_text: Rgb::new(0x1e, 0x1e, 0x2e),
                    border: Rgb::new(0x45, 0x47, 0x5a), // surface1
                    accent: Rgb::new(0x89, 0xb4, 0xfa),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xef, 0xf1, 0xf5), // Latte base
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x4c, 0x4f, 0x69),
                    muted: Rgb::new(0x8c, 0x8f, 0xa1),
                    selection: Rgb::new(0x1e, 0x66, 0xf5),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xcc, 0xcd, 0xd6),
                    accent: Rgb::new(0x1e, 0x66, 0xf5),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Dracula => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x28, 0x2a, 0x36),
                    field: Rgb::new(0x44, 0x47, 0x5a),
                    text: Rgb::new(0xf8, 0xf8, 0xf2),
                    muted: Rgb::new(0xbd, 0x93, 0xf9), // purple muted
                    selection: Rgb::new(0xbd, 0x93, 0xf9),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0x62, 0x72, 0xa4), // comment
                    accent: Rgb::new(0xff, 0x79, 0xc6), // pink
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xf8, 0xf8, 0xf2),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x28, 0x2a, 0x36),
                    muted: Rgb::new(0x62, 0x72, 0xa4),
                    selection: Rgb::new(0xbd, 0x93, 0xf9),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xbd, 0x93, 0xf9),
                    accent: Rgb::new(0xff, 0x79, 0xc6),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Nord => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x2e, 0x34, 0x40), // polar night 0
                    field: Rgb::new(0x3b, 0x42, 0x52),   // polar night 1
                    text: Rgb::new(0xec, 0xef, 0xf4),   // snow storm 2
                    muted: Rgb::new(0x81, 0xa1, 0xc1),   // frost
                    selection: Rgb::new(0x88, 0xc0, 0xd0),
                    selection_text: Rgb::new(0x2e, 0x34, 0x40),
                    border: Rgb::new(0x4c, 0x56, 0x6a),
                    accent: Rgb::new(0x81, 0xa1, 0xc1),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xec, 0xef, 0xf4),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x2e, 0x34, 0x40),
                    muted: Rgb::new(0x4c, 0x56, 0x6a),
                    selection: Rgb::new(0x5e, 0x81, 0xac),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xd8, 0xde, 0xe9),
                    accent: Rgb::new(0x5e, 0x81, 0xac),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Gruvbox => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x28, 0x28, 0x28), // bg0
                    field: Rgb::new(0x3c, 0x38, 0x36),   // bg1
                    text: Rgb::new(0xeb, 0xdb, 0xb2),   // fg
                    muted: Rgb::new(0xa8, 0x99, 0x84),   // gray
                    selection: Rgb::new(0xfb, 0x49, 0x34), // red
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0x50, 0x49, 0x45), // bg2
                    accent: Rgb::new(0xfe, 0x80, 0x19), // orange
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xfb, 0xf1, 0xc7), // bg0 light
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x3c, 0x38, 0x36),
                    muted: Rgb::new(0x7c, 0x6f, 0x64),
                    selection: Rgb::new(0xcc, 0x24, 0x1d),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xd5, 0xc4, 0xa1),
                    accent: Rgb::new(0xd6, 0x5d, 0x0e),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::TokyoNight => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x1a, 0x1b, 0x26), // Storm
                    field: Rgb::new(0x24, 0x28, 0x3b),
                    text: Rgb::new(0xc0, 0xca, 0xf5),
                    muted: Rgb::new(0x56, 0x5f, 0x89),
                    selection: Rgb::new(0x7a, 0xa2, 0xf7), // blue
                    selection_text: Rgb::new(0x1a, 0x1b, 0x26),
                    border: Rgb::new(0x29, 0x2e, 0x42),
                    accent: Rgb::new(0x7a, 0xa2, 0xf7),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xd5, 0xd6, 0xdb),
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x1a, 0x1b, 0x26),
                    muted: Rgb::new(0x56, 0x5f, 0x89),
                    selection: Rgb::new(0x34, 0x5c, 0xb2),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xcb, 0xcc, 0xd1),
                    accent: Rgb::new(0x34, 0x5c, 0xb2),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
            Self::Solarized => match appearance {
                Appearance::Dark => Palette {
                    surface: Rgb::new(0x00, 0x2b, 0x36), // base03
                    field: Rgb::new(0x07, 0x36, 0x42),   // base02
                    text: Rgb::new(0x83, 0x94, 0x96),   // base0
                    muted: Rgb::new(0x58, 0x6e, 0x75),   // base01
                    selection: Rgb::new(0x26, 0x8b, 0xd2), // blue
                    selection_text: Rgb::new(0xfd, 0xf6, 0xe3),
                    border: Rgb::new(0x07, 0x36, 0x42),
                    accent: Rgb::new(0x2a, 0xa1, 0x98), // cyan
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.35,
                },
                Appearance::Light => Palette {
                    surface: Rgb::new(0xfd, 0xf6, 0xe3), // base3
                    field: Rgb::new(0xff, 0xff, 0xff),
                    text: Rgb::new(0x65, 0x7b, 0x83), // base00
                    muted: Rgb::new(0x93, 0xa1, 0xa1), // base1
                    selection: Rgb::new(0x26, 0x8b, 0xd2),
                    selection_text: Rgb::new(0xff, 0xff, 0xff),
                    border: Rgb::new(0xee, 0xe8, 0xd5), // base2
                    accent: Rgb::new(0x2a, 0xa1, 0x98),
                    backdrop: Rgb::new(0x00, 0x00, 0x00),
                    backdrop_alpha: 0.20,
                },
            },
        }
    }
}


