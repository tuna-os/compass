//! Grouping the installed fonts, and showing what each one looks like.
//!
//! A port of `FontService` (`src/server/src/services/font-service/`), minus
//! `QFontDatabase` itself.
//!
//! # A font browser is only as good as its grouping
//!
//! A machine has hundreds of font files and perhaps eighty families a person
//! would recognise. Everything here exists to turn the first number into the
//! second: strip the foundry Qt appends, fold `Inter Bold Italic` back into
//! `Inter`, pick one member to represent the group, and file it under the
//! script it is actually for. Get any of that wrong and the list is either
//! unusable or quietly missing the font somebody wanted.
//!
//! The two tables — category names and per-script pangrams — were extracted
//! from the C++ mechanically rather than retyped, because a pangram with a
//! typo in it is wrong in a way nobody reviewing Rust would catch.

/// The scripts `QFontDatabase` reports, as far as this service cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WritingSystem {
    /// Latin.
    Latin,
    /// Cyrillic.
    Cyrillic,
    /// Greek.
    Greek,
    /// Japanese.
    Japanese,
    /// Korean.
    Korean,
    /// Simplified Chinese.
    SimplifiedChinese,
    /// Traditional Chinese.
    TraditionalChinese,
    /// Arabic.
    Arabic,
    /// Hebrew.
    Hebrew,
    /// Thai.
    Thai,
    /// Lao.
    Lao,
    /// Devanagari.
    Devanagari,
    /// Bengali.
    Bengali,
    /// Gurmukhi.
    Gurmukhi,
    /// Gujarati.
    Gujarati,
    /// Tamil.
    Tamil,
    /// Telugu.
    Telugu,
    /// Kannada.
    Kannada,
    /// Malayalam.
    Malayalam,
    /// Sinhala.
    Sinhala,
    /// Thaana.
    Thaana,
    /// Tibetan.
    Tibetan,
    /// Myanmar.
    Myanmar,
    /// Khmer.
    Khmer,
    /// Armenian.
    Armenian,
    /// Georgian.
    Georgian,
    /// Syriac.
    Syriac,
    /// Ogham.
    Ogham,
    /// Runic.
    Runic,
    /// N'Ko.
    Nko,
}

/// The sections a font can be filed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontCategory {
    /// Latin.
    Latin,
    /// Cyrillic.
    Cyrillic,
    /// Greek.
    Greek,
    /// A fixed-pitch Latin font.
    Monospace,
    /// A font patched with the Nerd Font glyph set.
    NerdFonts,
    /// An emoji font.
    Emoji,
    /// Japanese and Korean together.
    Cjk,
    /// Japanese.
    Japanese,
    /// Korean.
    Korean,
    /// Simplified Chinese.
    SimplifiedChinese,
    /// Traditional Chinese.
    TraditionalChinese,
    /// Arabic.
    Arabic,
    /// Hebrew.
    Hebrew,
    /// Thai.
    Thai,
    /// Lao.
    Lao,
    /// Devanagari.
    Devanagari,
    /// Bengali.
    Bengali,
    /// Gurmukhi.
    Gurmukhi,
    /// Gujarati.
    Gujarati,
    /// Tamil.
    Tamil,
    /// Telugu.
    Telugu,
    /// Kannada.
    Kannada,
    /// Malayalam.
    Malayalam,
    /// Sinhala.
    Sinhala,
    /// Armenian.
    Armenian,
    /// Georgian.
    Georgian,
    /// Thaana.
    Thaana,
    /// Tibetan.
    Tibetan,
    /// Myanmar.
    Myanmar,
    /// Khmer.
    Khmer,
    /// Syriac.
    Syriac,
    /// Ogham.
    Ogham,
    /// Runic.
    Runic,
    /// N'Ko.
    Nko,
    /// Anything with no script of its own.
    Symbols,
}

/// A set of categories, one bit each.
pub type FontCategoryMask = u64;

/// The bit standing for `category`.
#[must_use]
pub fn category_bit(category: FontCategory) -> FontCategoryMask {
    1u64 << category_index(category)
}

