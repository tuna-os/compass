//! Snippets: the Manage Snippets view, and the forms that create, edit and
//! fill one in.
//!
//! The store and the expansion are the engine's; this keeps the list the
//! engine sent, the filter over it, and what each form holds. The forms are
//! [`crate::preferences_page`] forms, with a text area for the content.

use compass_core::shortcut_form::Mode;
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::{PreferenceInput, PreferenceInputKind, Snippet};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// The list arrived (possibly empty).
    Ready,
    /// Snippets cannot be listed, and why.
    Failed(String),
}

/// The Manage Snippets view's state.
#[derive(Debug, Clone)]
pub struct SnippetsPage {
    /// The filter text.
    pub query: String,
    /// Every snippet, in the store's order.
    pub all: Vec<Snippet>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last action did not happen, until the next keystroke.
    pub notice: Option<String>,
}

impl Default for SnippetsPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            all: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

impl SnippetsPage {
    /// Takes the engine's list, or why there is none.
    pub fn apply(&mut self, result: Result<Vec<Snippet>, String>) {
        match result {
            Ok(snippets) => {
                self.all = snippets;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.all.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
    }

    /// Recomputes `shown` for the current query, fuzzy over the name, the
    /// keyword and the text, keeping the selection in range. An empty query
    /// lists every snippet in the store's order.
    pub fn refilter(&mut self) {
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.shown = (0..self.all.len()).collect();
        } else {
            let mut scored: Vec<(u32, usize)> = self
                .all
                .iter()
                .enumerate()
                .filter_map(|(index, snippet)| {
                    let fields = [
                        WeightedField::new(&snippet.name, 1.0),
                        WeightedField::new(snippet.keyword().unwrap_or_default(), 0.8),
                        WeightedField::new(snippet.text().unwrap_or_default(), 0.4),
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

    /// The selected snippet, if any.
    #[must_use]
    pub fn selected_snippet(&self) -> Option<&Snippet> {
        self.all.get(*self.shown.get(self.selected)?)
    }
}

/// The form's field names, which the submission is read back by.
pub const NAME_FIELD: &str = "name";
/// The content field.
pub const CONTENT_FIELD: &str = "content";
/// The keyword field.
pub const KEYWORD_FIELD: &str = "keyword";
/// The expand-as-word field.
pub const WORD_FIELD: &str = "word";

/// The create, edit or duplicate form, as `SnippetFormViewHost::initialize`
/// fills it: a duplicate is named "Copy of …", and a new snippet expands as a
/// word.
#[must_use]
pub fn form(mode: Mode, existing: Option<&Snippet>, from_manage: bool) -> PreferencesPage {
    let name = match (mode, existing) {
        (Mode::Duplicate, Some(snippet)) => format!("Copy of {}", snippet.name),
        (_, Some(snippet)) => snippet.name.clone(),
        (_, None) => String::new(),
    };
    let content = existing
        .map(|snippet| match &snippet.data {
            compass_core::snippet_store::SnippetData::Text { text } => text.clone(),
            compass_core::snippet_store::SnippetData::File { file } => file.clone(),
        })
        .unwrap_or_default();
    let keyword = existing
        .and_then(Snippet::keyword)
        .unwrap_or_default()
        .to_owned();
    let word = existing
        .and_then(|snippet| snippet.expansion.as_ref())
        .is_none_or(|expansion| expansion.word);
    let title = match (mode, existing) {
        (Mode::Edit, Some(snippet)) => format!("Edit \"{}\"", snippet.name),
        (Mode::Duplicate, Some(snippet)) => format!("Duplicate \"{}\"", snippet.name),
        _ => "Create Snippet".to_owned(),
    };
    let text = |name: &str, title: &str, placeholder: &str, required: bool, value: String| {
        PreferenceInput {
            name: name.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            placeholder: placeholder.to_owned(),
            required,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(value)),
        }
    };
    let fields = vec![
        text(NAME_FIELD, "Name", "Snippet name", true, name),
        PreferenceInput {
            description: "Use {cursor}, {clipboard}, {date format=\"yyyy-MM-dd\"}, {uuid}, \
                          {shell code=\"…\"} or {argument name=\"…\"}"
                .to_owned(),
            kind: PreferenceInputKind::TextArea,
            ..text(CONTENT_FIELD, "Content", "Snippet content", true, content)
        },
        PreferenceInput {
            description: "Typing it expands the snippet, where keyword expansion is available"
                .to_owned(),
            ..text(KEYWORD_FIELD, "Keyword", ";keyword", false, keyword)
        },
        PreferenceInput {
            name: WORD_FIELD.to_owned(),
            title: "Expansion".to_owned(),
            description: String::new(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Checkbox {
                label: "Expand as word".to_owned(),
            },
            value: Some(serde_json::Value::Bool(word)),
        },
    ];
    PreferencesPage::new(
        Purpose::SnippetForm { mode, from_manage },
        existing
            .map(|snippet| snippet.id.clone())
            .unwrap_or_default(),
        title,
        fields,
    )
}

/// What the form holds: `(name, content, keyword, word)`.
#[must_use]
pub fn form_values(page: &PreferencesPage) -> (String, String, String, bool) {
    let value = |name: &str| {
        page.fields
            .iter()
            .zip(&page.values)
            .find(|(field, _)| field.name == name)
            .map(|(_, value)| value.clone())
    };
    let text = |name: &str| match value(name) {
        Some(FieldValue::Text(text)) => text,
        _ => String::new(),
    };
    (
        text(NAME_FIELD),
        text(CONTENT_FIELD),
        text(KEYWORD_FIELD).trim().to_owned(),
        matches!(value(WORD_FIELD), Some(FieldValue::Checked(true))),
    )
}

/// The form a snippet's arguments are entered in before it is copied or
/// pasted, or `None` when its text takes none.
#[must_use]
pub fn arguments_form(snippet: &Snippet, paste: bool) -> Option<PreferencesPage> {
    let text = snippet.text()?;
    let arguments =
        compass_core::snippet_expander::arguments(&compass_core::shortcut::parse_link(text).parts);
    if arguments.is_empty() {
        return None;
    }
    let fields = arguments
        .into_iter()
        .map(|argument| PreferenceInput {
            placeholder: if argument.default_value.is_empty() {
                argument.name.clone()
            } else {
                argument.default_value.clone()
            },
            required: argument.default_value.is_empty(),
            name: argument.name.clone(),
            title: argument.name,
            description: String::new(),
            kind: PreferenceInputKind::Text,
            value: None,
        })
        .collect();
    Some(PreferencesPage::new(
        Purpose::SnippetArguments { paste },
        snippet.id.clone(),
        snippet.name.clone(),
        fields,
    ))
}

/// The argument values an arguments form holds, as `(name, value)`; an empty
/// field takes its default.
#[must_use]
pub fn argument_values(page: &PreferencesPage) -> Vec<(String, String)> {
    page.fields
        .iter()
        .zip(&page.values)
        .map(|(field, value)| {
            let text = match value {
                FieldValue::Text(text) if !text.trim().is_empty() => text.clone(),
                _ if !field.required => field.placeholder.clone(),
                _ => String::new(),
            };
            (field.name.clone(), text)
        })
        .collect()
}

/// A row's second line: its keyword when it has one, else the start of its
/// text on one line.
#[must_use]
pub fn subtitle(snippet: &Snippet) -> String {
    if let Some(keyword) = snippet.keyword() {
        return keyword.to_owned();
    }
    let text = match &snippet.data {
        compass_core::snippet_store::SnippetData::Text { text } => text.as_str(),
        compass_core::snippet_store::SnippetData::File { file } => file.as_str(),
    };
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::snippet_store::{SnippetData, StoredExpansion};

    fn snippet(id: &str, name: &str, text: &str, keyword: Option<&str>) -> Snippet {
        Snippet {
            id: id.into(),
            name: name.into(),
            data: SnippetData::Text { text: text.into() },
            expansion: keyword.map(|keyword| StoredExpansion {
                keyword: keyword.into(),
                apps: vec!["gedit.desktop".into()],
                word: false,
            }),
            ..Snippet::default()
        }
    }

    #[test]
    fn the_filter_is_fuzzy_over_name_keyword_and_text() {
        let mut page = SnippetsPage::default();
        page.apply(Ok(vec![
            snippet("a", "Signature", "Best regards", Some(";sig")),
            snippet("b", "Address", "1 Rue de la Paix", None),
        ]));
        assert_eq!(page.shown, [0, 1]);
        page.query = "adress".into();
        page.refilter();
        assert_eq!(page.shown, [1]);
        page.query = ";sig".into();
        page.refilter();
        assert_eq!(page.selected_snippet().map(|s| s.id.as_str()), Some("a"));
    }

    #[test]
    fn the_form_prefills_and_reads_back() {
        let existing = snippet("snp-1", "Sig", "Best,\n{cursor}", Some(";sig"));
        let edit = form(Mode::Edit, Some(&existing), true);
        assert_eq!(edit.title, "Edit \"Sig\"");
        assert!(edit.has_text_area());
        assert_eq!(
            form_values(&edit),
            ("Sig".into(), "Best,\n{cursor}".into(), ";sig".into(), false)
        );
        let copy = form(Mode::Duplicate, Some(&existing), false);
        assert_eq!(form_values(&copy).0, "Copy of Sig");
        let new = form(Mode::Create, None, false);
        assert_eq!(
            form_values(&new),
            (String::new(), String::new(), String::new(), true),
            "a new snippet expands as a word"
        );
    }

    #[test]
    fn arguments_default_when_left_empty() {
        let with_args = snippet(
            "snp-1",
            "Hello",
            "Hi {name}, re: {argument name=\"topic\" default=\"news\"}",
            None,
        );
        let mut page = arguments_form(&with_args, true).expect("two arguments");
        assert_eq!(page.purpose, Purpose::SnippetArguments { paste: true });
        page.values[0] = FieldValue::Text("Zoë".into());
        assert_eq!(
            argument_values(&page),
            [
                ("name".to_owned(), "Zoë".to_owned()),
                ("topic".to_owned(), "news".to_owned())
            ]
        );
        assert!(
            arguments_form(&snippet("b", "Plain", "{cursor}{clipboard}", None), false).is_none()
        );
    }

    #[test]
    fn the_subtitle_is_the_keyword_or_the_first_words() {
        assert_eq!(subtitle(&snippet("a", "S", "x", Some(";s"))), ";s");
        assert_eq!(subtitle(&snippet("a", "S", "one\n  two", None)), "one two");
    }
}
