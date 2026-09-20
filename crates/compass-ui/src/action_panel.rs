//! The action panel's rows, filter and navigation.
//!
//! Ports `src/server/src/ui/action-panel/action-panel-model.cpp` — the panel
//! every command's actions are shown in. The panel is a list of *sections*
//! flattened into rows, where some rows are headings and dividers that cannot
//! be selected, so almost everything here is about the difference between a
//! row and a selectable row.
//!
//! Nothing draws. The flattening, the filter and the navigation are separable
//! from Iced and are what a display server would otherwise be needed to test.

/// One action a panel offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Stable dispatch identity, independent of the displayed title.
    pub id: Option<String>,
    /// What it is called, and what the filter matches against.
    pub title: String,
    /// The key that runs it without opening the panel, if it has one.
    pub shortcut: Option<String>,
}

impl Action {
    /// An action with no shortcut.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: None,
            title: title.into(),
            shortcut: None,
        }
    }

    /// Assign the identity used by the host to execute this action.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The same action with a shortcut bound.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

/// A named group of actions.
///
/// The name may be empty, and that is not the same as having no section: an
/// unnamed section still separates its actions from the ones above with a
/// divider, it just does not announce why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelSection {
    /// The heading, or empty for none.
    pub name: String,
    /// The actions, in order.
    pub actions: Vec<Action>,
}

/// What a flattened row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A rule between two sections. Not selectable.
    Divider,
    /// A section's name. Not selectable.
    Header,
    /// An action. Selectable.
    Item,
}

/// One row of the flattened panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What it is.
    pub kind: RowKind,
    /// Which section it belongs to.
    pub section: usize,
    /// Which action within that section, for an [`RowKind::Item`].
    pub action: Option<usize>,
}

impl Row {
    /// Whether the selection may land here.
    #[must_use]
    pub const fn selectable(&self) -> bool {
        matches!(self.kind, RowKind::Item)
    }
}

/// Flatten the panel's sections into rows, keeping only what the filter matches.
///
/// Three rules, and each one is a thing that looks wrong on screen if it is
/// missed:
///
/// - A section whose actions are all filtered out contributes **nothing** —
///   no heading and no divider. A heading over an empty space is the most
///   visible way to get this wrong.
/// - The divider goes *between* sections, so there is never one above the
///   first row. It is emitted before the next section's heading rather than
///   after the previous section's last action, which is what makes "between"
///   hold when a middle section drops out under the filter.
/// - An unnamed section gets no heading but still gets its divider, because
///   the separation is what the section is *for* even when it has nothing to
///   say about itself.
#[must_use]
pub fn flatten(sections: &[PanelSection], filter: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut needs_divider = false;

    for (index, section) in sections.iter().enumerate() {
        let matched: Vec<usize> = section
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| matches_filter(&action.title, filter))
            .map(|(position, _)| position)
            .collect();

        if matched.is_empty() {
            continue;
        }

        if needs_divider {
            rows.push(Row {
                kind: RowKind::Divider,
                section: index,
                action: None,
            });
        }

        if !section.name.is_empty() {
            rows.push(Row {
                kind: RowKind::Header,
                section: index,
                action: None,
            });
        }

        for position in matched {
            rows.push(Row {
                kind: RowKind::Item,
                section: index,
                action: Some(position),
            });
        }

        needs_divider = true;
    }

    rows
}

/// Whether an action's title matches the filter.
///
/// An empty filter matches everything — it is the panel just opened, not a
/// search for nothing. No early return is needed for that: `all` over an empty
/// iterator is already true, and a guard no mutation can reach is worse than
/// no guard. Matching is a case-insensitive subsequence, which is what the
/// fuzzy matcher does for a single field and is enough here: the panel is a
/// dozen rows, not an index.
#[must_use]
pub fn matches_filter(title: &str, filter: &str) -> bool {
    let mut haystack = title.chars().flat_map(char::to_lowercase);
    filter
        .chars()
        .flat_map(char::to_lowercase)
        .all(|needle| haystack.any(|c| c == needle))
}

/// Which way the selection moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Towards the top.
    Up,
    /// Towards the bottom.
    Down,
}

