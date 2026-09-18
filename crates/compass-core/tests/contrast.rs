//! What counts as readable against a given background.
//!
//! Read off `ContrastHelper` (`src/server/src/ui/image/contrast-helper.hpp`)
//! and, for the luminance and ratio, WCAG 2.x itself.

use compass_core::contrast::{
    DEFAULT_MIN_RATIO, Hsl, MAX_ITERATIONS, Rgb, contrast_ratio, from_hsl, needs_lighter,
    relative_luminance, to_hsl, tonal_contrast_color, tonal_contrast_color_with_amount,
};

const BLACK: Rgb = Rgb::new(0, 0, 0);
const WHITE: Rgb = Rgb::new(255, 255, 255);

/// Close enough for floating point.
fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

#[test]
fn the_default_ratio_is_wcag_aa() {
    assert_eq!(DEFAULT_MIN_RATIO, 4.5);
    assert_eq!(MAX_ITERATIONS, 30);
}

#[test]
fn black_and_white_have_the_luminances_the_standard_gives() {
    assert!(near(relative_luminance(BLACK), 0.0));
    assert!(near(relative_luminance(WHITE), 1.0));
}

#[test]
fn the_channel_coefficients_are_the_wcag_ones() {
    // Green dominates because the eye does: a pure green is far brighter to
    // look at than a pure blue of the same value.
    assert!(near(relative_luminance(Rgb::new(255, 0, 0)), 0.2126));
    assert!(near(relative_luminance(Rgb::new(0, 255, 0)), 0.7152));
    assert!(near(relative_luminance(Rgb::new(0, 0, 255)), 0.0722));
}

#[test]
fn the_linearisation_has_a_knee_near_zero() {
    // Below 0.03928 the curve is linear; the split is what makes very dark
    // colours' luminances come out sensibly rather than crushed.
    let low = relative_luminance(Rgb::new(10, 10, 10));
    let high = relative_luminance(Rgb::new(11, 11, 11));
    assert!(low < high, "still monotonic across the knee");
    assert!(low > 0.0);
}

#[test]
fn black_on_white_is_the_maximum_ratio() {
    // 21:1 exactly, which is the number the standard is built around.
    assert!(near(contrast_ratio(BLACK, WHITE), 21.0));
}

#[test]
fn a_colour_against_itself_is_one_to_one() {
    assert!(near(contrast_ratio(WHITE, WHITE), 1.0));
    assert!(near(
        contrast_ratio(Rgb::new(70, 130, 180), Rgb::new(70, 130, 180)),
        1.0
    ));
}

#[test]
fn the_ratio_is_symmetric() {
    // The lighter of the two is always on top, so the order of the arguments
    // cannot change the answer.
    let a = Rgb::new(30, 60, 90);
    let b = Rgb::new(200, 180, 160);
    assert!(near(contrast_ratio(a, b), contrast_ratio(b, a)));
}

#[test]
fn a_dark_background_wants_something_lighter() {
    assert!(needs_lighter(BLACK));
    assert!(needs_lighter(Rgb::new(20, 20, 60)));
}

#[test]
fn a_light_background_wants_something_darker() {
    assert!(!needs_lighter(WHITE));
    assert!(!needs_lighter(Rgb::new(240, 240, 200)));
}

#[test]
fn the_direction_is_decided_by_luminance_not_by_lightness() {
    // A saturated yellow and a saturated blue have the same HSL lightness and
    // nothing like the same brightness, so the threshold has to be on the
    // luminance or one of them gets an unreadable answer.
    let yellow = Rgb::new(255, 255, 0);
    let blue = Rgb::new(0, 0, 255);
    assert_eq!(to_hsl(yellow).lightness, to_hsl(blue).lightness);
    assert!(!needs_lighter(yellow), "yellow is bright");
    assert!(needs_lighter(blue), "blue is not");
}

#[test]
fn a_grey_has_no_hue_and_stays_grey() {
    // Qt reports -1 for an achromatic colour; feeding that back must give a
    // grey again rather than red.
    let grey = to_hsl(Rgb::new(128, 128, 128));
    assert_eq!(grey.hue, None);
    assert_eq!(grey.saturation, 0);

    let back = from_hsl(grey);
    assert_eq!(back.r, back.g);
    assert_eq!(back.g, back.b);
}

#[test]
fn hsl_round_trips_for_saturated_colours() {
    for color in [
        Rgb::new(255, 0, 0),
        Rgb::new(0, 255, 0),
        Rgb::new(0, 0, 255),
        Rgb::new(70, 130, 180),
        Rgb::new(200, 100, 50),
    ] {
        let back = from_hsl(to_hsl(color));
        let diff = |a: u8, b: u8| i32::from(a).abs_diff(i32::from(b).unsigned_abs() as i32);
        assert!(
            diff(back.r, color.r) <= 2 && diff(back.g, color.g) <= 2 && diff(back.b, color.b) <= 2,
            "{color:?} round-tripped to {back:?}"
        );
    }
}

#[test]
fn black_and_white_round_trip_exactly() {
    assert_eq!(from_hsl(to_hsl(BLACK)), BLACK);
    assert_eq!(from_hsl(to_hsl(WHITE)), WHITE);
}

