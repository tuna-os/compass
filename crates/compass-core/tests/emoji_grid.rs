//! The emoji and symbol picker.
//!
//! Ported from `src/server/src/builtins/vicinae/emoji-grid-model.cpp` and the
//! skin-tone arithmetic in `src/lib/glyph/src/emoji.cpp`.

use compass_core::emoji_grid::{
    CellIcon, GlyphKind, SKIN_TONES, SkinTone, TONE_SECTION_TITLE, VARIATION_SELECTOR_16,
    action_registers_visit, apply_skin_tone, cell_icon, copied_glyph, effective_tone,
    formatted_codepoint, main_actions, skin_tone_by_id, skin_tone_info, tone_options,
};

const WAVE: &str = "\u{1F44B}";
const WAVE_VS16: &str = "\u{1F44B}\u{FE0F}";
/// Man technologist: person, ZWJ, laptop.
const TECHNOLOGIST: &str = "\u{1F468}\u{200D}\u{1F4BB}";

// --- the tone table -----------------------------------------------------

#[test]
fn the_default_tone_carries_an_empty_modifier() {
    // Which is what makes applying it a no-op rather than a special case in
    // the applier.
    assert_eq!(skin_tone_info(SkinTone::Default).modifier, "");
}

#[test]
fn every_other_tone_carries_a_fitzpatrick_modifier() {
    for info in SKIN_TONES.iter().filter(|t| t.tone != SkinTone::Default) {
        assert_eq!(info.modifier.chars().count(), 1, "{}", info.id);
        let cp = info.modifier.chars().next().unwrap() as u32;
        assert!((0x1F3FB..=0x1F3FF).contains(&cp), "{} is {cp:X}", info.id);
    }
}

#[test]
fn the_table_holds_six_rows_in_the_cpp_order() {
    assert_eq!(SKIN_TONES.len(), 6);
    assert_eq!(SKIN_TONES[0].tone, SkinTone::Default);
    assert_eq!(SKIN_TONES[5].tone, SkinTone::Dark);
}

#[test]
fn the_six_names_are_pinned_as_written() {
    // Only one of them reaches a test through a label elsewhere, so the rest
    // would drift unnoticed.
    let names: Vec<&str> = SKIN_TONES.iter().map(|t| t.display_name).collect();
    assert_eq!(
        names,
        [
            "Default",
            "Light",
            "Medium Light",
            "Medium",
            "Medium Dark",
            "Dark"
        ]
    );
}

#[test]
fn the_six_ids_are_pinned_as_written() {
    // They are what gets stored, so a change here silently loses everyone's
    // saved preference.
    let ids: Vec<&str> = SKIN_TONES.iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        [
            "default",
            "light",
            "medium-light",
            "medium",
            "medium-dark",
            "dark"
        ]
    );
}

#[test]
fn tones_are_looked_up_by_their_stored_id() {
    assert_eq!(skin_tone_by_id("medium-light"), Some(SkinTone::MediumLight));
    assert_eq!(skin_tone_by_id("default"), Some(SkinTone::Default));
    assert_eq!(skin_tone_by_id("bright"), None);
}

// --- applying a tone ----------------------------------------------------

#[test]
fn the_default_tone_leaves_a_glyph_alone() {
    assert_eq!(apply_skin_tone(WAVE, SkinTone::Default), WAVE);
}

#[test]
fn a_tone_follows_the_glyph() {
    assert_eq!(
        apply_skin_tone(WAVE, SkinTone::Dark),
        format!("{WAVE}\u{1F3FF}")
    );
}

#[test]
fn the_tone_goes_after_the_first_codepoint_not_at_the_end() {
    // In a person-joined-to-an-object sequence the tone belongs to the person.
    let toned = apply_skin_tone(TECHNOLOGIST, SkinTone::Medium);
    assert_eq!(toned, "\u{1F468}\u{1F3FD}\u{200D}\u{1F4BB}");
    assert!(!toned.ends_with('\u{1F3FD}'));
}

#[test]
fn a_variation_selector_is_stripped_when_a_tone_is_applied() {
    // A tone modifier already forces the coloured presentation, and leaving
    // the selector in produces a sequence some fonts refuse to compose — the
    // glyph then breaks apart into its parts.
    let toned = apply_skin_tone(WAVE_VS16, SkinTone::Light);
    assert!(!toned.contains(VARIATION_SELECTOR_16));
    assert_eq!(toned, format!("{WAVE}\u{1F3FB}"));
}