/// The next selectable row, skipping headings and dividers.
///
/// `from` may be `-1`, which is how the first selectable row is asked for
/// without pretending the selection was somewhere. A panel with no selectable
/// rows returns `from` unchanged rather than an index into nothing.
#[must_use]
pub fn next_selectable(rows: &[Row], from: isize, step: Step, wrap: bool) -> isize {
    let count = rows.len() as isize;
    // No guard for an empty panel: both loops below are bounded by `count`, so
    // they do not run and `from` comes back. A guard no mutation can reach is
    // worse than no guard.
    let delta = match step {
        Step::Down => 1,
        Step::Up => -1,
    };

    let mut at = from + delta;
    while at >= 0 && at < count {
        if rows[at as usize].selectable() {
            return at;
        }
        at += delta;
    }

    if !wrap {
        return from;
    }

    let mut at = if delta > 0 { 0 } else { count - 1 };
    while at >= 0 && at < count {
        if rows[at as usize].selectable() {
            return at;
        }
        at += delta;
    }
    from
}

/// The first action of the next or previous *section*.
///
/// Moving down goes to the first action of whichever section comes next.
/// Moving up goes to the first action of the **current** section when the
/// selection is not already there — so one press takes you to the top of what
/// you are in, and a second takes you to the section above. That is what makes
/// the key usable without counting rows.
#[must_use]
pub fn next_section(rows: &[Row], from: isize, step: Step, wrap: bool) -> isize {
    if rows.is_empty() {
        return from;
    }
    let current = rows
        .get(usize::try_from(from).unwrap_or(usize::MAX))
        .map(|row| row.section);

    match step {
        Step::Down => {
            for (index, row) in rows.iter().enumerate().skip((from + 1).max(0) as usize) {
                if row.selectable() && Some(row.section) != current {
                    return index as isize;
                }
            }
            if wrap {
                next_selectable(rows, -1, Step::Down, false)
            } else {
                from
            }
        }
        Step::Up => {
            let start = section_start(rows, from, current);
            if start < from {
                return start;
            }
            match previous_section(rows, start) {
                Some(section) => first_of_section(rows, section).unwrap_or(from),
                None if wrap => last_section(rows)
                    .and_then(|section| first_of_section(rows, section))
                    .unwrap_or(from),
                None => from,
            }
        }
    }
}

/// The first selectable row of the section `from` is in.
fn section_start(rows: &[Row], from: isize, current: Option<usize>) -> isize {
    let mut start = from;
    let mut index = from - 1;
    while index >= 0 {
        let row = &rows[index as usize];
        if row.selectable() {
            if Some(row.section) != current {
                break;
            }
            start = index;
        }
        index -= 1;
    }
    start
}

/// The section of the nearest selectable row above `start`.
fn previous_section(rows: &[Row], start: isize) -> Option<usize> {
    (0..start)
        .rev()
        .map(|index| &rows[index as usize])
        .find(|row| row.selectable())
        .map(|row| row.section)
}

/// The first selectable row belonging to `section`.
fn first_of_section(rows: &[Row], section: usize) -> Option<isize> {
    rows.iter()
        .position(|row| row.selectable() && row.section == section)
        .map(|index| index as isize)
}

/// The section of the last selectable row.
fn last_section(rows: &[Row]) -> Option<usize> {
    rows.iter()
        .rev()
        .find(|row| row.selectable())
        .map(|row| row.section)
}

/// Which row the list should scroll to when the selection lands on `index`.
///
/// Moving *up* onto a row whose predecessor is a heading scrolls to the
/// heading instead. Without it the heading sits just above the viewport and
/// the first row of a section looks like the middle of the one before.
#[must_use]
pub fn scroll_target(rows: &[Row], index: isize, step: Step) -> isize {
    if step == Step::Up
        && index > 0
        && let Some(previous) = rows.get((index - 1) as usize)
        && previous.kind == RowKind::Header
    {
        return index - 1;
    }
    index
}

/// The row a keyboard shortcut runs.
///
/// The first bound action in flat order wins. Two actions sharing a shortcut
/// is a mistake nothing reports, so the order decides it rather than nothing
/// happening — and the order is the panel's own, which is the one the author
/// wrote.
#[must_use]
pub fn row_for_shortcut(rows: &[Row], sections: &[PanelSection], shortcut: &str) -> Option<usize> {
    rows.iter().position(|row| {
        row.selectable()
            && row
                .action
                .and_then(|position| sections.get(row.section)?.actions.get(position))
                .and_then(|action| action.shortcut.as_deref())
                == Some(shortcut)
    })
}

/// Where the selection goes when the filter changes.
///
/// To the first selectable row. The rows under a filter are a different list,
/// and keeping the old index would leave the selection on whatever now sits
/// there — which for an action panel means running something else.
#[must_use]
pub fn selection_after_filter(rows: &[Row]) -> isize {
    next_selectable(rows, -1, Step::Down, false)
}