/// A category's position in the table, which is also its bit.
fn category_index(category: FontCategory) -> u32 {
    CATEGORIES
        .iter()
        .position(|info| info.category == category)
        .unwrap_or(0) as u32
}

/// A category's display name and the glyph shown beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryInfo {
    /// Which category.
    pub category: FontCategory,
    /// What it is called.
    pub name: &'static str,
    /// A sample character, where one is meaningful.
    pub glyph: Option<&'static str>,
}

/// Every category, in the order the browser lists them.
///
/// Extracted from the C++ table rather than retyped.
pub const CATEGORIES: &[CategoryInfo] = &[
    CategoryInfo {
        category: FontCategory::Latin,
        name: "Latin",
        glyph: Some("Aa"),
    },
    CategoryInfo {
        category: FontCategory::Cyrillic,
        name: "Cyrillic",
        glyph: Some("Аб"),
    },
    CategoryInfo {
        category: FontCategory::Greek,
        name: "Greek",
        glyph: Some("Αα"),
    },
    CategoryInfo {
        category: FontCategory::Monospace,
        name: "Monospace",
        glyph: Some("Aa"),
    },
    CategoryInfo {
        category: FontCategory::NerdFonts,
        name: "Nerd Fonts",
        glyph: Some("Aa"),
    },
    CategoryInfo {
        category: FontCategory::Emoji,
        name: "Emoji",
        glyph: Some("😀"),
    },
    CategoryInfo {
        category: FontCategory::Cjk,
        name: "CJK",
        glyph: Some("永"),
    },
    CategoryInfo {
        category: FontCategory::Japanese,
        name: "Japanese",
        glyph: Some("あ"),
    },
    CategoryInfo {
        category: FontCategory::Korean,
        name: "Korean",
        glyph: Some("한"),
    },
    CategoryInfo {
        category: FontCategory::SimplifiedChinese,
        name: "Simplified Chinese",
        glyph: Some("汉"),
    },
    CategoryInfo {
        category: FontCategory::TraditionalChinese,
        name: "Traditional Chinese",
        glyph: Some("漢"),
    },
    CategoryInfo {
        category: FontCategory::Arabic,
        name: "Arabic",
        glyph: Some("أب"),
    },
    CategoryInfo {
        category: FontCategory::Hebrew,
        name: "Hebrew",
        glyph: Some("אב"),
    },
    CategoryInfo {
        category: FontCategory::Thai,
        name: "Thai",
        glyph: Some("ก"),
    },
    CategoryInfo {
        category: FontCategory::Lao,
        name: "Lao",
        glyph: Some("ກ"),
    },
    CategoryInfo {
        category: FontCategory::Devanagari,
        name: "Devanagari",
        glyph: Some("अ"),
    },
    CategoryInfo {
        category: FontCategory::Bengali,
        name: "Bengali",
        glyph: Some("অ"),
    },
    CategoryInfo {
        category: FontCategory::Gurmukhi,
        name: "Gurmukhi",
        glyph: Some("ਅ"),
    },
    CategoryInfo {
        category: FontCategory::Gujarati,
        name: "Gujarati",
        glyph: Some("અ"),
    },
    CategoryInfo {
        category: FontCategory::Tamil,
        name: "Tamil",
        glyph: Some("அ"),
    },
    CategoryInfo {
        category: FontCategory::Telugu,
        name: "Telugu",
        glyph: Some("అ"),
    },
    CategoryInfo {
        category: FontCategory::Kannada,
        name: "Kannada",
        glyph: Some("ಅ"),
    },
    CategoryInfo {
        category: FontCategory::Malayalam,
        name: "Malayalam",
        glyph: Some("അ"),
    },
    CategoryInfo {
        category: FontCategory::Sinhala,
        name: "Sinhala",
        glyph: Some("අ"),
    },
    CategoryInfo {
        category: FontCategory::Armenian,
        name: "Armenian",
        glyph: Some("Աա"),
    },
    CategoryInfo {
        category: FontCategory::Georgian,
        name: "Georgian",
        glyph: Some("ა"),
    },
    CategoryInfo {
        category: FontCategory::Thaana,
        name: "Thaana",
        glyph: Some("ތ"),
    },
    CategoryInfo {
        category: FontCategory::Tibetan,
        name: "Tibetan",
        glyph: Some("ཀ"),
    },
    CategoryInfo {
        category: FontCategory::Myanmar,
        name: "Myanmar",
        glyph: Some("က"),
    },
    CategoryInfo {
        category: FontCategory::Khmer,
        name: "Khmer",
        glyph: Some("ក"),
    },
    CategoryInfo {
        category: FontCategory::Syriac,
        name: "Syriac",
        glyph: None,
    },
    CategoryInfo {
        category: FontCategory::Ogham,
        name: "Ogham",
        glyph: None,
    },
    CategoryInfo {
        category: FontCategory::Runic,
        name: "Runic",
        glyph: None,
    },
    CategoryInfo {
        category: FontCategory::Nko,
        name: "N'Ko",
        glyph: None,
    },
    CategoryInfo {
        category: FontCategory::Symbols,
        name: "Symbols",
        glyph: None,
    },
];

