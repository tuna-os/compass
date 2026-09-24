//! The emoji and symbol picker: its state, and what it decides.
//!
//! The table is `compass_core::glyph`'s, parsed from the committed C++ one at
//! build time. Search is the same weighted fuzzy scoring the root list uses,
//! over the name first and the CLDR keywords after it.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

/// The most rows drawn at once. The table holds thousands of entries and a
/// launcher list is read from the top; a query narrows it long before this.
pub const MAX_ROWS: usize = 200;

/// The picker's state.
#[derive(Debug, Clone, Default)]
pub struct EmojiPage {
    /// The filter text.
    pub query: String,
    /// Positions in `compass_core::glyph::glyphs()` that match, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
}

impl EmojiPage {
    /// A picker showing the table from the top.
    #[must_use]
    pub fn new() -> Self {
        let mut page = Self::default();
        page.refilter();
        page
    }

    /// Recomputes `shown` for the current query: the table's own order when
    /// it is empty, the best matches first otherwise.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let glyphs = compass_core::glyph::glyphs();
        let query = self.query.trim();
        if query.is_empty() {
            self.shown = (0..glyphs.len().min(MAX_ROWS)).collect();
            return;
        }
        let query = Query::new(query);
        let mut scored: Vec<(usize, u32)> = glyphs
            .iter()
            .enumerate()
            .filter_map(|(index, glyph)| {
                let mut fields = vec![WeightedField::new(glyph.name, 1.0)];
                fields.extend(
                    glyph
                        .keywords()
                        .iter()
                        .map(|keyword| WeightedField::new(keyword, 0.7)),
                );
                let found = score_weighted(&fields, &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((index, found.score))
            })
            .collect();
        // Stable on ties, so equal matches keep the table's order.
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.shown = scored
            .into_iter()
            .take(MAX_ROWS)
            .map(|(index, _)| index)
            .collect();
    }

    /// The selected entry.
    #[must_use]
    pub fn selected_glyph(&self) -> Option<&'static compass_core::glyph::Glyph> {
        compass_core::glyph::glyphs().get(*self.shown.get(self.selected)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_or_a_keyword_finds_the_emoji_and_the_list_is_bounded() {
        let mut page = EmojiPage::new();
        assert_eq!(
            page.shown.len(),
            MAX_ROWS.min(compass_core::glyph::glyphs().len())
        );

        page.query = "thumbs up".into();
        page.refilter();
        assert_eq!(page.selected_glyph().map(|g| g.character), Some("👍"));

        page.query = "pizza".into();
        page.refilter();
        assert!(
            page.shown
                .iter()
                .any(|&i| compass_core::glyph::glyphs()[i].character == "🍕")
        );

        page.query = "zzzqqqxx".into();
        page.refilter();
        assert!(page.shown.is_empty());
    }
}
