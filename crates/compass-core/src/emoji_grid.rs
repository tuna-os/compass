//! The emoji and symbol picker.
//!
//! Ports `src/server/src/builtins/vicinae/emoji-grid-model.cpp` together with
//! the skin-tone arithmetic it leans on from `src/lib/glyph/src/emoji.cpp` —
//! which glyph is copied, what the action panel offers, and which skin-tone
//! options are worth showing.

/// The five modifiers, plus the absence of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkinTone {
    /// No modifier: the glyph as the font draws it.
    #[default]
    Default,
    /// Fitzpatrick type 1–2.
    Light,
    /// Fitzpatrick type 3.
    MediumLight,
    /// Fitzpatrick type 4.
    Medium,
    /// Fitzpatrick type 5.
    MediumDark,
    /// Fitzpatrick type 6.
    Dark,
}

/// One row of the skin-tone table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinToneInfo {
    /// Which tone.
    pub tone: SkinTone,
    /// How it is stored.
    pub id: &'static str,
    /// How it is shown.
    pub display_name: &'static str,
    /// The modifier itself, empty for [`SkinTone::Default`].
    pub modifier: &'static str,
}

/// The skin-tone table, in the order the C++ declares it.
///
/// `Default` is first and carries an **empty** modifier, which is what makes
/// applying it a no-op rather than a special case in the applier.
pub const SKIN_TONES: &[SkinToneInfo] = &[
    SkinToneInfo {
        tone: SkinTone::Default,
        id: "default",
        display_name: "Default",
        modifier: "",
    },
    SkinToneInfo {
        tone: SkinTone::Light,
        id: "light",
        display_name: "Light",
        modifier: "\u{1F3FB}",
    },
    SkinToneInfo {
        tone: SkinTone::MediumLight,
        id: "medium-light",
        display_name: "Medium Light",
        modifier: "\u{1F3FC}",
    },
    SkinToneInfo {
        tone: SkinTone::Medium,
        id: "medium",
        display_name: "Medium",
        modifier: "\u{1F3FD}",
    },
    SkinToneInfo {
        tone: SkinTone::MediumDark,
        id: "medium-dark",
        display_name: "Medium Dark",
        modifier: "\u{1F3FE}",
    },
    SkinToneInfo {
        tone: SkinTone::Dark,
        id: "dark",
        display_name: "Dark",
        modifier: "\u{1F3FF}",
    },
];

/// The variation selector that asks for the coloured form of a glyph.
pub const VARIATION_SELECTOR_16: char = '\u{FE0F}';

/// Look a tone up by its stored id.
#[must_use]
pub fn skin_tone_by_id(id: &str) -> Option<SkinTone> {
    SKIN_TONES.iter().find(|t| t.id == id).map(|t| t.tone)
}

/// The table row for a tone.
#[must_use]
pub fn skin_tone_info(tone: SkinTone) -> &'static SkinToneInfo {
    SKIN_TONES
        .iter()
        .find(|t| t.tone == tone)
        .expect("every tone is in the table")
}

/// Apply a skin tone to a glyph.
///
/// The modifier goes immediately after the **first** codepoint, not at the
/// end: in a sequence like a person joined to an object, the tone belongs to
/// the person.
///
/// Every variation selector in what follows is then stripped. The C++ comment
/// gives the reason and it is not cosmetic — a tone modifier already forces
/// the coloured presentation, and leaving the selector in produces a sequence
/// some fonts refuse to compose, so the glyph would break apart into its
/// parts.
#[must_use]
pub fn apply_skin_tone(glyph: &str, tone: SkinTone) -> String {
    let info = skin_tone_info(tone);
    let mut chars = glyph.chars();
    let Some(first) = chars.next() else {
        return glyph.to_owned();
    };
    let rest: String = chars.filter(|c| *c != VARIATION_SELECTOR_16).collect();
    format!("{first}{}{rest}", info.modifier)
}

/// The codepoint label shown and copied for a glyph.
///
/// Only the **first** codepoint, uppercase hex, padded to at least four
/// digits. A sequence therefore reports the codepoint of the thing it is a
/// sequence *of*, which is the number someone looking it up wants.
#[must_use]
pub fn formatted_codepoint(glyph: &str) -> String {
    let Some(first) = glyph.chars().next() else {
        return "U+0000".to_owned();
    };
    format!("U+{:04X}", first as u32)
}

/// Which tone a glyph is actually drawn with.
///
/// The glyph's own remembered tone wins; failing that the picker's current
/// tone; failing that no modifier at all. Per-glyph beats per-picker because
/// someone who has set a tone on one glyph meant that glyph.
#[must_use]
pub fn effective_tone(glyph_tone: Option<SkinTone>, picker_tone: Option<SkinTone>) -> SkinTone {
    glyph_tone.unwrap_or_else(|| picker_tone.unwrap_or_default())
}

