//! Every emoji and curated symbol the launcher offers.
//!
//! Ports `src/lib/glyph`. The table itself is not written here and not
//! regenerated here: `build.rs` parses the committed
//! `src/lib/glyph/src/glyph.cpp`, which
//! `src/lib/glyph/scripts/gen.ts` produced from the Unicode Character
//! Database, emoji-test and the CLDR English annotations.
//!
//! # Why read the C++ rather than generate afresh
//!
//! Regenerating needs network access, a Node toolchain and the same curation
//! decisions (`scripts/src/curate.ts` picks which symbol blocks are in, and
//! `categories.ts` fixes the category order). Two generators run at different
//! times would drift, and the drift would show up as an emoji one engine
//! offers and the other does not. Parsing the committed table makes that
//! impossible, and a change to its shape fails the build rather than
//! quietly producing a shorter table.
//!
//! The generated file says Unicode 17.0.0, Emoji 17.0, CLDR 48.2.0 on its
//! first line; [`SOURCE_VERSIONS`] carries that line so a reader can tell what
//! this build holds.

use std::ops::Range;

/// One entry: an emoji or a symbol.
///
/// `Clone` but not `Copy`: the keyword span is a `Range`, which is not `Copy`
/// on purpose in the standard library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    /// The character itself, which may be several code points.
    pub character: &'static str,
    /// Its CLDR name, lower case.
    pub name: &'static str,
    /// Where its keywords live in [`KEYWORDS`].
    keywords: Range<usize>,
    /// Emoji or symbol.
    pub kind: Kind,
    /// Which section it belongs to.
    pub category: Category,
    /// Whether it takes a skin-tone modifier.
    pub skinnable: bool,
}

/// One section of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The category this section shows.
    pub category: Category,
    /// Emoji or symbol.
    pub kind: Kind,
    /// The heading, e.g. `"Smileys & Emotion"`.
    pub label: &'static str,
    /// Where its members live in [`glyphs`].
    members: Range<usize>,
}

include!(concat!(env!("OUT_DIR"), "/glyph_table.rs"));

impl Glyph {
    /// The search keywords for this entry.
    #[must_use]
    pub fn keywords(&self) -> &'static [&'static str] {
        &KEYWORDS[self.keywords.clone()]
    }
}

impl Section {
    /// The entries in this section, in table order.
    #[must_use]
    pub fn members(&self) -> &'static [Glyph] {
        &GLYPHS[self.members.clone()]
    }
}

/// The first line of the generated C++ table, naming the data's versions.
///
/// Read at build time from the file itself rather than written down, so it
/// cannot claim a Unicode release this build does not hold.
pub const SOURCE_VERSIONS: &str = include_str!(concat!(env!("OUT_DIR"), "/glyph_versions.txt"));

/// Every entry, in the table's own order.
#[must_use]
pub fn glyphs() -> &'static [Glyph] {
    GLYPHS
}

/// Every section, in the order the picker shows them.
#[must_use]
pub fn sections() -> &'static [Section] {
    SECTIONS
}

/// The entry for exactly this character, if there is one.
///
/// The C++ binary-searches a sorted index; this scans, because 5,059 string
/// comparisons is a few microseconds and a second sorted structure is a second
/// thing to keep in step. If a profile ever says otherwise, the index is in
/// the same generated file.
#[must_use]
pub fn lookup(character: &str) -> Option<&'static Glyph> {
    GLYPHS.iter().find(|glyph| glyph.character == character)
}

/// Whether a code point only changes how the one before it is drawn: a
/// variation selector or a skin-tone modifier.
fn is_presentation_mark(c: char) -> bool {
    matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}')
}

/// Whether `text` is exactly one emoji, as `emoji::isUtf8EncodedEmoji`
/// answers it — which is how an icon string an extension or a script header
/// gives is told apart from a file name or a URL.
///
/// **A declared divergence in how, not in what.** The C++ runs Google's emoji
/// segmenter over the text and its own copy of the Unicode emoji properties.
/// This compares against the emoji in [`glyphs`] — every fully-qualified
/// sequence of the Emoji release the table was generated from — with the
/// variation selectors and skin tones taken out of both sides, so a toned or
/// text-presentation spelling of a listed emoji is still one. What differs is
/// the edge: a ZWJ sequence no vendor ships is two emoji to this and one to
/// the segmenter, and a bare text-default symbol such as `☺` counts here.
#[must_use]
pub fn is_emoji(text: &str) -> bool {
    let bare = || text.chars().filter(|c| !is_presentation_mark(*c));
    if bare().next().is_none() {
        return false;
    }
    GLYPHS
        .iter()
        .filter(|glyph| glyph.kind == Kind::Emoji)
        .any(|glyph| {
            glyph
                .character
                .chars()
                .filter(|c| !is_presentation_mark(*c))
                .eq(bare())
        })
}

