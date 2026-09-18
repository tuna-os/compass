//! How the font browser groups and labels what is installed.
//!
//! Read off `FontService` (`src/server/src/services/font-service/`).

use compass_core::font_service::{
    CATEGORIES, DEMO_SCRIPTS, DISTINCTIVE_SCRIPTS, EMOJI_SAMPLE, FontCategory, KNOWN_SYMBOL_FONTS,
    NERD_FONT_ABBREVIATIONS, PANGRAMS, STYLE_TOKENS, WritingSystem, base_family, categorize,
    category_bit, category_name, emoji_is_color, glyph_for, is_known_symbol_font, name_looks_emoji,
    name_looks_nerd_font, ordered_categories, pangram_for, primary_writing_system,
    representative_family, script_category, specimen_block, specimen_markdown, strip_foundry,
};

/// No font database to fall back to.
fn no_samples(_system: WritingSystem) -> Option<String> {
    None
}

#[test]
fn every_category_has_a_name() {
    for info in CATEGORIES {
        assert!(!info.name.is_empty(), "{:?} has no name", info.category);
    }
    assert_eq!(category_name(FontCategory::NerdFonts), "Nerd Fonts");
    assert_eq!(category_name(FontCategory::Nko), "N'Ko");
    assert_eq!(
        category_name(FontCategory::SimplifiedChinese),
        "Simplified Chinese"
    );
}

#[test]
fn the_categories_are_unique_and_ordered() {
    let ordered = ordered_categories();
    assert_eq!(ordered.len(), CATEGORIES.len());
    assert_eq!(ordered[0], FontCategory::Latin, "Latin leads the list");

    let mut seen = ordered.clone();
    seen.sort_by_key(|category| format!("{category:?}"));
    seen.dedup();
    assert_eq!(seen.len(), ordered.len(), "a category is listed twice");
}

#[test]
fn every_category_has_its_own_bit() {
    // The mask is a u64 and there are 35 categories; two sharing a bit would
    // make a filter silently select the wrong section.
    let mut bits: Vec<u64> = CATEGORIES
        .iter()
        .map(|i| category_bit(i.category))
        .collect();
    let count = bits.len();
    bits.sort_unstable();
    bits.dedup();
    assert_eq!(bits.len(), count);
    assert!(count <= 64, "{count} categories will not fit in a u64 mask");
}

#[test]
fn the_scriptless_categories_have_no_glyph() {
    // Ogham, Runic, N'Ko and Syriac have no widely-installed font, so a glyph
    // beside them would render as a box on most machines.
    for category in [
        FontCategory::Syriac,
        FontCategory::Ogham,
        FontCategory::Runic,
        FontCategory::Nko,
        FontCategory::Symbols,
    ] {
        let info = CATEGORIES
            .iter()
            .find(|i| i.category == category)
            .expect("in the table");
        assert_eq!(info.glyph, None, "{category:?}");
    }
}

#[test]
fn a_foundry_suffix_is_stripped() {
    // Qt appends " [foundry]" when two foundries register the same family.
    assert_eq!(strip_foundry("Courier [Adobe]"), "Courier");
    assert_eq!(strip_foundry("Courier"), "Courier");
}

#[test]
fn something_that_is_not_a_foundry_suffix_is_left_alone() {
    assert_eq!(strip_foundry("Font [Unclosed"), "Font [Unclosed");
    assert_eq!(
        strip_foundry("[Bracketed]"),
        "[Bracketed]",
        "no space before it"
    );
    // The C++ requires the index to be greater than zero, so a name that is
    // *nothing but* a foundry suffix keeps it rather than becoming empty.
    assert_eq!(strip_foundry(" [Adobe]"), " [Adobe]");
}

#[test]
fn style_words_fold_into_the_typeface() {
    // Without this the browser lists a dozen entries for one typeface and none
    // of them is the one to pick.
    assert_eq!(base_family("Inter Bold Italic"), "Inter");
    assert_eq!(base_family("Inter"), "Inter");
    assert_eq!(base_family("JetBrains Mono ExtraBold"), "JetBrains Mono");
}

#[test]
fn a_font_actually_called_black_keeps_its_name() {
    // The loop stops at one token, so a style word that is the whole name
    // survives rather than folding the family away to nothing.
    assert_eq!(base_family("Black"), "Black");
    assert_eq!(base_family("Light"), "Light");
}