#[test]
fn every_variation_selector_is_stripped_not_just_the_first() {
    let messy = format!("{WAVE}\u{FE0F}\u{200D}\u{1F4BB}\u{FE0F}");
    let toned = apply_skin_tone(&messy, SkinTone::Light);
    assert!(!toned.contains(VARIATION_SELECTOR_16));
}

#[test]
fn a_leading_variation_selector_is_kept_because_it_is_the_first_codepoint() {
    // The strip applies to what follows the first codepoint. Nothing in the
    // data starts with a selector, but the rule is worth pinning: the first
    // codepoint is never touched.
    let odd = format!("\u{FE0F}{WAVE}");
    assert!(apply_skin_tone(&odd, SkinTone::Light).starts_with(VARIATION_SELECTOR_16));
}

#[test]
fn an_empty_glyph_comes_back_empty() {
    assert_eq!(apply_skin_tone("", SkinTone::Dark), "");
}

// --- the codepoint label ------------------------------------------------

#[test]
fn a_codepoint_is_uppercase_hex_padded_to_four() {
    assert_eq!(formatted_codepoint("A"), "U+0041");
    assert_eq!(formatted_codepoint(WAVE), "U+1F44B");
}

#[test]
fn only_the_first_codepoint_of_a_sequence_is_reported() {
    // A sequence reports the codepoint of the thing it is a sequence *of*,
    // which is the number someone looking it up wants.
    assert_eq!(formatted_codepoint(TECHNOLOGIST), "U+1F468");
}

#[test]
fn the_padding_does_not_truncate_a_longer_codepoint() {
    assert_eq!(formatted_codepoint("\u{10FFFF}"), "U+10FFFF");
}

#[test]
fn an_empty_glyph_reports_a_zero_codepoint() {
    assert_eq!(formatted_codepoint(""), "U+0000");
}

// --- which tone is in force ---------------------------------------------

#[test]
fn a_glyphs_own_tone_beats_the_pickers() {
    // Someone who set a tone on one glyph meant that glyph.
    assert_eq!(
        effective_tone(Some(SkinTone::Dark), Some(SkinTone::Light)),
        SkinTone::Dark
    );
}

#[test]
fn the_pickers_tone_applies_where_a_glyph_has_none() {
    assert_eq!(effective_tone(None, Some(SkinTone::Light)), SkinTone::Light);
}

#[test]
fn with_neither_there_is_no_modifier() {
    assert_eq!(effective_tone(None, None), SkinTone::Default);
}

// --- what gets copied ---------------------------------------------------

#[test]
fn a_skinnable_glyph_is_copied_with_its_tone() {
    assert_eq!(
        copied_glyph(WAVE, true, SkinTone::Dark),
        format!("{WAVE}\u{1F3FF}")
    );
}

#[test]
fn a_glyph_that_takes_no_tone_is_copied_as_it_is() {
    // Applying a modifier to it would produce a sequence that renders as the
    // glyph followed by a coloured square.
    let cat = "\u{1F431}";
    assert_eq!(copied_glyph(cat, false, SkinTone::Dark), cat);
}

// --- the grid cell ------------------------------------------------------

#[test]
fn a_symbol_uses_a_different_scheme_from_an_emoji() {
    // It is drawn with the interface font: an arrow rendered as an emoji would
    // be the wrong size and weight beside its neighbours.
    assert_eq!(
        cell_icon("→", GlyphKind::Symbol, false, None, None),
        CellIcon::Symbol("→".to_owned())
    );
}

#[test]
fn a_symbol_ignores_skin_tones_entirely() {
    // The symbol branch returns before the tone is looked at.
    assert_eq!(
        cell_icon("→", GlyphKind::Symbol, true, Some(SkinTone::Dark), None),
        CellIcon::Symbol("→".to_owned())
    );
}

#[test]
fn a_skinnable_cell_is_drawn_in_its_effective_tone() {
    assert_eq!(
        cell_icon(WAVE, GlyphKind::Emoji, true, None, Some(SkinTone::Medium)),
        CellIcon::Emoji(format!("{WAVE}\u{1F3FD}"))
    );
}

#[test]
fn a_non_skinnable_cell_is_drawn_as_it_is() {
    let cat = "\u{1F431}";
    assert_eq!(
        cell_icon(cat, GlyphKind::Emoji, false, Some(SkinTone::Dark), None),
        CellIcon::Emoji(cat.to_owned())
    );
}

// --- the action panel ---------------------------------------------------

#[test]
fn copying_leads_by_default() {
    assert_eq!(main_actions(true, "copy", false)[0], "copy");
    assert_eq!(main_actions(true, "copy", false)[1], "paste");
}

