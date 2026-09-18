//! The root search list's sections and selection.
//!
//! The launcher's main list is not one list. It is favourites, then results,
//! then — when nothing matched — the fallback commands, each under its own
//! heading. That shape is what this module holds, together with the selection
//! moving *through* it, because a selection that treats a sectioned list as a
//! flat one is the bug this exists to prevent: the arrow key stops on a
//! heading, or skips the first row of the next section, and neither is visible
//! until someone tries it on a list with more than one section.
//!
//! Nothing here draws. The ranking is [`compass_core::root_items::search`] and
//! the behaviours around it are [`compass_core::root_view`]; both are ported
//! and tested. What is new is the arrangement, and it is kept out of the Iced
//! `view` function so it can be tested in a container with no display server.

use compass_core::root_items::{RootItem, SearchOptions, search};

/// What a section is called and why it is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    /// Items the user pinned, in the order they arranged them.
    Favorites,
    /// What matched what was typed.
    Results,
    /// Commands offered when nothing matched.
    Fallbacks,
    /// Recently used, shown for the empty query.
    Suggestions,
}

impl SectionKind {
    /// The heading drawn above the section.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Favorites => "Favorites",
            Self::Results => "Results",
            Self::Fallbacks => "Use with...",
            Self::Suggestions => "Suggestions",
        }
    }
}

/// One heading and the rows under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Which section this is.
    pub kind: SectionKind,
    /// Indices into the caller's item slice, in display order.
    pub rows: Vec<usize>,
}

/// The whole list, as the view draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RootList {
    /// The sections, in display order. Empty sections are never included.
    pub sections: Vec<Section>,
}

impl RootList {
    /// How many selectable rows there are in total.
    ///
    /// Headings are not rows. A list of two sections with one item each has
    /// two selectable positions, not four, and this is the number the
    /// selection arithmetic works in.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sections.iter().map(|s| s.rows.len()).sum()
    }

    /// Whether there is anything to select.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The item index at a flat selection position.
    #[must_use]
    pub fn item_at(&self, position: usize) -> Option<usize> {
        let mut remaining = position;
        for section in &self.sections {
            if remaining < section.rows.len() {
                return Some(section.rows[remaining]);
            }
            remaining -= section.rows.len();
        }
        None
    }

    /// Which section a flat position falls in, and where inside it.
    #[must_use]
    pub fn locate(&self, position: usize) -> Option<(usize, usize)> {
        let mut remaining = position;
        for (index, section) in self.sections.iter().enumerate() {
            if remaining < section.rows.len() {
                return Some((index, remaining));
            }
            remaining -= section.rows.len();
        }
        None
    }
}

/// Build the list for a query.
///
/// Favourites come first and only for the **empty** query: once something has
/// been typed, a favourite that does not match is not an answer, and keeping
/// it above the results would push the thing the user asked for down the page.
///
/// Fallbacks appear only when nothing matched, which is what makes them
/// fallbacks rather than a fifth section always on screen.
#[must_use]
pub fn build(items: &[RootItem], query: &str, fallbacks: &[usize], now: i64) -> RootList {
    let mut sections = Vec::new();

    if query.is_empty() {
        let favorites = favorite_rows(items);
        if !favorites.is_empty() {
            sections.push(Section {
                kind: SectionKind::Favorites,
                rows: favorites,
            });
        }
    }

    let opts = SearchOptions {
        include_favorites: !query.is_empty(),
        ..SearchOptions::default()
    };
    let rows: Vec<usize> = search(items, query, &opts, now)
        .into_iter()
        .map(|scored| scored.index)
        .collect();

    if !rows.is_empty() {
        sections.push(Section {
            kind: if query.is_empty() {
                SectionKind::Suggestions
            } else {
                SectionKind::Results
            },
            rows,
        });
    }

    if sections.is_empty() && !fallbacks.is_empty() {
        sections.push(Section {
            kind: SectionKind::Fallbacks,
            rows: fallbacks.to_vec(),
        });
    }

    RootList { sections }
}

/// The favourites, in the order the user arranged them.
///
/// Sorted by the stored index rather than by score: the whole point of
/// arranging them is that the arrangement is kept.
#[must_use]
pub fn favorite_rows(items: &[RootItem]) -> Vec<usize> {
    let mut rows: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.meta.enabled)
        .filter_map(|(index, item)| item.meta.favorite_idx.map(|fav| (fav, index)))
        .collect();
    rows.sort_by_key(|(fav, _)| *fav);
    rows.into_iter().map(|(_, index)| index).collect()
}

/// Which way the selection moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// Towards the top.
    Up,
    /// Towards the bottom.
    Down,
}

/// Move the selection one row.
///
/// The arithmetic is flat and the sections are invisible to it, which is the
/// point: a selection that knew about sections would have to decide what to do
/// *on* a heading, and the answer is that it never lands on one because a
/// heading is not a position.
///
/// Wrapping is a setting because the two behaviours conflict with reaching
/// search history: with wrapping on, the up arrow at the top jumps to the
/// bottom, so it cannot also mean "previous search". An empty list stays at 0
/// rather than wrapping to an index that is not there.
#[must_use]
pub fn move_selection(list: &RootList, selected: usize, direction: Move, wrap: bool) -> usize {
    let len = list.len();
    if len == 0 {
        return 0;
    }
    match direction {
        Move::Down => {
            if selected + 1 < len {
                selected + 1
            } else if wrap {
                0
            } else {
                selected
            }
        }
        Move::Up => {
            if selected > 0 {
                selected - 1
            } else if wrap {
                len - 1
            } else {
                selected
            }
        }
    }
}

/// Where the selection goes when the list is rebuilt.
///
/// Back to the top. A list rebuilt because the query changed is a different
/// list, and keeping the old position would leave the selection on whatever
/// happens to be third now — which is how a launcher opens something the user
/// was not looking at.
#[must_use]
pub const fn selection_after_rebuild() -> usize {
    0
}

/// Clamp a selection to a list that may have shrunk under it.
///
/// A rebuild that arrives while the user is reading — a new window appearing,
/// an index finishing — must not leave the selection past the end.
#[must_use]
pub fn clamp_selection(list: &RootList, selected: usize) -> usize {
    let len = list.len();
    if len == 0 {
        return 0;
    }
    selected.min(len - 1)
}
