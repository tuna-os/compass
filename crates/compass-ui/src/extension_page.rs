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
    /// A confirmation the extension waits on; Enter and Escape answer it.
    pub alert: Option<crate::backend::ExtensionPrompt>,
    /// How many views the extension has pushed; Escape pops above one.
    pub depth: u32,
    /// A detail's Markdown, parsed once per render rather than per frame.
    pub markdown: Vec<iced::widget::markdown::Item>,
    /// A form's values as the person has them, by field name.
    pub form_values: serde_json::Map<String, serde_json::Value>,
    /// How many times the person has edited each field, for `onChange`'s
    /// echo count (ADR-0009).
    pub form_edits: std::collections::BTreeMap<String, u64>,
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
            alert: None,
            depth: 1,
            markdown: Vec::new(),
            form_values: serde_json::Map::new(),
            form_edits: std::collections::BTreeMap::new(),
        }
    }

    /// Takes the engine's latest answer.
    pub fn apply(&mut self, state: crate::backend::ExtensionViewState) {
        self.version = state.version;
        self.alert = state.alert;
        if state.view.is_some() {
            if state.depth != self.depth {
                // A different screen: its search starts empty, as Raycast's does.
                self.query.clear();
                self.selected = 0;
                self.form_values.clear();
                self.form_edits.clear();
            }
            self.depth = state.depth.max(1);
        }
        if let Some(view) = state.view {
            // A grid is shown as rows until the launcher draws image tiles:
            // the same search, selection and actions, one cell per row.
            let view = match *view {
                View::Grid(grid) => Box::new(View::List(grid_as_list(grid))),
                other => Box::new(other),
            };
            let key = self.selected_item().and_then(|item| item.key.clone());
            self.markdown = match view.as_ref() {
                View::Detail(detail) => detail
                    .markdown
                    .as_deref()
                    .map(|text| iced::widget::markdown::parse(text).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            if let View::Form(form) = view.as_ref() {
                self.take_form_values(form);
            }
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
            Some(View::Form(form)) => form.actions.as_ref(),
            _ => None,
        }
    }

    /// The form, when the view is one.
    #[must_use]
    pub fn form(&self) -> Option<&compass_extension_api::view::FormView> {
        match &self.view {
            Some(View::Form(form)) => Some(form),
            _ => None,
        }
    }

    /// What an action is called with: a form's values, so `SubmitForm`'s
    /// `onSubmit` gets them (a plain `onAction` ignores its arguments);
    /// nothing elsewhere.
    #[must_use]
    pub fn action_args(&self) -> Vec<serde_json::Value> {
        if self.form().is_some() {
            vec![serde_json::Value::Object(self.form_values.clone())]
        } else {
            Vec::new()
        }
    }

    /// The person set `name` to `value`: kept, counted, and the field's
    /// `onChange` with its arguments, if it has one.
    pub fn edit_field(
        &mut self,
        name: &str,
        value: serde_json::Value,
    ) -> Option<(HandlerId, Vec<serde_json::Value>)> {
        self.form_values.insert(name.to_owned(), value.clone());
        let count = self.form_edits.entry(name.to_owned()).or_default();
        *count += 1;
        let count = *count;
        let handler = self.form()?.items.iter().find_map(|item| match item {
            compass_extension_api::view::FormItem::Field(field) if field.name == name => {
                field.on_change.clone()
            }
            _ => None,
        })?;
        Some((handler, vec![value, serde_json::Value::from(count)]))
    }

    /// A render's field values, where they do not undo the person's typing:
    /// a starting value only fills an empty field, and an echo only lands
    /// when it answers the latest edit (or is newer, the extension having
    /// set the field itself).
    fn take_form_values(&mut self, form: &compass_extension_api::view::FormView) {
        use compass_extension_api::view::FormItem;
        for item in &form.items {
            let FormItem::Field(field) = item else {
                continue;
            };
            let Some(value) = field.value.as_ref().map(field_json) else {
                continue;
            };
            let edits = self.form_edits.get(&field.name).copied().unwrap_or(0);
            let take = match field.echo {
                Some(echo) => echo.raw() >= edits,
                None => !self.form_values.contains_key(&field.name),
            };
            if take {
                self.form_values.insert(field.name.clone(), value);
            }
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

/// A field value as the extension reads it in `Form.Values`.
fn field_json(value: &compass_extension_api::view::FieldValue) -> serde_json::Value {
    use compass_extension_api::view::FieldValue;
    match value {
        FieldValue::Text(text) | FieldValue::Date(text) => serde_json::Value::String(text.clone()),
        FieldValue::Bool(on) => serde_json::Value::Bool(*on),
        FieldValue::Integer(n) => serde_json::Value::from(*n),
        FieldValue::Paths(all) | FieldValue::Values(all) => {
            serde_json::Value::Array(all.iter().cloned().map(serde_json::Value::String).collect())
        }
        FieldValue::Empty => serde_json::Value::Null,
    }
}

/// `grid`'s cells as list rows, sections and actions kept.
fn grid_as_list(
    grid: compass_extension_api::view::GridView,
) -> compass_extension_api::view::ListView {
    use compass_extension_api::view::{ListSection, ListView};
    ListView {
        navigation_title: grid.navigation_title,
        is_loading: grid.is_loading,
        on_selection_change: grid.on_selection_change,
        search: grid.search,
        actions: grid.actions,
        empty_state: grid.empty_state,
        sections: grid
            .sections
            .into_iter()
            .map(|section| ListSection {
                title: section.title,
                subtitle: section.subtitle,
                items: section
                    .items
                    .into_iter()
                    .map(|cell| {
                        let mut item = ListItem::new(cell.title);
                        item.key = cell.key;
                        item.subtitle = cell.subtitle.or(cell.tooltip);
                        item.keywords = cell.keywords;
                        item.actions = cell.actions;
                        item
                    })
                    .collect(),
                ..ListSection::default()
            })
            .collect(),
        ..ListView::default()
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
            alert: None,
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
    fn a_grid_is_searched_and_acted_on_like_a_list() {
        use compass_extension_api::view::{GridContent, GridItem, GridSection, GridView, Image};
        let cell = |title: &str, handler: &str| GridItem {
            id: compass_extension_api::id::NodeId::ROOT,
            key: Some(title.to_owned()),
            title: title.to_owned(),
            subtitle: None,
            content: GridContent::Image(Image::builtin("star")),
            tooltip: None,
            keywords: Vec::new(),
            actions: Some(ActionPanel::of([Action::new("Copy", handler)])),
        };
        let mut grid = GridView {
            sections: vec![GridSection {
                items: vec![cell("sun", "cb-1"), cell("moon", "cb-2")],
                ..GridSection::default()
            }],
            ..GridView::default()
        };
        grid.search.host_filtering = true;
        let mut page = ExtensionPage::new(1, "Emoji");
        page.apply(state(1, View::Grid(grid)));
        assert_eq!(page.status, Status::Ready);
        assert_eq!(page.shown.len(), 2);
        page.query = "moon".into();
        page.refilter();
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-2"));
    }

    #[test]
    fn a_form_keeps_typing_over_stale_echoes_and_submits_its_values() {
        use compass_extension_api::input::Seq;
        use compass_extension_api::view::{FieldKind, FieldValue, FormField, FormItem, FormView};
        let form = |value: &str, echo: Option<u64>| {
            View::Form(FormView {
                items: vec![FormItem::Field(Box::new(FormField {
                    id: compass_extension_api::id::NodeId::ROOT,
                    name: "title".into(),
                    title: None,
                    error: None,
                    info: None,
                    autofocus: false,
                    value: Some(FieldValue::Text(value.into())),
                    echo: echo.map(Seq::from_raw),
                    on_change: Some(HandlerId::new("cb-change")),
                    kind: FieldKind::Text { placeholder: None },
                }))],
                actions: Some(ActionPanel::of([Action::new("Create", "cb-submit")])),
                ..FormView::default()
            })
        };
        let mut page = ExtensionPage::new(1, "New");
        page.apply(state(1, form("", None)));
        assert_eq!(page.form_values["title"], "");

        let first = page.edit_field("title", "a".into());
        assert_eq!(
            first.map(|(h, args)| (h.0, args)),
            Some(("cb-change".into(), vec!["a".into(), 1.into()]))
        );
        page.edit_field("title", "ab".into());
        page.apply(state(2, form("a", Some(1))));
        assert_eq!(page.form_values["title"], "ab", "a stale echo is ignored");
        page.apply(state(3, form("AB", Some(2))));
        assert_eq!(
            page.form_values["title"], "AB",
            "the answer to the latest edit lands, even when the extension changed it"
        );

        assert_eq!(
            page.primary_action().map(|h| h.0.as_str()),
            Some("cb-submit")
        );
        assert_eq!(page.action_args(), [serde_json::json!({"title": "AB"})]);
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
            alert: None,
        });
        assert!(matches!(&page.status, Status::Stopped(why) if why.contains("<grid>")));

        let mut quiet = ExtensionPage::new(2, "Quiet");
        quiet.apply(ExtensionViewState {
            version: 1,
            view: None,
            problem: None,
            ended: true,
            depth: 1,
            alert: None,
        });
        assert_eq!(quiet.status, Status::Stopped("Quiet finished".into()));
    }
}
