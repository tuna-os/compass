//! Named appearance presets (#84).
//!
//! Nobody thinks in individual toggles — they think "make it look like
//! Raycast". So the appearance options are reachable as a small set of named
//! presets, with the individual knobs still available underneath: a preset
//! supplies the defaults for everything in `launcher.appearance`, and any key
//! written explicitly alongside it wins.
//!
//! # What a preset is, and what it is not
//!
//! A preset is a [`design::Geometry`] plus a handful of structural flags. It is
//! **not** a transcription of another product's colours. We have no source for
//! Raycast's or Flow's exact palettes, and inventing hex values while calling
//! them by a product's name would be a claim this repository cannot support.
//! What the reference images in
//! [#84](https://github.com/tuna-os/compass/issues/84#issuecomment-5722088525)
//! actually establish is that those launchers differ from each other in
//! *shape* — corner radius, row height, spacing, whether the selection is an
//! inset pill or a full-width band — over one identical widget tree. That is
//! what varies here, on top of the Adwaita-derived palette the desktop gives
//! us.
//!
//! `rofi` is the exception the reference grid itself flags: it is genuinely
//! denser rather than differently coloured.

use crate::design::{self, Geometry};

/// The shipped presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Spotlight-simple and recognisably GNOME: the default.
    #[default]
    Gnome,
    /// Roomier rows, softer corners, icons on.
    Raycast,
    /// Tight radius, a rule under the field, full-width band selection.
    Flow,
    /// Dense, sharp, minimal chrome.
    Rofi,
}

/// The name written in `launcher.appearance.preset` for each preset.
pub const NAMES: [(&str, Preset); 4] = [
    ("gnome", Preset::Gnome),
    ("raycast", Preset::Raycast),
    ("flow", Preset::Flow),
    ("rofi", Preset::Rofi),
];

impl Preset {
    /// The preset a configuration names, if it names one this build knows.
    ///
    /// Case-insensitive, because a config file is written by hand. An
    /// unrecognised name yields `None` rather than an error: `vicinae.json`'s
    /// contract is that a value this build does not understand is survivable,
    /// and refusing to start the launcher over a misspelt theme name would be
    /// the wrong trade. [`resolve`] falls back to the default and the caller
    /// can say so.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.trim().to_ascii_lowercase();
        NAMES
            .iter()
            .find(|(candidate, _)| *candidate == lowered)
            .map(|(_, preset)| *preset)
    }

    /// The name this preset is written as.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Preset::Gnome => "gnome",
            Preset::Raycast => "raycast",
            Preset::Flow => "flow",
            Preset::Rofi => "rofi",
        }
    }

    /// The geometry this preset draws with.
    ///
    /// `card_width` and `card_top_fraction` are deliberately the same in all
    /// four: where the window sits and how wide it is are not what
    /// distinguishes these looks, and varying them would move the VM tier's
    /// containment box for no gain.
    #[must_use]
    pub const fn geometry(self) -> Geometry {
        let base = design::GEOMETRY;
        match self {
            // Exactly what ships today. Stated as the base rather than as
            // copied numbers so the two cannot drift.
            Preset::Gnome => base,

            Preset::Raycast => Geometry {
                card_radius: 20,
                card_padding: 12,
                field_height: 60,
                field_radius: 14,
                row_height: 56,
                row_radius: 12,
                row_spacing: 4,
                ..base
            },

            // Small radius and a full-width band rather than an inset pill,
            // per the reference images.
            Preset::Flow => Geometry {
                card_radius: 6,
                card_padding: 0,
                field_radius: 0,
                row_radius: 0,
                row_spacing: 0,
                ..base
            },

            // The dense end. Sharp corners, no gaps, smaller type.
            Preset::Rofi => Geometry {
                card_radius: 0,
                card_padding: 4,
                field_height: 40,
                field_radius: 0,
                row_height: 30,
                row_radius: 0,
                row_spacing: 0,
                icon_size: 20,
                title_size: 13,
                subtitle_size: 11,
                query_size: 16,
                ..base
            },
        }
    }

    /// Whether this preset shows result icons (#85).
    #[must_use]
    pub const fn icons(self) -> bool {
        match self {
            // "Spotlight-simple: no icons" — #83's stated default.
            Preset::Gnome | Preset::Rofi => false,
            Preset::Raycast | Preset::Flow => true,
        }
    }

    /// Whether a full-width hairline rule separates the field from the results.
    ///
    /// Flow's most legible structural trait after the band selection.
    #[must_use]
    pub const fn field_rule(self) -> bool {
        matches!(self, Preset::Flow)
    }
}

/// An appearance, resolved from a preset and the keys written alongside it.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Which preset supplied the defaults.
    pub preset: Preset,
    /// Sizes and spacing.
    pub geometry: Geometry,
    /// Whether result rows show icons.
    pub icons: bool,
    /// Whether a rule separates the field from the results.
    pub field_rule: bool,
    /// The name that was written but not recognised, if one was.
    ///
    /// Carried rather than discarded so the caller can warn. Silently drawing
    /// the default for a name the user typed would leave them adjusting a
    /// setting that is not being read.
    pub unknown_name: Option<String>,
}

