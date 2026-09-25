//! `src/lib/glyph/tests`, ported: which strings are an emoji, and skin tones.

use compass_core::emoji_grid::{SkinTone, apply_skin_tone};
use compass_core::glyph::is_emoji;

// --- special-cases.cpp ---------------------------------------------------

#[test]
fn a_zwj_family_is_one_emoji() {
    assert!(is_emoji("👨‍👩‍👧‍👦"));
}

#[test]
fn a_zwj_person_with_a_computer_is_one_emoji() {
    assert!(is_emoji("👨‍💻"));
}

#[test]
fn a_skin_tone_variation_is_still_an_emoji() {
    assert!(is_emoji("👋🏽"));
    assert!(is_emoji("👍🏿"));
}

#[test]
fn a_flag_sequence_is_an_emoji() {
    assert!(is_emoji("🇺🇸"));
    assert!(is_emoji("🇫🇷"));
}

#[test]
fn a_keycap_sequence_is_an_emoji() {
    assert!(is_emoji("1️⃣"));
    assert!(is_emoji("#️⃣"));
}

#[test]
fn an_emoji_with_a_variation_selector_is_an_emoji() {
    assert!(is_emoji("❤️"));
    assert!(is_emoji("☺️"));
}

#[test]
fn plain_text_is_not_an_emoji() {
    assert!(!is_emoji("hello"));
    assert!(!is_emoji("123"));
}

#[test]
fn text_with_an_emoji_in_it_is_not_an_emoji() {
    assert!(!is_emoji("hello 😀"));
}

// --- beyond the C++ cases, for what an icon string actually is ----------

#[test]
fn a_toned_zwj_sequence_is_one_emoji() {
    assert!(is_emoji("💆🏿‍♂"));
}

#[test]
fn two_emoji_side_by_side_are_not_one() {
    assert!(!is_emoji("😀😀"));
}

#[test]
fn paths_urls_and_nothing_are_not_emoji() {
    assert!(!is_emoji(""));
    assert!(!is_emoji("\u{FE0F}"));
    assert!(!is_emoji("icon.png"));
    assert!(!is_emoji("https://example.com/a.png"));
    assert!(!is_emoji("→"), "a symbol from the picker is not an emoji");
}

// --- tones.cpp -----------------------------------------------------------

#[test]
fn every_tone_applies() {
    assert_eq!(apply_skin_tone("👋", SkinTone::Light), "👋🏻");
    assert_eq!(apply_skin_tone("👋", SkinTone::MediumLight), "👋🏼");
    assert_eq!(apply_skin_tone("👋", SkinTone::Medium), "👋🏽");
    assert_eq!(apply_skin_tone("👋", SkinTone::MediumDark), "👋🏾");
    assert_eq!(apply_skin_tone("👋", SkinTone::Dark), "👋🏿");
}

#[test]
fn applying_a_tone_strips_the_variation_selector() {
    assert_eq!(apply_skin_tone("🖐️", SkinTone::Dark), "🖐🏿");
    assert_eq!(apply_skin_tone("🖐️", SkinTone::Medium), "🖐🏽");
}

#[test]
fn applying_a_tone_to_a_zwj_sequence_strips_the_variation_selector() {
    assert_eq!(apply_skin_tone("💆‍♂️", SkinTone::Dark), "💆🏿‍♂");
    assert_eq!(apply_skin_tone("💆‍♂️", SkinTone::Light), "💆🏻‍♂");
}