#[test]
fn a_readable_colour_is_found_for_a_dark_background() {
    let derived = tonal_contrast_color(Rgb::new(20, 20, 60), DEFAULT_MIN_RATIO);
    assert!(
        contrast_ratio(Rgb::new(20, 20, 60), derived) >= DEFAULT_MIN_RATIO,
        "{derived:?} is not readable"
    );
    assert!(
        relative_luminance(derived) > relative_luminance(Rgb::new(20, 20, 60)),
        "it went lighter"
    );
}

#[test]
fn a_background_the_first_guess_does_not_clear_is_stepped_towards() {
    // The starting point is lightness 200, which already clears 4.5 against a
    // very dark background — so a test using one never exercises the search at
    // all. A mid-dark background starts at about 2.5 and has to be walked up.
    let background = Rgb::new(100, 100, 140);
    let first_guess = from_hsl(Hsl {
        hue: to_hsl(background).hue,
        saturation: to_hsl(background).saturation.max(150),
        lightness: 200,
    });
    assert!(
        contrast_ratio(background, first_guess) < DEFAULT_MIN_RATIO,
        "this background must not be readable on the first guess, or the test proves nothing"
    );

    let derived = tonal_contrast_color(background, DEFAULT_MIN_RATIO);
    assert!(contrast_ratio(background, derived) >= DEFAULT_MIN_RATIO);
    assert!(
        relative_luminance(derived) > relative_luminance(first_guess),
        "the search has to have moved past where it started"
    );
}

#[test]
fn a_readable_colour_is_found_for_a_light_background() {
    let background = Rgb::new(240, 230, 200);
    let derived = tonal_contrast_color(background, DEFAULT_MIN_RATIO);
    assert!(contrast_ratio(background, derived) >= DEFAULT_MIN_RATIO);
    assert!(
        relative_luminance(derived) < relative_luminance(background),
        "it went darker"
    );
}

#[test]
fn the_derived_colour_keeps_the_backgrounds_hue() {
    // The point of "tonal": text on an album cover should look like it belongs
    // to the cover, not like a system default dropped on top.
    let background = Rgb::new(20, 20, 120);
    let derived = tonal_contrast_color(background, DEFAULT_MIN_RATIO);
    let background_hue = to_hsl(background).hue.expect("saturated");
    let derived_hue = to_hsl(derived).hue.expect("still saturated");
    let apart = (i32::from(background_hue) - i32::from(derived_hue)).abs();
    assert!(
        apart.min(360 - apart) <= 5,
        "{background_hue} vs {derived_hue}"
    );
}

#[test]
fn a_mid_grey_background_gives_up_rather_than_looping() {
    // Nothing reaches 4.5 against a mid grey. The iteration bound is what
    // makes that terminate at all, and it must return a colour rather than
    // hanging or panicking.
    let background = Rgb::new(128, 128, 128);
    let derived = tonal_contrast_color(background, DEFAULT_MIN_RATIO);
    assert!(contrast_ratio(background, derived) > 1.0, "it still moved");
}

#[test]
fn a_lower_ratio_is_satisfied_sooner() {
    let background = Rgb::new(60, 60, 60);
    let strict = tonal_contrast_color(background, 7.0);
    let loose = tonal_contrast_color(background, 3.0);
    assert!(
        relative_luminance(strict) >= relative_luminance(loose),
        "a stricter ratio has to go at least as far"
    );
}

#[test]
fn the_amount_variant_desaturates_towards_plain_black_or_white() {
    // A caller asking for none of the background's colour should get something
    // near-neutral, which is what makes this the variant for body text.
    let background = Rgb::new(150, 30, 30);
    let colourful = tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 1.0);
    let neutral = tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 0.0);

    assert_eq!(to_hsl(neutral).saturation, 0);
    assert!(to_hsl(colourful).saturation > to_hsl(neutral).saturation);
}

#[test]
fn the_amount_is_clamped() {
    let background = Rgb::new(150, 30, 30);
    assert_eq!(
        tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 5.0),
        tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 1.0)
    );
    assert_eq!(
        tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, -1.0),
        tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 0.0)
    );
}

#[test]
fn the_amount_variant_also_reaches_the_ratio() {
    let background = Rgb::new(20, 20, 60);
    let derived = tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 0.5);
    assert!(contrast_ratio(background, derived) >= DEFAULT_MIN_RATIO);
}

#[test]
fn both_variants_move_the_same_way() {
    // They differ in how far and how finely they step, not in which direction,
    // so a caller swapping one for the other never gets text the wrong side of
    // the background.
    for background in [BLACK, WHITE, Rgb::new(20, 20, 60), Rgb::new(240, 230, 200)] {
        let plain = tonal_contrast_color(background, DEFAULT_MIN_RATIO);
        let amount = tonal_contrast_color_with_amount(background, DEFAULT_MIN_RATIO, 0.5);
        let background_luminance = relative_luminance(background);
        assert_eq!(
            relative_luminance(plain) > background_luminance,
            relative_luminance(amount) > background_luminance,
            "{background:?}"
        );
    }
}
