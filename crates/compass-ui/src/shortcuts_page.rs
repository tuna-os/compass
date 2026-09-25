//! Shortcuts (quicklinks): the Manage Shortcuts view, and the forms that
//! create, edit and open one.
//!
//! The store and the opening are the engine's; this keeps what the launcher
//! decides on its own — the filter, what each form holds, and which argument
//! fields a link asks for. The forms are [`crate::preferences_page`] forms, so
//! they draw and submit the way every other form in the launcher does.

use compass_core::shortcut_form::{self, Mode};
use compass_core::shortcut_service::CachedShortcut;
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// The Manage Shortcuts view's state.
#[derive(Debug, Clone, Default)]
pub struct ShortcutsPage {
    /// The filter text.
    pub query: String,
    /// Positions in the index's shortcut list that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// Why the last action did not happen, until the next keystroke.
    pub notice: Option<String>,
    /// The detail pane for the selected shortcut, once the engine answered.
    pub detail: Option<Detail>,
}

/// What the detail pane shows beyond the stored shortcut
/// (`ManageShortcutsViewHost::loadDetail`): the link expanded and the
/// application that opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    /// The shortcut's id, so a late answer for another row is dropped.
    pub id: String,
    /// The link with its placeholders filled, or why it could not be.
    pub expanded: Result<String, String>,
    /// The application's name, `(Default)` after it when it is the
    /// default opener rather than a chosen one.
    pub app: Option<String>,
}

/// The pane's metadata, as `loadDetail` lists it: name, application (when
/// one resolves), how often and when it was last opened, and when it was
/// created. `date` writes a Unix time as `QDateTime::toString()` does.
#[must_use]
pub fn detail_fields(
    shortcut: &CachedShortcut,
    app: Option<&str>,
    date: impl Fn(u64) -> String,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![("Name", shortcut.name.clone())];
    if let Some(app) = app {
        fields.push(("Application", app.to_owned()));
    }
    fields.push(("Opened", shortcut.open_count.to_string()));
    fields.push((
        "Last Opened",
        shortcut
            .last_opened_at
            .map_or_else(|| "Never".to_owned(), &date),
    ));
    fields.push(("Created at", date(shortcut.created_at)));
    fields
}

/// The application line: a chosen one by name, the default one with
/// `(Default)` after it.
#[must_use]
pub fn app_label(name: &str, default: bool) -> String {
    if default {
        format!("{name} (Default)")
    } else {
        name.to_owned()
    }
}

impl ShortcutsPage {
    /// Recomputes `shown` over `all` for the current query, keeping the
    /// selection where it was when it still points at a row. An empty query
    /// lists every shortcut in the store's order.
    pub fn refilter(&mut self, all: &[CachedShortcut]) {
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.shown = (0..all.len()).collect();
        } else {
            let mut scored: Vec<(u32, usize)> = all
                .iter()
                .enumerate()
                .filter_map(|(index, shortcut)| {
                    let fields = [
                        WeightedField::new(&shortcut.name, 1.0),
                        WeightedField::new(&shortcut.link.raw, 0.6),
                    ];
                    let found = score_weighted(&fields, &query);
                    (found.quality >= MIN_QUALITY && found.score > 0)
                        .then_some((found.score, index))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.shown = scored.into_iter().map(|(_, index)| index).collect();
        }
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
    }

    /// The selected shortcut's position in the index's list, if any.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.shown.get(self.selected).copied()
    }
}

/// The form's field names, which the submission is read back by.
pub const NAME_FIELD: &str = "name";
/// The link field.
pub const LINK_FIELD: &str = "link";
/// The application field.
pub const APP_FIELD: &str = "app";
/// The icon field.
pub const ICON_FIELD: &str = "icon";