/// The category's display name.
#[must_use]
pub fn category_name(category: FontCategory) -> &'static str {
    CATEGORIES
        .iter()
        .find(|info| info.category == category)
        .map_or("", |info| info.name)
}

/// The categories in table order.
#[must_use]
pub fn ordered_categories() -> Vec<FontCategory> {
    CATEGORIES.iter().map(|info| info.category).collect()
}

/// Scripts that identify a font on their own, and the category each means.
///
/// Every one of these is a script no Latin font also covers by accident, which
/// is what makes "this font has Thai in it" enough to file it under Thai.
pub const DISTINCTIVE_SCRIPTS: &[(WritingSystem, FontCategory)] = &[
    (WritingSystem::Arabic, FontCategory::Arabic),
    (WritingSystem::Hebrew, FontCategory::Hebrew),
    (WritingSystem::Thai, FontCategory::Thai),
    (WritingSystem::Lao, FontCategory::Lao),
    (WritingSystem::Devanagari, FontCategory::Devanagari),
    (WritingSystem::Bengali, FontCategory::Bengali),
    (WritingSystem::Gurmukhi, FontCategory::Gurmukhi),
    (WritingSystem::Gujarati, FontCategory::Gujarati),
    (WritingSystem::Tamil, FontCategory::Tamil),
    (WritingSystem::Telugu, FontCategory::Telugu),
    (WritingSystem::Kannada, FontCategory::Kannada),
    (WritingSystem::Malayalam, FontCategory::Malayalam),
    (WritingSystem::Sinhala, FontCategory::Sinhala),
    (WritingSystem::Thaana, FontCategory::Thaana),
    (WritingSystem::Tibetan, FontCategory::Tibetan),
    (WritingSystem::Myanmar, FontCategory::Myanmar),
    (WritingSystem::Khmer, FontCategory::Khmer),
    (WritingSystem::Armenian, FontCategory::Armenian),
    (WritingSystem::Georgian, FontCategory::Georgian),
    (WritingSystem::Syriac, FontCategory::Syriac),
    (WritingSystem::Ogham, FontCategory::Ogham),
    (WritingSystem::Runic, FontCategory::Runic),
    (WritingSystem::Nko, FontCategory::Nko),
];