/// The glyph that gets copied.
///
/// A glyph that takes no tone is copied as it is — applying a modifier to it
/// would produce a sequence that renders as the glyph followed by a coloured
/// square.
#[must_use]
pub fn copied_glyph(character: &str, skinnable: bool, tone: SkinTone) -> String {
    if skinnable {
        apply_skin_tone(character, tone)
    } else {
        character.to_owned()
    }
}

/// The icon a grid cell shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellIcon {
    /// A symbol, drawn from a font rather than as an emoji.
    Symbol(String),
    /// An emoji.
    Emoji(String),
}

/// What kind of glyph a row holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphKind {
    /// An emoji.
    Emoji,
    /// A symbol — an arrow, a mathematical sign, a Greek letter.
    Symbol,
}

/// Choose a grid cell's icon.
///
/// Symbols take a different scheme from emoji, because they are drawn with the
/// interface font rather than the colour emoji font — an arrow rendered as an
/// emoji would be the wrong size and the wrong weight beside its neighbours.
#[must_use]
pub fn cell_icon(
    character: &str,
    kind: GlyphKind,
    skinnable: bool,
    glyph_tone: Option<SkinTone>,
    picker_tone: Option<SkinTone>,
) -> CellIcon {
    if kind == GlyphKind::Symbol {
        return CellIcon::Symbol(character.to_owned());
    }
    if skinnable {
        return CellIcon::Emoji(apply_skin_tone(
            character,
            effective_tone(glyph_tone, picker_tone),
        ));
    }
    CellIcon::Emoji(character.to_owned())
}

/// The first section of a glyph's action panel.
///
/// Copy and paste both register a visit, so the picker's ordering learns from
/// either — a glyph reached by pasting is as much "used" as one reached by
/// copying. Where both appear, the preference decides only their order.
#[must_use]
pub fn main_actions(supports_paste: bool, default_action: &str, pinned: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if supports_paste {
        if default_action == "paste" {
            out.push("paste");
            out.push("copy");
        } else {
            out.push("copy");
            out.push("paste");
        }
    } else {
        out.push("copy");
    }
    out.push("copy-name");
    out.push("copy-codepoint");
    out.push("copy-category");
    out.push("edit-keyword");
    out.push("reset-ranking");
    out.push(if pinned { "unpin" } else { "pin" });
    out
}

/// Which actions register a visit when they run.
///
/// Only the two that put the glyph somewhere. Copying its *name* or its
/// codepoint is looking something up, not using the glyph, and counting it
/// would let a search for a name drift the picker's ordering.
#[must_use]
pub fn action_registers_visit(action: &str) -> bool {
    matches!(action, "copy" | "paste")
}

/// A skin-tone option offered in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToneOption {
    /// Which tone it sets, or `None` for "reset to preference".
    pub tone: Option<SkinTone>,
    /// What the row is called.
    pub label: String,
    /// The glyph drawn beside it, in that tone.
    pub preview: String,
}

/// The skin-tone section of a glyph's action panel.
///
/// Empty for a glyph that takes no tone. Otherwise: a "reset to preference"
/// row **only when the glyph is currently overriding** the picker's tone —
/// there is nothing to reset to otherwise — followed by every tone that is
/// neither the current one nor the picker's default.
///
/// Both exclusions matter. Skipping the current tone keeps the panel from
/// offering to do nothing; skipping the picker's default keeps it from
/// offering a second route to what the reset row already does.
#[must_use]
pub fn tone_options(
    character: &str,
    skinnable: bool,
    glyph_tone: Option<SkinTone>,
    picker_tone: Option<SkinTone>,
) -> Vec<ToneOption> {
    if !skinnable {
        return Vec::new();
    }
    let default_tone = picker_tone.unwrap_or_default();
    let tone = glyph_tone.unwrap_or(default_tone);
    let mut out = Vec::new();

    if tone != default_tone {
        out.push(ToneOption {
            tone: None,
            label: "Reset to preference".to_owned(),
            preview: apply_skin_tone(character, default_tone),
        });
    }

    for info in SKIN_TONES {
        if info.tone == tone || info.tone == default_tone {
            continue;
        }
        out.push(ToneOption {
            tone: Some(info.tone),
            label: format!("{} skin tone", info.display_name),
            preview: apply_skin_tone(character, info.tone),
        });
    }

    out
}

/// The section heading the tone options sit under.
pub const TONE_SECTION_TITLE: &str = "Skin tones";