#[test]
fn style_words_come_off_the_end_only() {
    // "Bold Script" is a typeface whose name begins with Bold.
    assert_eq!(base_family("Bold Script"), "Bold Script");
}

#[test]
fn the_style_list_covers_the_common_weights_and_slopes() {
    for token in [
        "thin", "light", "regular", "bold", "black", "italic", "oblique",
    ] {
        assert!(
            STYLE_TOKENS.contains(&token),
            "{token} should be a style word"
        );
    }
}

#[test]
fn the_group_is_represented_by_its_own_name_when_a_font_has_it() {
    let members = vec!["Inter".to_owned(), "Inter Display".to_owned()];
    assert_eq!(representative_family("Inter", &members), "Inter");
}

#[test]
fn the_exact_match_wins_even_when_it_is_not_the_shortest() {
    // Checked first, so the group's own name is used whenever a font really
    // has it — picking the shortest regardless would show a different font
    // from the one the section is named after.
    let members = vec!["Noto Sans".to_owned(), "Noto Sa".to_owned()];
    assert_eq!(representative_family("Noto Sans", &members), "Noto Sans");
}

#[test]
fn a_group_with_no_exact_member_is_represented_by_the_shortest() {
    // Most likely the regular weight, and in any case the least surprising
    // thing to show.
    let members = vec!["Inter Display".to_owned(), "Inter Tight".to_owned()];
    assert_eq!(representative_family("Inter", &members), "Inter Tight");
}

#[test]
fn an_emoji_font_is_recognised_by_name() {
    for family in ["Noto Color Emoji", "OpenMoji", "JoyPixels", "Blobmoji"] {
        assert!(name_looks_emoji(family), "{family}");
    }
    assert!(!name_looks_emoji("Inter"));
}

#[test]
fn colour_emoji_is_guessed_in_the_right_order() {
    // The colour test has to come first: "Noto Color Emoji" would otherwise be
    // caught by the Noto rule and reported monochrome.
    assert!(emoji_is_color("Noto Color Emoji"));
    assert!(!emoji_is_color("Noto Emoji"));
    assert!(!emoji_is_color("Symbola Mono Emoji"));
    assert!(!emoji_is_color("Emoji Black"));
    assert!(emoji_is_color("JoyPixels"), "unknown means colour");
}

#[test]
fn a_nerd_font_is_recognised_by_its_words_or_its_abbreviation() {
    assert!(name_looks_nerd_font("JetBrainsMono Nerd Font"));
    assert!(name_looks_nerd_font("Hack NF"));
    assert!(name_looks_nerd_font("Hack NFM"));
    for abbreviation in NERD_FONT_ABBREVIATIONS {
        assert!(name_looks_nerd_font(&format!("Hack {abbreviation}")));
    }
}

#[test]
fn an_abbreviation_only_counts_as_a_whole_token() {
    // Otherwise every font with those two letters in its name is a Nerd Font.
    assert!(!name_looks_nerd_font("Nfinity"));
    assert!(!name_looks_nerd_font("Banff"));
}

#[test]
fn the_urw_symbol_fonts_are_known_by_name() {
    // They map their symbols onto plain ASCII, so the font database calls them
    // Latin and they would otherwise sit in the Latin section as gibberish.
    for family in KNOWN_SYMBOL_FONTS {
        assert!(is_known_symbol_font(family));
        assert!(is_known_symbol_font(&family.to_uppercase()));
    }
    assert!(!is_known_symbol_font("Inter"));
}

#[test]
fn a_font_with_no_scripts_is_symbols() {
    assert_eq!(script_category(&[]), FontCategory::Symbols);
}

#[test]
fn japanese_and_korean_together_are_cjk() {
    assert_eq!(
        script_category(&[WritingSystem::Japanese, WritingSystem::Korean]),
        FontCategory::Cjk
    );
    assert_eq!(
        script_category(&[WritingSystem::Japanese]),
        FontCategory::Japanese
    );
    assert_eq!(
        script_category(&[WritingSystem::Korean]),
        FontCategory::Korean
    );
}

#[test]
fn one_distinctive_script_files_a_font_under_it() {
    assert_eq!(script_category(&[WritingSystem::Thai]), FontCategory::Thai);
    assert_eq!(
        script_category(&[WritingSystem::Devanagari]),
        FontCategory::Devanagari
    );
}