/// The pangram shown for each script.
///
/// Extracted from the C++ rather than retyped: a pangram with a typo in it is
/// wrong in a way nobody reviewing Rust would catch, and the whole point of a
/// pangram is that it exercises every letter.
pub const PANGRAMS: &[(WritingSystem, &str)] = &[
    (
        WritingSystem::Latin,
        "The quick brown fox jumps over the lazy dog",
    ),
    (
        WritingSystem::Cyrillic,
        "Съешь же ещё этих мягких французских булок да выпей чаю",
    ),
    (
        WritingSystem::Greek,
        "Ταχίστη αλώπηξ βαφής ψημένη γη, δρασκελίζει υπέρ νωθρού κυνός",
    ),
    (
        WritingSystem::Japanese,
        "いろはにほへと ちりぬるを わかよたれそ つねならむ うゐのおくやま けふこえて あさきゆめみし ゑひもせす",
    ),
    (
        WritingSystem::Korean,
        "키스의 고유조건은 입술끼리 만나야 하고 특별한 기술은 필요치 않다",
    ),
    (WritingSystem::SimplifiedChinese, "视野无限广，窗外有蓝天"),
    (WritingSystem::TraditionalChinese, "視野無限廣，窗外有藍天"),
    (
        WritingSystem::Arabic,
        "صِف خَلقَ خَودِ كَمِثلِ الشَمسِ إِذ بَزَغَت — يَحظى الضَجيعُ بِها نَجلاءَ مِعطارِ",
    ),
    (
        WritingSystem::Hebrew,
        "דג סקרן שט בים מאוכזב ולפתע מצא חברה",
    ),
    (
        WritingSystem::Thai,
        "เป็นมนุษย์สุดประเสริฐเลิศคุณค่า กว่าบรรดาฝูงสัตว์เดรัจฉาน จงฝ่าฟันพัฒนาวิชาการ",
    ),
    (
        WritingSystem::Devanagari,
        "ऋषियों को सताने वाले दुष्ट राक्षसों के राजा रावण का सर्वनाश करने वाले भगवान श्रीराम",
    ),
];

/// The pangram for `system`, if there is one.
///
/// `None` where the C++ falls through to `QFontDatabase::writingSystemSample`,
/// which only the font database can answer.
#[must_use]
pub fn pangram_for(system: WritingSystem) -> Option<&'static str> {
    PANGRAMS
        .iter()
        .find(|(candidate, _)| *candidate == system)
        .map(|(_, text)| *text)
}

/// The scripts the specimen shows after the font's own, in order.
pub const DEMO_SCRIPTS: &[WritingSystem] = &[
    WritingSystem::Japanese,
    WritingSystem::Korean,
    WritingSystem::SimplifiedChinese,
    WritingSystem::TraditionalChinese,
    WritingSystem::Arabic,
    WritingSystem::Hebrew,
    WritingSystem::Thai,
    WritingSystem::Lao,
    WritingSystem::Devanagari,
    WritingSystem::Bengali,
    WritingSystem::Gurmukhi,
    WritingSystem::Gujarati,
    WritingSystem::Tamil,
    WritingSystem::Telugu,
    WritingSystem::Kannada,
    WritingSystem::Malayalam,
    WritingSystem::Sinhala,
    WritingSystem::Thaana,
    WritingSystem::Tibetan,
    WritingSystem::Myanmar,
    WritingSystem::Khmer,
    WritingSystem::Syriac,
    WritingSystem::Ogham,
    WritingSystem::Runic,
    WritingSystem::Nko,
];

/// The emoji sample, which is the same whatever the font covers.
pub const EMOJI_SAMPLE: &str = "\u{1F600} \u{1F602} \u{1F60D} \u{1F60E} \u{1F973} \u{1F62D} \u{1F44D} \u{1F64F} \u{1F389} \u{2764}\u{FE0F} \u{1F525} \u{2B50} \u{1F308} \u{2600}\u{FE0F} \u{1F436} \u{1F431} \u{1F98A} \u{1F427} \u{1F355} \u{1F369} \u{2615} \u{1F680} \u{2708}\u{FE0F} \u{26BD} \u{1F3B8} \u{1F4F7} \u{1F1EB}\u{1F1F7} \u{1F1EF}\u{1F1F5}";