impl Category {
    /// The heading the C++ `categoryLabel` returns.
    #[must_use]
    pub fn label(self) -> &'static str {
        SECTIONS
            .iter()
            .find(|section| section.category == self)
            .map_or("", |section| section.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    const CPP: &str = "src/lib/glyph/src/glyph.cpp";

    fn read_cpp() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join(CPP);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// The count the C++ array declares, e.g. `std::array<Item, 5059>`.
    fn declared_count(source: &str, name: &str) -> usize {
        let at = source
            .find(name)
            .unwrap_or_else(|| panic!("{CPP} no longer declares {name}"));
        let before = &source[..at];
        let open = before.rfind('<').expect("a template argument list");
        let args = &before[open + 1..before.rfind('>').expect("a closing angle bracket")];
        args.rsplit(',')
            .next()
            .expect("a size argument")
            .trim()
            .parse()
            .expect("the size is a number")
    }

    #[test]
    fn the_table_holds_exactly_what_the_cpp_declares() {
        // The check that makes the whole build-time parse trustworthy: the
        // C++ writes its own lengths into the array types, so a parser that
        // dropped entries is caught here rather than by an emoji quietly
        // going missing.
        let source = read_cpp();
        assert_eq!(glyphs().len(), declared_count(&source, "g_items = "));
        assert_eq!(sections().len(), declared_count(&source, "g_sections = "));
        assert_eq!(KEYWORDS.len(), declared_count(&source, "kw_pool = "));
    }

    #[test]
    fn the_sections_cover_every_entry_exactly_once() {
        // The spans are offsets into one array, so an off-by-one in the parse
        // shows up as a section overlapping its neighbour.
        let mut covered = 0;
        let mut expected_start = 0;
        for section in sections() {
            assert_eq!(
                section.members.start, expected_start,
                "section {:?} does not start where the previous one ended",
                section.category
            );
            expected_start = section.members.end;
            covered += section.members().len();
        }
        assert_eq!(covered, glyphs().len());
        assert_eq!(expected_start, glyphs().len());
    }

    #[test]
    fn a_few_well_known_entries_are_where_they_should_be() {
        let grinning = lookup("😀").expect("the grinning face is in the table");
        assert_eq!(grinning.name, "grinning face");
        assert_eq!(grinning.category, Category::SmileysAndEmotion);
        assert_eq!(grinning.kind, Kind::Emoji);
        assert!(!grinning.skinnable);
        assert!(
            grinning.keywords().contains(&"smile"),
            "keywords: {:?}",
            grinning.keywords()
        );

        assert!(lookup("🙂").is_some(), "the slightly smiling face");
        assert!(
            lookup("not an emoji").is_none(),
            "a lookup must not invent an entry"
        );
    }

    #[test]
    fn a_skinnable_entry_is_marked_as_one() {
        // `skinnable` is the last field of each row and the only bool, so a
        // parser that read the wrong column would get every one of these
        // wrong. The waving hand takes a tone; the grinning face does not.
        let wave = lookup("👋").expect("the waving hand is in the table");
        assert!(wave.skinnable, "the waving hand takes a skin tone");
        assert!(
            glyphs().iter().filter(|g| g.skinnable).count() > 50,
            "only {} skinnable entries; the column is probably being misread",
            glyphs().iter().filter(|g| g.skinnable).count()
        );
        assert!(
            glyphs().iter().any(|g| !g.skinnable),
            "everything is skinnable, which cannot be right"
        );
    }

    #[test]
    fn every_keyword_span_is_inside_the_pool() {
        for glyph in glyphs() {
            assert!(
                glyph.keywords.end <= KEYWORDS.len(),
                "{} reaches past the keyword pool",
                glyph.character
            );
            assert!(
                glyph.keywords.start <= glyph.keywords.end,
                "{} has a backwards keyword span",
                glyph.character
            );
        }
    }

    #[test]
    fn the_characters_are_unique() {
        // `lookup` returns the first match, so a duplicated character would
        // make one of the two unreachable.
        let mut seen = BTreeSet::new();
        for glyph in glyphs() {
            assert!(
                seen.insert(glyph.character),
                "{} appears twice",
                glyph.character
            );
        }
    }

    #[test]
    fn the_category_labels_are_the_cpps() {
        // `categoryLabel` is a switch in the C++; the labels here come from
        // the section table, and the two must agree.
        let source = read_cpp();
        for section in sections() {
            assert_eq!(section.category.label(), section.label);
            assert!(
                source.contains(&format!("\"{}\"", section.label)),
                "{CPP} does not contain the label {:?}",
                section.label
            );
        }
        assert_eq!(Category::MiscSymbols.label(), "Misc Symbols");
        assert_eq!(Category::NumberForms.label(), "Number Forms");
    }

    #[test]
    fn the_source_versions_are_the_generated_files_own_first_line() {
        let first = read_cpp().lines().next().unwrap_or_default().to_owned();
        assert_eq!(SOURCE_VERSIONS.trim(), first.trim());
        assert!(
            SOURCE_VERSIONS.contains("Unicode"),
            "the version line does not name a Unicode release: {SOURCE_VERSIONS}"
        );
    }

    #[test]
    fn both_kinds_are_present() {
        assert!(glyphs().iter().any(|g| g.kind == Kind::Emoji));
        assert!(
            glyphs().iter().any(|g| g.kind == Kind::Symbol),
            "the curated symbols are missing, so only half the table was parsed"
        );
    }
}