#[test]
fn a_pan_unicode_font_is_filed_under_latin_rather_than_buried() {
    // Several distinctive scripts plus a European one. Filing it under, say,
    // Gujarati would hide a general-purpose font from everyone.
    assert_eq!(
        script_category(&[
            WritingSystem::Latin,
            WritingSystem::Gujarati,
            WritingSystem::Thai,
        ]),
        FontCategory::Latin
    );
}

#[test]
fn several_distinctive_scripts_and_no_european_one_take_the_first() {
    assert_eq!(
        script_category(&[WritingSystem::Thai, WritingSystem::Hebrew]),
        FontCategory::Hebrew,
        "Hebrew comes before Thai in the distinctive table"
    );
}

#[test]
fn a_font_with_only_a_european_script_is_latin() {
    assert_eq!(
        script_category(&[WritingSystem::Cyrillic]),
        FontCategory::Latin
    );
    assert_eq!(
        script_category(&[WritingSystem::Greek]),
        FontCategory::Latin
    );
}

#[test]
fn a_nerd_font_outranks_monospace() {
    // A patched font is almost always monospace Latin, so without this order
    // the Monospace section would be nothing but Nerd Fonts.
    let classified = categorize("Hack Nerd Font", &[WritingSystem::Latin], true);
    assert_eq!(classified.primary, FontCategory::NerdFonts);
    assert!(classified.has(FontCategory::Monospace), "still filterable");
    assert!(classified.has(FontCategory::NerdFonts));
    assert!(
        classified.has(FontCategory::Latin),
        "and still findable under the script it is actually for"
    );
}

#[test]
fn a_fixed_pitch_latin_font_is_monospace() {
    let classified = categorize("JetBrains Mono", &[WritingSystem::Latin], true);
    assert_eq!(classified.primary, FontCategory::Monospace);
    assert!(
        classified.has(FontCategory::Latin),
        "the script bit is kept alongside the section it was promoted to"
    );
}

#[test]
fn a_fixed_pitch_font_of_another_script_keeps_that_script() {
    // The Monospace section is a Latin convenience; a fixed-pitch Thai font
    // belongs under Thai.
    let classified = categorize("Some Thai Mono", &[WritingSystem::Thai], true);
    assert_eq!(classified.primary, FontCategory::Thai);
    assert!(classified.has(FontCategory::Monospace), "still filterable");
}

#[test]
fn cyrillic_and_greek_are_filter_facets_and_never_sections() {
    // A Latin font covering Cyrillic is findable by "show me Cyrillic" without
    // being moved out of the Latin section.
    let classified = categorize(
        "Inter",
        &[
            WritingSystem::Latin,
            WritingSystem::Cyrillic,
            WritingSystem::Greek,
        ],
        false,
    );
    assert_eq!(classified.primary, FontCategory::Latin);
    assert!(classified.has(FontCategory::Cyrillic));
    assert!(classified.has(FontCategory::Greek));
}

#[test]
fn an_emoji_font_is_classified_by_name_before_anything_else() {
    let classified = categorize("Noto Color Emoji", &[WritingSystem::Latin], true);
    assert_eq!(classified.primary, FontCategory::Emoji);
    assert!(classified.color);
    assert!(
        !classified.has(FontCategory::Monospace),
        "an emoji font is only emoji"
    );
}

#[test]
fn a_known_symbol_font_is_moved_out_of_latin() {
    let classified = categorize("D050000L", &[WritingSystem::Latin], false);
    assert_eq!(classified.primary, FontCategory::Symbols);
}

#[test]
fn only_emoji_fonts_are_ever_marked_colour() {
    let classified = categorize("Inter Color", &[WritingSystem::Latin], false);
    assert!(!classified.color, "the colour flag is emoji-only");
}

#[test]
fn a_latin_font_shows_aa_and_one_without_latin_does_not() {
    // Showing "Aa" for a font with no Latin in it draws three empty boxes.
    assert_eq!(
        glyph_for(FontCategory::Latin, &[WritingSystem::Latin]),
        Some("Aa")
    );
    assert_eq!(
        glyph_for(FontCategory::Latin, &[WritingSystem::Cyrillic]),
        Some("\u{410}\u{431}")
    );
    assert_eq!(
        glyph_for(FontCategory::Latin, &[WritingSystem::Greek]),
        Some("\u{391}\u{3B1}")
    );
    assert_eq!(glyph_for(FontCategory::Latin, &[WritingSystem::Thai]), None);
}