/// The create, edit or duplicate form, as `ShortcutFormViewHost` fills it.
///
/// `applications` are `(title, id)` for the application dropdown, after the
/// `Default` entry; `from_manage` is where saving returns to.
#[must_use]
pub fn form(
    mode: Mode,
    existing: Option<&CachedShortcut>,
    applications: &[(String, String)],
    from_manage: bool,
) -> PreferencesPage {
    let existing = existing.map(|shortcut| shortcut_form::Existing {
        id: shortcut.id.clone(),
        name: shortcut.name.clone(),
        url: shortcut.link.raw.clone(),
        app: shortcut.app.clone(),
        icon: shortcut.icon.clone(),
    });
    let icons = icon_options();
    let initial = shortcut_form::initial(
        mode,
        existing.as_ref(),
        existing
            .as_ref()
            .is_some_and(|e| applications.iter().any(|(_, id)| *id == e.app)),
        existing
            .as_ref()
            .is_some_and(|e| icons.iter().any(|(_, url)| *url == e.icon)),
    );
    let mut app_options = vec![(
        shortcut_form::DEFAULT_ICON_LABEL.to_owned(),
        shortcut_form::DEFAULT_APP.to_owned(),
    )];
    app_options.extend(applications.iter().cloned());
    let title = match mode {
        Mode::Create => "Create Shortcut".to_owned(),
        Mode::Edit | Mode::Duplicate => initial.navigation_title.clone(),
    };
    let fields = vec![
        text_field(NAME_FIELD, "Name", "Shortcut name", false, &initial.name),
        PreferenceInput {
            description: "Use {argument name=\"query\"}, {clipboard}, {selection} or {uuid} \
                          for what is filled in when it opens"
                .to_owned(),
            ..text_field(
                LINK_FIELD,
                "Link",
                "https://google.com/search?q={argument}",
                true,
                &initial.link,
            )
        },
        dropdown(APP_FIELD, "Open with", app_options, &initial.app),
        dropdown(ICON_FIELD, "Icon", icons, &initial.icon),
    ];
    PreferencesPage::new(
        Purpose::ShortcutForm { mode, from_manage },
        existing.map(|e| e.id).unwrap_or_default(),
        title,
        fields,
    )
}

/// The icons the form offers: `Default`, then every built-in icon by name,
/// as `buildIconItems` lists them, valued by their image URL.
#[must_use]
pub fn icon_options() -> Vec<(String, String)> {
    let mut options = vec![(
        shortcut_form::DEFAULT_ICON_LABEL.to_owned(),
        shortcut_form::DEFAULT_ICON.to_owned(),
    )];
    options.extend(compass_core::builtin_icon::names().iter().map(|name| {
        (
            (*name).to_owned(),
            compass_core::image_url::ImageUrl::builtin(*name).to_url(),
        )
    }));
    options
}

fn text_field(
    name: &str,
    title: &str,
    placeholder: &str,
    required: bool,
    value: &str,
) -> PreferenceInput {
    PreferenceInput {
        name: name.to_owned(),
        title: title.to_owned(),
        description: String::new(),
        placeholder: placeholder.to_owned(),
        required,
        kind: PreferenceInputKind::Text,
        value: Some(serde_json::Value::String(value.to_owned())),
    }
}

fn dropdown(
    name: &str,
    title: &str,
    options: Vec<(String, String)>,
    value: &str,
) -> PreferenceInput {
    PreferenceInput {
        name: name.to_owned(),
        title: title.to_owned(),
        description: String::new(),
        placeholder: String::new(),
        required: true,
        kind: PreferenceInputKind::Dropdown { options },
        value: Some(serde_json::Value::String(value.to_owned())),
    }
}

/// What the form holds, by field: `(name, link, app, icon)`.
#[must_use]
pub fn form_values(page: &PreferencesPage) -> (String, String, String, String) {
    let value = |name: &str| {
        page.fields
            .iter()
            .zip(&page.values)
            .find(|(field, _)| field.name == name)
            .map(|(_, value)| match value {
                FieldValue::Text(text) => text.clone(),
                FieldValue::Choice(choice) => choice.clone().unwrap_or_default(),
                FieldValue::Checked(_) | FieldValue::Kept(_) => String::new(),
            })
            .unwrap_or_default()
    };
    (
        value(NAME_FIELD),
        value(LINK_FIELD),
        value(APP_FIELD),
        value(ICON_FIELD),
    )
}

/// The form a shortcut's arguments are entered in before it opens, or `None`
/// when its link takes none.
///
/// One text field per argument, as `RootShortcutItem::arguments` declares
/// them: named after the argument, required unless it has a default, which
/// is shown as the placeholder and used when the field is left empty.
#[must_use]
pub fn arguments_form(shortcut: &CachedShortcut) -> Option<PreferencesPage> {
    if shortcut.link.arguments.is_empty() {
        return None;
    }
    let fields = shortcut
        .link
        .arguments
        .iter()
        .enumerate()
        .map(|(position, argument)| PreferenceInput {
            name: position.to_string(),
            title: argument.name.clone(),
            description: String::new(),
            placeholder: if argument.default_value.is_empty() {
                argument.name.clone()
            } else {
                argument.default_value.clone()
            },
            required: argument.default_value.is_empty(),
            kind: PreferenceInputKind::Text,
            value: None,
        })
        .collect();
    Some(PreferencesPage::new(
        Purpose::ShortcutArguments,
        shortcut.id.clone(),
        display_name(shortcut).to_owned(),
        fields,
    ))
}

/// Whether a shortcut can be a fallback: its link takes exactly one
/// argument, which the query fills (`RootShortcutItem::isSuitableForFallback`).
#[must_use]
pub fn suitable_for_fallback(shortcut: &CachedShortcut) -> bool {
    shortcut.link.arguments.len() == 1
}