/// The style words stripped from the end of a family name.
///
/// `Inter Bold Italic` is `Inter`; without this the browser lists a dozen
/// entries for one typeface and none of them is the one to pick.
pub const STYLE_TOKENS: &[&str] = &[
    "thin",
    "hairline",
    "extralight",
    "ultralight",
    "light",
    "regular",
    "normal",
    "book",
    "medium",
    "semibold",
    "demibold",
    "demi",
    "semi",
    "bold",
    "extrabold",
    "ultrabold",
    "black",
    "heavy",
    "fat",
    "poster",
    "extra",
    "ultra",
    "italic",
    "oblique",
    "inclined",
];

/// Names that mean a font is emoji.
pub const EMOJI_NAME_MARKERS: &[&str] = &["emoji", "openmoji", "joypixels", "blobmoji"];

/// The abbreviations a Nerd Font patch appends.
pub const NERD_FONT_ABBREVIATIONS: &[&str] = &["NF", "NFM", "NFP"];

/// Symbol fonts whose name is the only tell.
///
/// The URW PostScript symbol fonts map their symbols onto plain ASCII, so the
/// font database reports them as Latin and they would otherwise show up in the
/// Latin section rendering as gibberish.
pub const KNOWN_SYMBOL_FONTS: &[&str] = &["d050000l", "standard symbols ps"];

/// Strip the `" [foundry]"` Qt appends to disambiguate a family.
///
/// Only when it really is a suffix: the index must be greater than zero, so a
/// name that is nothing but a bracketed word keeps it.
#[must_use]
pub fn strip_foundry(family: &str) -> &str {
    if !family.ends_with(']') {
        return family;
    }
    match family.rfind(" [") {
        Some(index) if index > 0 => &family[..index],
        _ => family,
    }
}

/// Fold a family name down to the typeface it belongs to.
///
/// Style words come off the end one at a time, and never the last token: a
/// font actually called `Black` stays `Black` rather than becoming nothing.
#[must_use]
pub fn base_family(family: &str) -> String {
    let mut tokens: Vec<&str> = strip_foundry(family)
        .split(' ')
        .filter(|token| !token.is_empty())
        .collect();
    while tokens.len() > 1
        && STYLE_TOKENS.contains(&tokens[tokens.len() - 1].to_lowercase().as_str())
    {
        tokens.pop();
    }
    tokens.join(" ")
}

/// Which member of a group the browser shows.
///
/// The group's own name when a font actually has it, otherwise the shortest
/// member — which is the one most likely to be the regular weight.
#[must_use]
pub fn representative_family<'a>(base: &'a str, members: &'a [String]) -> &'a str {
    if members.iter().any(|member| member == base) {
        return base;
    }
    members
        .iter()
        .min_by_key(|member| member.chars().count())
        .map_or(base, String::as_str)
}