#[test]
fn a_scripted_category_takes_its_glyph_from_the_table() {
    assert_eq!(
        glyph_for(FontCategory::Thai, &[WritingSystem::Thai]),
        Some("ก")
    );
    assert_eq!(glyph_for(FontCategory::Japanese, &[]), Some("あ"));
}

#[test]
fn every_distinctive_script_maps_back_to_a_writing_system() {
    for (_, category) in DISTINCTIVE_SCRIPTS {
        assert!(
            primary_writing_system(*category).is_some(),
            "{category:?} has no primary script, so its specimen would be empty"
        );
    }
}

#[test]
fn the_pangrams_are_the_cpp_ones() {
    // Extracted mechanically rather than retyped: a pangram with a typo in it
    // fails at the one job a pangram has.
    assert_eq!(
        pangram_for(WritingSystem::Latin),
        Some("The quick brown fox jumps over the lazy dog")
    );
    assert_eq!(PANGRAMS.len(), 11);
    for (_, text) in PANGRAMS {
        assert!(!text.is_empty());
    }
}

#[test]
fn a_specimen_shows_the_sample_at_four_weights() {
    let block = specimen_block("Sample");
    assert!(block.starts_with("# Sample"));
    assert!(block.contains("**Sample**"), "bold");
    assert!(block.contains("*Sample*"), "italic");
    assert_eq!(block.matches("Sample").count(), 4);
}

#[test]
fn an_emoji_specimen_is_the_same_whatever_the_font_covers() {
    let markdown = specimen_markdown(FontCategory::Emoji, &[WritingSystem::Latin], no_samples);
    assert!(markdown.contains(EMOJI_SAMPLE));
    assert!(!markdown.contains("quick brown fox"));
}

#[test]
fn the_fonts_own_script_leads_the_specimen() {
    let markdown = specimen_markdown(
        FontCategory::Japanese,
        &[WritingSystem::Japanese, WritingSystem::Latin],
        no_samples,
    );
    let japanese = pangram_for(WritingSystem::Japanese).expect("a pangram");
    assert!(markdown.starts_with(&format!("# {japanese}")), "{markdown}");
}

#[test]
fn other_covered_scripts_follow_separated_by_a_rule() {
    let markdown = specimen_markdown(
        FontCategory::Latin,
        &[WritingSystem::Latin, WritingSystem::Thai],
        no_samples,
    );
    assert!(markdown.contains("---\n\n"), "a separator between blocks");
    assert!(markdown.contains("quick brown fox"));
    assert!(markdown.contains(pangram_for(WritingSystem::Thai).expect("a pangram")));
}

#[test]
fn only_one_chinese_script_is_shown() {
    // A font covering both would otherwise print two nearly identical lines.
    let markdown = specimen_markdown(
        FontCategory::Latin,
        &[
            WritingSystem::Latin,
            WritingSystem::SimplifiedChinese,
            WritingSystem::TraditionalChinese,
        ],
        no_samples,
    );
    let simplified = pangram_for(WritingSystem::SimplifiedChinese).expect("a pangram");
    let traditional = pangram_for(WritingSystem::TraditionalChinese).expect("a pangram");
    assert!(markdown.contains(simplified));
    assert!(!markdown.contains(traditional));
}

#[test]
fn a_font_covering_nothing_showable_falls_back_to_latin() {
    // An empty specimen page reads as a broken preview.
    let markdown = specimen_markdown(FontCategory::Symbols, &[], no_samples);
    assert!(markdown.contains("quick brown fox"));
}

#[test]
fn a_script_with_no_pangram_uses_the_databases_sample() {
    // Eleven scripts have a hand-written pangram; the rest fall through to
    // QFontDatabase::writingSystemSample, which only it can answer.
    assert_eq!(pangram_for(WritingSystem::Armenian), None);

    let markdown = specimen_markdown(
        FontCategory::Armenian,
        &[WritingSystem::Armenian],
        |system| (system == WritingSystem::Armenian).then(|| "Armenian sample".to_owned()),
    );
    assert!(markdown.contains("Armenian sample"), "{markdown}");
}

#[test]
fn the_demo_scripts_do_not_include_the_european_ones() {
    // Latin, Cyrillic and Greek are shown only when they are the font's own
    // script; listing them as demos would put a Latin pangram under every
    // font in the browser.
    for system in [
        WritingSystem::Latin,
        WritingSystem::Cyrillic,
        WritingSystem::Greek,
    ] {
        assert!(!DEMO_SCRIPTS.contains(&system), "{system:?}");
    }
}
