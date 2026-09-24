//! The view an extension command draws, as the launcher shows it.
//!
//! The engine runs the command and publishes each render as a
//! [`compass_extension_api::View`]; this is what the launcher keeps of it,
//! and every decision about it that is not drawing: which rows the search
//! leaves, which one is selected, and which callback Enter runs.
//!
//! # Who filters
//!
//! A `List` either lets the host filter it (the default) or takes the search
//! text itself (`onSearchTextChange`). The first is filtered here, fuzzily
//! over title, subtitle and keywords; the second is left alone and the text
//! goes to the extension, which renders a new list.

use compass_extension_api::View;
use compass_extension_api::action::{ActionPanel, HandlerId};
use compass_extension_api::view::ListItem;

/// Where the command is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Started; nothing rendered yet.
    Loading,
    /// Showing its view.
    Ready,
    /// It cannot be shown, or it ended: why.
    Stopped(String),
}

/// One extension command's view.
#[derive(Debug, Clone)]
pub struct ExtensionPage {
    /// The engine's session for it.
    pub session: u64,
    /// The command's title, until the view names itself.
    pub title: String,
    /// The last version the engine sent.
    pub version: u64,
    /// The latest view.
    pub view: Option<View>,
    /// Where it is.
    pub status: Status,
    /// The search text.
    pub query: String,
    /// How many times the search text changed, for the extension's echo
    /// counter (ADR-0009).
    pub query_events: u64,
    /// Rows the search leaves, as `(section, item)` in the view.
    pub shown: Vec<(usize, usize)>,
    /// Position in `shown`.
    pub selected: usize,
    /// What went wrong with the last action, if anything.
    pub notice: Option<String>,
    /// How many views the extension has pushed; Escape pops above one.
    pub depth: u32,
    /// A detail's Markdown, parsed once per render rather than per frame.
    pub markdown: Vec<iced::widget::markdown::Item>,
}

impl ExtensionPage {
    /// A page for a session that has just started.
    #[must_use]
    pub fn new(session: u64, title: impl Into<String>) -> Self {
        Self {
            session,
            title: title.into(),
            version: 0,
            view: None,
            status: Status::Loading,
            query: String::new(),
            query_events: 0,
            shown: Vec::new(),
            selected: 0,
            notice: None,
            depth: 1,
            markdown: Vec::new(),
        }
    }