/// Whether the name says this is an emoji font.
#[must_use]
pub fn name_looks_emoji(family: &str) -> bool {
    let lower = family.to_lowercase();
    EMOJI_NAME_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Whether the name says this is a Nerd Font patch.
///
/// Either the words `Nerd Font` anywhere, or one of the abbreviations as a
/// whole token — as a token, so a font called `Nfinity` is not one.
#[must_use]
pub fn name_looks_nerd_font(family: &str) -> bool {
    if family.to_lowercase().contains("nerd font") {
        return true;
    }
    family
        .split(' ')
        .filter(|token| !token.is_empty())
        .any(|token| {
            NERD_FONT_ABBREVIATIONS
                .iter()
                .any(|abbreviation| token.eq_ignore_ascii_case(abbreviation))
        })
}

/// Whether this is one of the known symbol fonts.
#[must_use]
pub fn is_known_symbol_font(family: &str) -> bool {
    let lower = family.to_lowercase();
    KNOWN_SYMBOL_FONTS.contains(&lower.as_str())
}

/// Whether an emoji font is a colour one.
///
/// Guessed from the name, in this order: anything saying `color` is; anything
/// saying `mono` or `black` is not; a Noto emoji font is not; everything else
/// is. The `color` test has to come first, or `Noto Color Emoji` would be
/// caught by the Noto rule and reported monochrome.
#[must_use]
pub fn emoji_is_color(family: &str) -> bool {
    let lower = family.to_lowercase();
    if lower.contains("color") {
        return true;
    }
    if lower.contains("mono") || lower.contains("black") {
        return false;
    }
    if lower.contains("noto") && lower.contains("emoji") {
        return false;
    }
    true
}

/// The category a font's scripts put it in.
///
/// # Several scripts is not several categories
///
/// A font covering one distinctive script is filed under it. A font covering
/// several is filed under Latin if it also has a European script — that is a
/// pan-Unicode font, and burying it under, say, Gujarati would hide it from
/// everyone — and otherwise under the first of them.
#[must_use]
pub fn script_category(systems: &[WritingSystem]) -> FontCategory {
    // The C++ returns early for an empty list; that is redundant, because with
    // no scripts there is nothing European and nothing distinctive, and the
    // fall-through below already answers Symbols.
    let has = |system: WritingSystem| systems.contains(&system);
    let has_european =
        has(WritingSystem::Latin) || has(WritingSystem::Cyrillic) || has(WritingSystem::Greek);

    let japanese = has(WritingSystem::Japanese);
    let korean = has(WritingSystem::Korean);
    if japanese && korean {
        return FontCategory::Cjk;
    }
    if japanese {
        return FontCategory::Japanese;
    }
    if korean {
        return FontCategory::Korean;
    }
    if has(WritingSystem::SimplifiedChinese) {
        return FontCategory::SimplifiedChinese;
    }
    if has(WritingSystem::TraditionalChinese) {
        return FontCategory::TraditionalChinese;
    }

    let distinctive: Vec<FontCategory> = DISTINCTIVE_SCRIPTS
        .iter()
        .filter(|(system, _)| has(*system))
        .map(|(_, category)| *category)
        .collect();

    match distinctive.len() {
        1 => distinctive[0],
        0 => {
            if has_european {
                FontCategory::Latin
            } else {
                FontCategory::Symbols
            }
        }
        _ => {
            if has_european {
                FontCategory::Latin
            } else {
                distinctive[0]
            }
        }
    }
}

/// The writing system a category's specimen leads with.
#[must_use]
pub fn primary_writing_system(category: FontCategory) -> Option<WritingSystem> {
    match category {
        FontCategory::Latin | FontCategory::Monospace | FontCategory::NerdFonts => {
            Some(WritingSystem::Latin)
        }
        FontCategory::Cjk | FontCategory::Japanese => Some(WritingSystem::Japanese),
        FontCategory::Korean => Some(WritingSystem::Korean),
        FontCategory::SimplifiedChinese => Some(WritingSystem::SimplifiedChinese),
        FontCategory::TraditionalChinese => Some(WritingSystem::TraditionalChinese),
        other => DISTINCTIVE_SCRIPTS
            .iter()
            .find(|(_, candidate)| *candidate == other)
            .map(|(system, _)| *system),
    }
}

/// The glyph shown beside a font in the list.
///
/// The Latin-ish categories pick from what the font actually covers, because
/// showing `Aa` for a font with no Latin in it draws three empty boxes.
#[must_use]
pub fn glyph_for(category: FontCategory, systems: &[WritingSystem]) -> Option<&'static str> {
    if matches!(
        category,
        FontCategory::Latin | FontCategory::Monospace | FontCategory::NerdFonts
    ) {
        if systems.contains(&WritingSystem::Latin) {
            return Some("Aa");
        }
        if systems.contains(&WritingSystem::Cyrillic) {
            return Some("\u{410}\u{431}");
        }
        if systems.contains(&WritingSystem::Greek) {
            return Some("\u{391}\u{3B1}");
        }
        return None;
    }
    CATEGORIES
        .iter()
        .find(|info| info.category == category)
        .and_then(|info| info.glyph)
}

/// How a font was classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    /// The section it is listed under.
    pub primary: FontCategory,
    /// Every category it can be filtered by.
    pub categories: FontCategoryMask,
    /// The glyph shown beside it.
    pub glyph: Option<&'static str>,
    /// Whether it is a colour emoji font.
    pub color: bool,
}