/// Resolve an appearance.
///
/// `preset` is `launcher.appearance.preset`; `icons` is the explicit
/// `launcher.appearance.icons`, which wins over whatever the preset says.
#[must_use]
pub fn resolve(preset: Option<&str>, icons: Option<bool>) -> Resolved {
    let named = preset.map(str::trim).filter(|name| !name.is_empty());
    let found = named.and_then(Preset::from_name);
    let unknown_name = match (named, found) {
        (Some(name), None) => Some(name.to_owned()),
        _ => None,
    };
    let preset = found.unwrap_or_default();

    Resolved {
        preset,
        geometry: preset.geometry(),
        icons: icons.unwrap_or_else(|| preset.icons()),
        field_rule: preset.field_rule(),
        unknown_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_gnome_and_it_is_exactly_what_ships() {
        // Not "looks like" what ships: the same values. A preset layer that
        // shifted the default by a pixel would move the VM tier's containment
        // box, and #84 is explicit that the box's numbers come from
        // measurement and must not be widened to fit.
        let resolved = resolve(None, None);
        assert_eq!(resolved.preset, Preset::Gnome);
        assert_eq!(
            format!("{:?}", resolved.geometry),
            format!("{:?}", design::GEOMETRY)
        );
        assert!(!resolved.icons, "#83 specifies gnome as no icons");
        assert!(!resolved.field_rule);
    }

    #[test]
    fn every_preset_resolves_to_its_own_values() {
        // One case per preset, so a preset that silently stops setting
        // something fails here rather than quietly reverting to the default.
        let gnome = resolve(Some("gnome"), None);
        let raycast = resolve(Some("raycast"), None);
        let flow = resolve(Some("flow"), None);
        let rofi = resolve(Some("rofi"), None);

        assert_eq!(gnome.geometry.row_height, design::GEOMETRY.row_height);
        assert!(!gnome.icons);

        assert_eq!(raycast.geometry.row_height, 56);
        assert_eq!(raycast.geometry.card_radius, 20);
        assert!(raycast.icons, "roomier rows and icons are the raycast look");
        assert!(!raycast.field_rule);

        assert_eq!(flow.geometry.card_radius, 6);
        assert_eq!(flow.geometry.row_radius, 0, "band selection, not a pill");
        assert!(flow.icons);
        assert!(flow.field_rule, "the hairline rule under the field");

        assert_eq!(rofi.geometry.row_height, 30);
        assert_eq!(rofi.geometry.card_radius, 0, "sharp corners");
        assert_eq!(rofi.geometry.row_spacing, 0);
        assert!(!rofi.icons, "keyboard-first and minimal");
    }

    #[test]
    fn no_two_presets_are_the_same_look() {
        // Cheap, and it is the assertion that catches a copy-paste preset.
        let shapes: Vec<String> = NAMES
            .iter()
            .map(|(name, _)| {
                let r = resolve(Some(name), None);
                format!("{:?}|{}|{}", r.geometry, r.icons, r.field_rule)
            })
            .collect();
        for (i, a) in shapes.iter().enumerate() {
            for (j, b) in shapes.iter().enumerate() {
                assert!(
                    i == j || a != b,
                    "{} and {} resolve identically",
                    NAMES[i].0,
                    NAMES[j].0
                );
            }
        }
    }

    #[test]
    fn an_explicit_key_beats_the_preset() {
        // The whole point of presets-with-knobs: a preset is defaults, not a
        // lock.
        assert!(
            !resolve(Some("raycast"), Some(false)).icons,
            "raycast turns icons on, but the user said off"
        );
        assert!(
            resolve(Some("gnome"), Some(true)).icons,
            "gnome turns icons off, but the user said on"
        );
    }

    #[test]
    fn a_name_is_matched_whatever_its_case_or_padding() {
        // A config file is written by hand.
        for name in ["Raycast", "RAYCAST", "  raycast  "] {
            assert_eq!(
                resolve(Some(name), None).preset,
                Preset::Raycast,
                "{name:?}"
            );
        }
    }

    #[test]
    fn an_unrecognised_name_draws_the_default_and_says_so() {
        // Survivable, per `vicinae.json`'s contract -- refusing to start over
        // a misspelt theme name would be the wrong trade. But not silent:
        // drawing the default without a word leaves someone adjusting a
        // setting nothing is reading.
        let resolved = resolve(Some("dracula"), None);
        assert_eq!(resolved.preset, Preset::Gnome);
        assert_eq!(resolved.unknown_name.as_deref(), Some("dracula"));
    }

    #[test]
    fn an_absent_or_empty_name_is_not_an_unknown_one() {
        // Nothing was written, so there is nothing to warn about.
        assert_eq!(resolve(None, None).unknown_name, None);
        assert_eq!(resolve(Some(""), None).unknown_name, None);
        assert_eq!(resolve(Some("   "), None).unknown_name, None);
    }

    #[test]
    fn every_preset_round_trips_through_its_name() {
        for (name, preset) in NAMES {
            assert_eq!(Preset::from_name(name), Some(preset));
            assert_eq!(preset.name(), name);
        }
    }
}
