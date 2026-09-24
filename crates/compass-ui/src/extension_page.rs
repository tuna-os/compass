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

/// What a row's icon is drawn from, resolved once per render.
#[derive(Debug, Clone, PartialEq)]
pub enum RowIcon {
    /// A file. A builtin is monochrome and drawn in `tint`, else the text
    /// colour; other art keeps its own colours.
    Art {
        /// The file.
        art: crate::icons::IconArt,
        /// Whether it is one of the builtin, single-colour icons.
        monochrome: bool,
        /// The colour the extension asked for.
        tint: Option<iced::Color>,
    },
    /// A flat colour: a grid cell whose content is one.
    Swatch(iced::Color),
}

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
    /// The extension's `assets` directory, which `Image` paths are relative to.
    pub assets: Option<std::path::PathBuf>,
    /// Whether the launcher is dark, for themed images.
    pub prefers_dark: bool,
    /// Each row's icon, by `(section, item)` like the list.
    pub icons: Vec<Vec<Option<RowIcon>>>,
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
            assets: None,
            prefers_dark: false,
            icons: Vec::new(),
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
                View::Grid(grid) => {
                    self.icons = grid
                        .sections
                        .iter()
                        .map(|s| s.items.iter().map(|c| self.cell_icon(&c.content)).collect())
                        .collect();
                    Box::new(View::List(grid_as_list(grid)))
                }
                other => {
                    self.icons = match &other {
                        View::List(list) => list
                            .sections
                            .iter()
                            .map(|s| {
                                s.items
                                    .iter()
                                    .map(|item| item.icon.as_ref().and_then(|i| self.image_icon(i)))
                                    .collect()
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    Box::new(other)
                }
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

    /// The icon of the row at `(section, item)`.
    #[must_use]
    pub fn icon(&self, section: usize, item: usize) -> Option<&RowIcon> {
        self.icons.get(section)?.get(item)?.as_ref()
    }

    fn cell_icon(&self, content: &compass_extension_api::view::GridContent) -> Option<RowIcon> {
        match content {
            compass_extension_api::view::GridContent::Image(image) => self.image_icon(image),
            compass_extension_api::view::GridContent::Color(color) => {
                color_of(color).map(RowIcon::Swatch)
            }
        }
    }

    /// The file an `Image` is drawn from: a builtin icon, a file in the
    /// extension's assets, or a `file://` URL. Remote URLs and file icons are
    /// not fetched yet, and fall back to the row's initial.
    fn image_icon(&self, image: &compass_extension_api::view::Image) -> Option<RowIcon> {
        use compass_extension_api::view::ImageSource;
        let tint = image.tint.as_ref().and_then(color_of);
        let mut source = &image.source;
        while let ImageSource::Themed { light, dark } = source {
            source = if self.prefers_dark { dark } else { light };
        }
        let (path, monochrome) = match source {
            ImageSource::Builtin(name) => (compass_core::builtin_icon::path(name)?, true),
            ImageSource::Asset(relative) => {
                let path = self.assets.as_ref()?.join(relative);
                (path.is_file().then_some(path)?, false)
            }
            ImageSource::Url(url) => {
                let path = std::path::PathBuf::from(url.strip_prefix("file://")?);
                (path.is_file().then_some(path)?, false)
            }
            ImageSource::FileIcon(_) | ImageSource::Themed { .. } => return None,
        };
        Some(RowIcon::Art {
            art: crate::icons::classify(&path)?,
            monochrome,
            tint,
        })
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

/// A `Color` as drawn: `#rrggbb[aa]`, or one of Raycast's named colours.
fn color_of(color: &compass_extension_api::view::Color) -> Option<iced::Color> {
    use compass_extension_api::view::Color;
    let rgb = |r, g, b| Some(iced::Color::from_rgb8(r, g, b));
    match color {
        Color::Literal(hex) => {
            let hex = hex.strip_prefix('#')?;
            let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            match hex.len() {
                6 => rgb(byte(0)?, byte(2)?, byte(4)?),
                8 => Some(iced::Color::from_rgba8(
                    byte(0)?,
                    byte(2)?,
                    byte(4)?,
                    f32::from(byte(6)?) / 255.0,
                )),
                _ => None,
            }
        }
        Color::Named(name) => match name.as_str() {
            "red" => rgb(0xf4, 0x43, 0x36),
            "orange" => rgb(0xff, 0x98, 0x00),
            "yellow" => rgb(0xff, 0xc1, 0x07),
            "green" => rgb(0x4c, 0xaf, 0x50),
            "blue" => rgb(0x21, 0x96, 0xf3),
            "purple" => rgb(0x9c, 0x27, 0xb0),
            "magenta" => rgb(0xe9, 0x1e, 0x63),
            _ => None,
        },
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
    fn row_icons_resolve_assets_file_urls_themes_and_colour_cells() {
        use compass_extension_api::view::{
            Color, GridContent, GridItem, GridSection, GridView, Image, ImageSource,
        };
        let assets = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("logo.png"), b"png").unwrap();
        std::fs::write(assets.path().join("moon.svg"), b"<svg/>").unwrap();
        let image = |source: ImageSource| {
            let mut image = Image::builtin(String::new());
            image.source = source;
            image
        };
        let mut logo = ListItem::new("logo");
        logo.icon = Some(image(ImageSource::Asset("logo.png".into())));
        let mut themed = ListItem::new("themed");
        themed.icon = Some(image(ImageSource::Themed {
            light: Box::new(ImageSource::Asset("missing.svg".into())),
            dark: Box::new(ImageSource::Url(format!(
                "file://{}",
                assets.path().join("moon.svg").display()
            ))),
        }));
        let mut remote = ListItem::new("remote");
        remote.icon = Some(image(ImageSource::Url("https://example.com/a.png".into())));

        let mut page = ExtensionPage::new(1, "Icons");
        page.assets = Some(assets.path().to_owned());
        page.prefers_dark = true;
        page.apply(state(1, list(vec![logo, themed, remote], true)));
        assert!(matches!(
            page.icon(0, 0),
            Some(RowIcon::Art {
                art: crate::icons::IconArt::Raster(_),
                monochrome: false,
                ..
            })
        ));
        assert!(
            matches!(
                page.icon(0, 1),
                Some(RowIcon::Art {
                    art: crate::icons::IconArt::Vector(_),
                    ..
                })
            ),
            "a dark launcher takes the themed image's dark side"
        );
        assert_eq!(page.icon(0, 2), None, "remote images are not fetched yet");

        let cell = GridItem {
            id: compass_extension_api::id::NodeId::ROOT,
            key: None,
            title: "red".into(),
            subtitle: None,
            content: GridContent::Color(Color::Literal("#ff0000".into())),
            tooltip: None,
            keywords: Vec::new(),
            actions: None,
        };
        page.apply(state(
            2,
            View::Grid(GridView {
                sections: vec![GridSection {
                    items: vec![cell],
                    ..GridSection::default()
                }],
                ..GridView::default()
            }),
        ));
        assert_eq!(
            page.icon(0, 0),
            Some(&RowIcon::Swatch(iced::Color::from_rgb8(0xff, 0, 0)))
        );
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