impl Classified {
    /// Whether this font is filed under `category`.
    #[must_use]
    pub fn has(&self, category: FontCategory) -> bool {
        self.categories & category_bit(category) != 0
    }
}

/// Classify a font.
///
/// # Nerd Fonts beat monospace, which beats the script
///
/// A patched font is almost always a monospace Latin one, so without that
/// order every Nerd Font would be listed under Monospace and the section would
/// be nothing else. Both are still *tagged*, so filtering by either finds it.
#[must_use]
pub fn categorize(family: &str, systems: &[WritingSystem], fixed_pitch: bool) -> Classified {
    if name_looks_emoji(family) {
        return Classified {
            primary: FontCategory::Emoji,
            categories: category_bit(FontCategory::Emoji),
            glyph: glyph_for(FontCategory::Emoji, systems),
            color: emoji_is_color(family),
        };
    }

    let mut script = script_category(systems);
    if script == FontCategory::Latin && is_known_symbol_font(family) {
        script = FontCategory::Symbols;
    }

    let nerd = name_looks_nerd_font(family);
    let primary = if nerd {
        FontCategory::NerdFonts
    } else if fixed_pitch && script == FontCategory::Latin {
        FontCategory::Monospace
    } else {
        script
    };

    // `primary` is already NerdFonts whenever `nerd`, so the C++'s separate
    // `if (nerd)` line adds a bit that is always set; only the monospace one
    // below can add something `primary` did not.
    let mut mask = category_bit(primary) | category_bit(script);
    if fixed_pitch {
        mask |= category_bit(FontCategory::Monospace);
    }
    // Filter-only facets: never a section of their own, but tagged on anything
    // that covers them, so "show me fonts with Cyrillic" finds a Latin font.
    if systems.contains(&WritingSystem::Cyrillic) {
        mask |= category_bit(FontCategory::Cyrillic);
    }
    if systems.contains(&WritingSystem::Greek) {
        mask |= category_bit(FontCategory::Greek);
    }

    Classified {
        primary,
        categories: mask,
        glyph: glyph_for(primary, systems),
        color: false,
    }
}

/// One markdown block showing `sample` at four weights.
#[must_use]
pub fn specimen_block(sample: &str) -> String {
    format!("# {sample}\n\n{sample}\n\n**{sample}**\n\n*{sample}*\n\n")
}

/// The specimen shown for a font.
///
/// The font's own script leads, then every demo script it also covers. Only
/// one of the two Chinese scripts is shown, because a font covering both would
/// otherwise print two nearly identical lines. A font with nothing to show
/// falls back to the Latin pangram rather than an empty page.
#[must_use]
pub fn specimen_markdown(
    category: FontCategory,
    systems: &[WritingSystem],
    sample_for: impl Fn(WritingSystem) -> Option<String>,
) -> String {
    if category == FontCategory::Emoji {
        return specimen_block(EMOJI_SAMPLE);
    }

    let has = |system: WritingSystem| systems.contains(&system);
    let primary = primary_writing_system(category);

    let mut order: Vec<WritingSystem> = Vec::new();
    if let Some(primary) = primary.filter(|system| has(*system)) {
        order.push(primary);
    }
    for system in DEMO_SCRIPTS {
        if has(*system) && primary != Some(*system) {
            order.push(*system);
        }
    }

    let mut markdown = String::new();
    let mut chinese_shown = false;
    for system in order {
        let chinese = matches!(
            system,
            WritingSystem::SimplifiedChinese | WritingSystem::TraditionalChinese
        );
        if chinese && chinese_shown {
            continue;
        }
        chinese_shown = chinese_shown || chinese;

        let Some(sample) = pangram_for(system)
            .map(str::to_owned)
            .or_else(|| sample_for(system))
        else {
            continue;
        };
        if !markdown.is_empty() {
            markdown.push_str("---\n\n");
        }
        markdown.push_str(&specimen_block(&sample));
    }

    if markdown.is_empty() {
        markdown = specimen_block(pangram_for(WritingSystem::Latin).unwrap_or_default());
    }
    markdown
}