#[test]
fn the_preference_reorders_the_first_two() {
    assert_eq!(main_actions(true, "paste", false)[0], "paste");
    assert_eq!(main_actions(true, "paste", false)[1], "copy");
}

#[test]
fn an_unset_preference_leads_with_copying() {
    assert_eq!(main_actions(true, "", false)[0], "copy");
}

#[test]
fn a_platform_that_cannot_paste_is_not_offered_it() {
    let actions = main_actions(false, "paste", false);
    assert!(!actions.contains(&"paste"));
    assert_eq!(actions[0], "copy");
}

#[test]
fn the_other_three_copies_are_always_offered() {
    let actions = main_actions(true, "copy", false);
    for id in ["copy-name", "copy-codepoint", "copy-category"] {
        assert!(actions.contains(&id), "missing {id}");
    }
}

#[test]
fn pinning_flips_with_the_state() {
    assert!(main_actions(true, "copy", false).contains(&"pin"));
    assert!(main_actions(true, "copy", true).contains(&"unpin"));
    assert!(!main_actions(true, "copy", true).contains(&"pin"));
}

#[test]
fn only_putting_the_glyph_somewhere_counts_as_using_it() {
    // Copying its name or its codepoint is looking something up. Counting it
    // would let a search for a name drift the picker's ordering.
    assert!(action_registers_visit("copy"));
    assert!(action_registers_visit("paste"));
    assert!(!action_registers_visit("copy-name"));
    assert!(!action_registers_visit("copy-codepoint"));
    assert!(!action_registers_visit("copy-category"));
    assert!(!action_registers_visit("pin"));
}

// --- the skin tone section ----------------------------------------------

#[test]
fn a_glyph_that_takes_no_tone_has_no_tone_section() {
    assert!(tone_options(WAVE, false, Some(SkinTone::Dark), None).is_empty());
}

#[test]
fn a_glyph_on_the_default_tone_is_offered_the_other_five() {
    let options = tone_options(WAVE, true, None, None);
    assert_eq!(options.len(), 5);
    assert!(options.iter().all(|o| o.tone.is_some()));
    assert!(options.iter().all(|o| o.tone != Some(SkinTone::Default)));
}

#[test]
fn there_is_nothing_to_reset_to_when_the_glyph_is_not_overriding() {
    assert!(
        tone_options(WAVE, true, None, None)
            .iter()
            .all(|o| o.tone.is_some())
    );
}

#[test]
fn an_overriding_glyph_is_offered_a_reset_first() {
    let options = tone_options(WAVE, true, Some(SkinTone::Dark), Some(SkinTone::Light));
    assert_eq!(options[0].tone, None);
    assert_eq!(options[0].label, "Reset to preference");
}

#[test]
fn the_reset_row_previews_the_tone_it_would_return_to() {
    let options = tone_options(WAVE, true, Some(SkinTone::Dark), Some(SkinTone::Light));
    assert_eq!(options[0].preview, format!("{WAVE}\u{1F3FB}"));
}

#[test]
fn the_current_tone_is_not_offered_again() {
    // The panel would otherwise offer to do nothing.
    let options = tone_options(WAVE, true, Some(SkinTone::Dark), Some(SkinTone::Light));
    assert!(!options.iter().any(|o| o.tone == Some(SkinTone::Dark)));
}

#[test]
fn the_pickers_own_tone_is_not_offered_either() {
    // The reset row already goes there; offering it twice would be two rows
    // doing the same thing.
    let options = tone_options(WAVE, true, Some(SkinTone::Dark), Some(SkinTone::Light));
    assert!(!options.iter().any(|o| o.tone == Some(SkinTone::Light)));
}

#[test]
fn an_overriding_glyph_is_offered_four_tones_and_a_reset() {
    // Six tones, less the current one, less the picker's, plus the reset.
    let options = tone_options(WAVE, true, Some(SkinTone::Dark), Some(SkinTone::Light));
    assert_eq!(options.len(), 5);
    assert_eq!(options.iter().filter(|o| o.tone.is_some()).count(), 4);
}

#[test]
fn each_option_previews_itself_in_its_own_tone() {
    let options = tone_options(WAVE, true, None, None);
    let dark = options
        .iter()
        .find(|o| o.tone == Some(SkinTone::Dark))
        .expect("a dark option");
    assert_eq!(dark.preview, format!("{WAVE}\u{1F3FF}"));
    assert_eq!(dark.label, "Dark skin tone");
}

#[test]
fn the_tone_section_has_a_heading() {
    assert_eq!(TONE_SECTION_TITLE, "Skin tones");
}
