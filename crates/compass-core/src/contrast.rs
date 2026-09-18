//! Picking a colour that can actually be read against another one.
//!
//! A port of `ContrastHelper` (`src/server/src/ui/image/contrast-helper.hpp`).
//!
//! # The luminance maths is a standard; the search is a judgement
//!
//! [`relative_luminance`] and [`contrast_ratio`] are WCAG 2.x, defined to the
//! digit, and they are pinned against the values the standard itself gives.
//! [`tonal_contrast_color`] is not a standard — it is this project's answer to
//! "given an album cover, what colour do I write on it" — and what is pinned
//! there is the properties it has to hold: it moves away from the background,
//! it keeps the background's hue, and it gives up rather than looping.

/// The contrast ratio WCAG calls AA for body text, and this helper's default.
pub const DEFAULT_MIN_RATIO: f64 = 4.5;

/// How many times the search will step before giving up.
///
/// A bound, not a target: on a mid-grey background no lightness reaches 4.5
/// against it, and a loop without this would never end.
pub const MAX_ITERATIONS: u32 = 30;

/// A colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// A colour from its channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// A colour in hue, saturation and lightness, on Qt's scales.
///
/// Hue is 0..=359 degrees, or `None` for a grey — Qt reports -1 there, and
/// feeding that back to `fromHsl` gives a grey again rather than red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hsl {
    /// The hue in degrees, or `None` when the colour has none.
    pub hue: Option<u16>,
    /// Saturation, 0..=255 as Qt scales it.
    pub saturation: u8,
    /// Lightness, 0..=255.
    pub lightness: u8,
}

/// One channel's contribution to luminance, linearised.
fn linearize(value: u8) -> f64 {
    let value = f64::from(value) / 255.0;
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The WCAG relative luminance of `color`, from 0 to 1.
///
/// The green coefficient dominates because the eye does: a pure green is far
/// brighter to look at than a pure blue of the same value.
#[must_use]
pub fn relative_luminance(color: Rgb) -> f64 {
    0.0722f64.mul_add(
        linearize(color.b),
        0.2126f64.mul_add(linearize(color.r), 0.7152 * linearize(color.g)),
    )
}

/// The WCAG contrast ratio between two colours, from 1 to 21.
///
/// Symmetric, and never below 1: the lighter of the two is always on top.
#[must_use]
pub fn contrast_ratio(first: Rgb, second: Rgb) -> f64 {
    let a = relative_luminance(first);
    let b = relative_luminance(second);
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Convert to HSL on Qt's scales.
#[must_use]
pub fn to_hsl(color: Rgb) -> Hsl {
    let r = f64::from(color.r) / 255.0;
    let g = f64::from(color.g) / 255.0;
    let b = f64::from(color.b) / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;

    if (max - min).abs() < f64::EPSILON {
        return Hsl {
            hue: None,
            saturation: 0,
            lightness: (lightness * 255.0).round() as u8,
        };
    }

    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };

    let hue = if (max - r).abs() < f64::EPSILON {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } * 60.0;

    Hsl {
        hue: Some(hue.round() as u16 % 360),
        saturation: (saturation * 255.0).round() as u8,
        lightness: (lightness * 255.0).round() as u8,
    }
}

/// Convert back from HSL.
#[must_use]
pub fn from_hsl(hsl: Hsl) -> Rgb {
    let lightness = f64::from(hsl.lightness) / 255.0;
    let saturation = f64::from(hsl.saturation) / 255.0;

    let Some(hue) = hsl.hue else {
        let grey = (lightness * 255.0).round() as u8;
        return Rgb::new(grey, grey, grey);
    };

    if saturation <= 0.0 {
        let grey = (lightness * 255.0).round() as u8;
        return Rgb::new(grey, grey, grey);
    }

    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0f64.mul_add(lightness, -q);
    let h = f64::from(hue) / 360.0;

    let channel = |t: f64| -> u8 {
        let mut t = t;
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let value = if t < 1.0 / 6.0 {
            (q - p).mul_add(6.0 * t, p)
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            (q - p).mul_add(6.0f64.mul_add(2.0 / 3.0 - t, 0.0), p)
        } else {
            p
        };
        (value * 255.0).round() as u8
    };

    Rgb::new(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// Whether a colour written on `background` has to be lighter than it.
///
/// The threshold is on the *luminance*, not the lightness: a saturated yellow
/// and a saturated blue have the same HSL lightness and nothing like the same
/// brightness.
#[must_use]
pub fn needs_lighter(background: Rgb) -> bool {
    relative_luminance(background) < 0.5
}

/// A readable colour in the background's own hue.
///
/// Starts from a strong version of the background's hue — saturation at least
/// 150, lightness 200 or 50 — and walks the lightness away from the background
/// until the ratio is met. Near the top of the range it also desaturates,
/// because past a lightness of 240 there is nowhere lighter to go and dropping
/// the saturation is the only way left to gain contrast.
#[must_use]
pub fn tonal_contrast_color(background: Rgb, min_ratio: f64) -> Rgb {
    let lighter = needs_lighter(background);
    let hsl = to_hsl(background);

    let mut saturation = hsl.saturation.max(150);
    let mut lightness: i32 = if lighter { 200 } else { 50 };

    let mut derived = from_hsl(Hsl {
        hue: hsl.hue,
        saturation,
        lightness: lightness as u8,
    });

    let mut iterations = 0;
    while contrast_ratio(background, derived) < min_ratio && iterations < MAX_ITERATIONS {
        if lighter {
            lightness = (lightness + 5).min(255);
            if lightness > 240 {
                saturation = saturation.saturating_sub(2).max(50);
            }
        } else {
            lightness = (lightness - 5).max(0);
        }
        derived = from_hsl(Hsl {
            hue: hsl.hue,
            saturation,
            lightness: lightness as u8,
        });
        iterations += 1;
    }

    derived
}

/// A readable colour carrying only `color_amount` of the background's
/// saturation.
///
/// The same search with two differences: the saturation is scaled rather than
/// floored, so a caller can ask for something close to plain white or black,
/// and it starts further out (245 or 20) and steps more finely (3, not 5) —
/// because with less colour to work with the first guess is usually already
/// close and overshooting would waste the contrast on glare.
#[must_use]
pub fn tonal_contrast_color_with_amount(background: Rgb, min_ratio: f64, color_amount: f64) -> Rgb {
    let color_amount = color_amount.clamp(0.0, 1.0);
    let lighter = needs_lighter(background);
    let hsl = to_hsl(background);

    let saturation = (f64::from(hsl.saturation) * color_amount) as u8;
    let mut lightness: i32 = if lighter { 245 } else { 20 };

    let mut derived = from_hsl(Hsl {
        hue: hsl.hue,
        saturation,
        lightness: lightness as u8,
    });

    let mut iterations = 0;
    while contrast_ratio(background, derived) < min_ratio && iterations < MAX_ITERATIONS {
        if lighter {
            lightness = (lightness + 3).min(255);
        } else {
            lightness = (lightness - 3).max(0);
        }
        derived = from_hsl(Hsl {
            hue: hsl.hue,
            saturation,
            lightness: lightness as u8,
        });
        iterations += 1;
    }

    derived
}