/// The argument values an arguments form holds, in order.
#[must_use]
pub fn argument_values(page: &PreferencesPage) -> Vec<String> {
    page.values
        .iter()
        .map(|value| match value {
            FieldValue::Text(text) => text.trim().to_owned(),
            _ => String::new(),
        })
        .collect()
}

/// What a shortcut's row is titled: its name, or its link when it has none
/// (the C++ shows an empty title, which cannot be told apart from a gap).
#[must_use]
pub fn display_name(shortcut: &CachedShortcut) -> &str {
    if shortcut.name.is_empty() {
        &shortcut.link.raw
    } else {
        &shortcut.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::shortcut_service::from_serialized;
    use compass_core::shortcut_store::SerializedShortcut;

    fn shortcut(id: &str, name: &str, url: &str) -> CachedShortcut {
        from_serialized(&SerializedShortcut {
            id: id.into(),
            name: name.into(),
            icon: "icon://omnicast/link".into(),
            url: url.into(),
            app: "default".into(),
            ..SerializedShortcut::default()
        })
    }

    #[test]
    fn the_filter_is_fuzzy_over_name_and_link() {
        let all = [
            shortcut("a", "GitHub Search", "https://github.com/search?q={q}"),
            shortcut("b", "Docs", "https://docs.rs/{crate}"),
        ];
        let mut page = ShortcutsPage::default();
        page.refilter(&all);
        assert_eq!(page.shown, [0, 1]);
        page.query = "gthub".into();
        page.refilter(&all);
        assert_eq!(page.shown, [0], "a typo still finds it");
        page.query = "docs.rs".into();
        page.refilter(&all);
        assert_eq!(page.selected_index(), Some(1), "the link is searched too");
    }

    #[test]
    fn the_pane_lists_what_load_detail_lists_in_its_order() {
        let mut docs = shortcut("sct-1", "Docs", "https://docs.rs/{crate}");
        docs.open_count = 3;
        docs.created_at = 10;
        let date = |at: u64| format!("t{at}");
        assert_eq!(
            detail_fields(&docs, Some("Firefox (Default)"), date),
            [
                ("Name", "Docs".to_owned()),
                ("Application", "Firefox (Default)".to_owned()),
                ("Opened", "3".to_owned()),
                ("Last Opened", "Never".to_owned()),
                ("Created at", "t10".to_owned()),
            ]
        );
        docs.last_opened_at = Some(20);
        let fields = detail_fields(&docs, None, date);
        assert!(!fields.iter().any(|(label, _)| *label == "Application"));
        assert!(fields.contains(&("Last Opened", "t20".to_owned())));
        assert_eq!(app_label("Firefox", true), "Firefox (Default)");
        assert_eq!(app_label("Firefox", false), "Firefox");
    }

    #[test]
    fn editing_prefills_and_duplicating_renames() {
        let existing = shortcut("sct-1", "Docs", "https://docs.rs/{crate}");
        let apps = vec![("Firefox".to_owned(), "firefox.desktop".to_owned())];
        let edit = form(Mode::Edit, Some(&existing), &apps, true);
        assert_eq!(edit.command_id, "sct-1");
        assert_eq!(edit.title, "Edit \"Docs\"");
        assert_eq!(
            form_values(&edit),
            (
                "Docs".to_owned(),
                "https://docs.rs/{crate}".to_owned(),
                "default".to_owned(),
                "icon://omnicast/link".to_owned()
            )
        );
        let copy = form(Mode::Duplicate, Some(&existing), &apps, false);
        assert_eq!(form_values(&copy).0, "Copy of Docs");
        let new = form(Mode::Create, None, &apps, false);
        assert_eq!(
            form_values(&new),
            (
                String::new(),
                String::new(),
                "default".to_owned(),
                "default".to_owned()
            )
        );
        assert_eq!(
            new.submission().err(),
            Some(vec!["Link".to_owned()]),
            "the link is the one text field that is required"
        );
    }

    #[test]
    fn arguments_are_asked_for_in_order_and_defaults_are_optional() {
        let link = shortcut(
            "sct-1",
            "Search",
            "https://x.test/?q={query}&l={argument name=\"lang\" default=\"en\"}",
        );
        let page = arguments_form(&link).expect("two arguments");
        assert_eq!(page.fields.len(), 2);
        assert_eq!(page.fields[0].title, "query");
        assert!(page.fields[0].required);
        assert_eq!(page.fields[1].title, "lang");
        assert_eq!(page.fields[1].placeholder, "en");
        assert!(!page.fields[1].required);
        assert!(arguments_form(&shortcut("b", "Plain", "https://x.test")).is_none());
    }
}
