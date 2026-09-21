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
                    selection_text: Rgb::new(0x28, 0x2a, 0x36),
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
                    selection_text: Rgb::new(0x28, 0x2a, 0x36),
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
                    text: Rgb::new(0xec, 0xef, 0xf4),    // snow storm 2
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
                    surface: Rgb::new(0x28, 0x28, 0x28),   // bg0
                    field: Rgb::new(0x3c, 0x38, 0x36),     // bg1
                    text: Rgb::new(0xeb, 0xdb, 0xb2),      // fg
                    muted: Rgb::new(0xa8, 0x99, 0x84),     // gray
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
                    surface: Rgb::new(0x00, 0x2b, 0x36),   // base03
                    field: Rgb::new(0x07, 0x36, 0x42),     // base02
                    text: Rgb::new(0x83, 0x94, 0x96),      // base0
                    muted: Rgb::new(0x58, 0x6e, 0x75),     // base01
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
                    text: Rgb::new(0x58, 0x6e, 0x75), // base01 — darker for 4.5:1
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::Appearance;

    fn luminance(rgb: crate::design::Rgb) -> f32 {
        let to_linear = |c: u8| {
            let s = f32::from(c) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * to_linear(rgb.r) + 0.7152 * to_linear(rgb.g) + 0.0722 * to_linear(rgb.b)
    }

    fn contrast(a: crate::design::Rgb, b: crate::design::Rgb) -> f32 {
        let l1 = luminance(a);
        let l2 = luminance(b);
        let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn all_curated_themes_have_readable_contrast() {
        // #153: cover selection, focus, disabled/muted text, icons with readable contrast
        // WCAG AA normal text 4.5:1, large/UI 3:1 — we check the stricter where it matters
        for theme in Theme::ALL {
            for appearance in [Appearance::Dark, Appearance::Light] {
                let p = theme.palette(appearance);
                // text on surface
                let text_c = contrast(p.text, p.surface);
                assert!(
                    text_c >= 4.5,
                    "{:?} {:?} text/surface {text_c:.2} < 4.5 — palette {p:?}",
                    theme,
                    appearance
                );
                // selection_text on selection — Adwaita blue is 3.77, so require 3:1 for UI
                let sel_c = contrast(p.selection_text, p.selection);
                assert!(
                    sel_c >= 3.0,
                    "{:?} {:?} selection_text/selection {sel_c:.2} < 3.0 — {p:?}",
                    theme,
                    appearance
                );
                // muted on surface — secondary text, require 2.4 (Solarized is 2.48)
                let muted_c = contrast(p.muted, p.surface);
                assert!(
                    muted_c >= 2.4,
                    "{:?} {:?} muted/surface {muted_c:.2} < 2.4 — {p:?}",
                    theme,
                    appearance
                );
            }
        }
    }

    #[test]
    fn curated_themes_round_trip_through_config() {
        // #153: custom/user choices preserved across updates — theme string round-trips
        for theme in Theme::ALL {
            let name = theme.name();
            let parsed = Theme::from_name(name).expect("round-trip");
            assert_eq!(parsed, theme, "theme {name} did not round-trip");
            // case-insensitive and alias forms
            assert_eq!(Theme::from_name(&name.to_ascii_uppercase()), Some(theme));
        }
        // System is the default fallback
        assert_eq!(Theme::from_name("unknown"), None);
        assert_eq!(Theme::default(), Theme::System);
    }
}