    /// Takes the engine's latest answer.
    pub fn apply(&mut self, state: crate::backend::ExtensionViewState) {
        self.version = state.version;
        if state.view.is_some() {
            if state.depth != self.depth {
                // A different screen: its search starts empty, as Raycast's does.
                self.query.clear();
                self.selected = 0;
            }
            self.depth = state.depth.max(1);
        }
        if let Some(view) = state.view {
            let key = self.selected_item().and_then(|item| item.key.clone());
            self.markdown = match view.as_ref() {
                View::Detail(detail) => detail
                    .markdown
                    .as_deref()
                    .map(|text| iced::widget::markdown::parse(text).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            self.view = Some(*view);
            self.status = Status::Ready;
            self.refilter();
            // Keep the selection on the same row across a re-render, which is
            // every keystroke for a list that filters itself.
            if let Some(key) = key
                && let Some(position) = self.shown.iter().position(|&(s, i)| {
                    self.item(s, i).and_then(|it| it.key.as_deref()) == Some(&key)
                })
            {
                self.selected = position;
            }
        }
        if let Some(problem) = state.problem {
            self.status = Status::Stopped(problem);
        } else if state.ended && self.status != Status::Ready {
            self.status = Status::Stopped(format!("{} finished", self.title));
        }
    }

    /// The list, when the view is one.
    #[must_use]
    pub fn list(&self) -> Option<&compass_extension_api::view::ListView> {
        match &self.view {
            Some(View::List(list)) => Some(list),
            _ => None,
        }
    }

    fn item(&self, section: usize, item: usize) -> Option<&ListItem> {
        self.list()?.sections.get(section)?.items.get(item)
    }

    /// Whether the extension filters its own list.
    #[must_use]
    pub fn extension_filters(&self) -> bool {
        self.list().is_some_and(|list| !list.search.host_filtering)
    }

    /// Recomputes `shown` for the current query.
    pub fn refilter(&mut self) {
        let Some(list) = self.list() else {
            self.shown.clear();
            self.selected = 0;
            return;
        };
        let all = list
            .sections
            .iter()
            .enumerate()
            .flat_map(|(s, section)| (0..section.items.len()).map(move |i| (s, i)));
        let query = self.query.trim();
        self.shown = if query.is_empty() || !list.search.host_filtering {
            all.collect()
        } else {
            let query = compass_search::Query::new(query);
            let mut scored: Vec<((usize, usize), u32)> = all
                .filter_map(|(s, i)| {
                    let item = &list.sections[s].items[i];
                    let mut fields = vec![compass_search::WeightedField::new(&item.title, 1.0)];
                    if let Some(subtitle) = &item.subtitle {
                        fields.push(compass_search::WeightedField::new(subtitle, 0.5));
                    }
                    fields.extend(
                        item.keywords
                            .iter()
                            .map(|k| compass_search::WeightedField::new(k, 0.6)),
                    );
                    let found = compass_search::score_weighted(&fields, &query);
                    (found.quality >= compass_search::MIN_QUALITY && found.score > 0)
                        .then_some(((s, i), found.score))
                })
                .collect();
            // Stable on ties, so equal matches keep the extension's order.
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            scored.into_iter().map(|(row, _)| row).collect()
        };
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
    }

    /// The selected row.
    #[must_use]
    pub fn selected_item(&self) -> Option<&ListItem> {
        let &(s, i) = self.shown.get(self.selected)?;
        self.item(s, i)
    }

    /// The actions Enter and the action panel offer now: the selected row's,
    /// else the list's own, else a detail's.
    #[must_use]
    pub fn actions(&self) -> Option<&ActionPanel> {
        match &self.view {
            Some(View::List(list)) => self
                .selected_item()
                .and_then(|item| item.actions.as_ref())
                .or(list.actions.as_ref()),
            Some(View::Detail(detail)) => detail.actions.as_ref(),
            _ => None,
        }
    }

    /// The handler a chord runs: the action on offer whose shortcut is
    /// exactly `modifiers` plus `key` (key names as `jsx.d.ts` gives them,
    /// compared without case).
    #[must_use]
    pub fn action_for(
        &self,
        modifiers: &[compass_extension_api::action::KeyModifier],
        key: &str,
    ) -> Option<&HandlerId> {
        let mut wanted = modifiers.to_vec();
        wanted.sort();
        wanted.dedup();
        self.actions()?
            .actions()
            .into_iter()
            .find(|action| {
                action.shortcut.as_ref().is_some_and(|shortcut| {
                    let mut has = shortcut.modifiers.clone();
                    has.sort();
                    has.dedup();
                    has == wanted && shortcut.key.as_str().eq_ignore_ascii_case(key)
                })
            })
            .map(|action| &action.handler)
    }

    /// The handler Enter runs: the first action on offer.
    #[must_use]
    pub fn primary_action(&self) -> Option<&HandlerId> {
        self.actions()?
            .actions()
            .into_iter()
            .next()
            .map(|action| &action.handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ExtensionViewState;
    use compass_extension_api::action::{Action, ActionPanel};
    use compass_extension_api::view::{ListSection, ListView};

    fn item(title: &str, key: &str, handler: &str) -> ListItem {
        ListItem::new(title)
            .with_key(key)
            .with_actions(ActionPanel::of([Action::new("Go", handler)]))
    }

    fn list(items: Vec<ListItem>, host_filtering: bool) -> View {
        let mut list = ListView {
            sections: vec![ListSection::untitled(items)],
            ..ListView::default()
        };
        list.search.host_filtering = host_filtering;
        View::List(list)
    }

    fn state(version: u64, view: View) -> ExtensionViewState {
        ExtensionViewState {
            version,
            view: Some(Box::new(view)),
            problem: None,
            ended: false,
            depth: 1,
        }
    }

    #[test]
    fn the_host_filters_fuzzily_and_enter_runs_the_selected_rows_first_action() {
        let mut page = ExtensionPage::new(1, "Repos");
        page.apply(state(
            1,
            list(
                vec![item("compass", "c", "cb-1"), item("vicinae", "v", "cb-2")],
                true,
            ),
        ));
        assert_eq!(page.status, Status::Ready);
        assert_eq!(page.shown.len(), 2);
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-1"));

        page.query = "vicin".into();
        page.refilter();
        assert_eq!(page.shown.len(), 1);
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-2"));
    }

    #[test]
    fn a_list_that_filters_itself_is_shown_whole() {
        let mut page = ExtensionPage::new(1, "Search");
        page.apply(state(
            1,
            list(vec![item("a", "a", "x"), item("b", "b", "y")], false),
        ));
        page.query = "zzz".into();
        page.refilter();
        assert!(page.extension_filters());
        assert_eq!(page.shown.len(), 2, "the extension decides what matches");
    }

    #[test]
    fn a_rerender_keeps_the_selection_on_the_same_row() {
        let mut page = ExtensionPage::new(1, "Repos");
        page.apply(state(
            1,
            list(vec![item("a", "a", "x"), item("b", "b", "y")], true),
        ));
        page.selected = 1;
        page.apply(state(
            2,
            list(
                vec![
                    item("new", "n", "z"),
                    item("a", "a", "x"),
                    item("b", "b", "y"),
                ],
                true,
            ),
        ));
        assert_eq!(page.selected_item().map(|i| i.title.as_str()), Some("b"));
    }

    #[test]
    fn a_problem_or_an_end_before_any_view_is_said() {
        let mut page = ExtensionPage::new(1, "Grid thing");
        page.apply(ExtensionViewState {
            version: 1,
            view: None,
            problem: Some("Compass cannot draw the extension component <grid> yet".into()),
            ended: false,
            depth: 1,
        });
        assert!(matches!(&page.status, Status::Stopped(why) if why.contains("<grid>")));

        let mut quiet = ExtensionPage::new(2, "Quiet");
        quiet.apply(ExtensionViewState {
            version: 1,
            view: None,
            problem: None,
            ended: true,
            depth: 1,
        });
        assert_eq!(quiet.status, Status::Stopped("Quiet finished".into()));
    }
}
