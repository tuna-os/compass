//! The `Clipboard` half of the extension API.
//!
//! Ports `ExtClipboardService`
//! (`src/server/src/extension/api/clipboard-service.hpp`): four methods that
//! translate the IDL's one loose `ClipboardContent` struct into the tagged
//! content the clipboard actually carries, and back.
//!
//! # Three destinations, not one
//!
//! The C++ reaches for a different thing in each method: `copy` goes to
//! `ClipboardService`, `paste` to `PasteService`, and `clear` goes straight to
//! `QGuiApplication::clipboard()->clear()`, bypassing both. [`Clipboard`] keeps
//! them as three methods rather than folding `clear` into a copy of nothing, so
//! a backend can route each where the C++ routes it.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "Clipboard/copy",
    "Clipboard/paste",
    "Clipboard/clear",
    "Clipboard/readContent",
];

/// What is being put on the clipboard.
///
/// The IDL's `ClipboardContent` has three independent optional fields; the
/// clipboard's own `Clipboard::Content` is a variant. [`parse_content`] is the
/// narrowing, and its precedence is load-bearing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Content {
    /// `Clipboard::NoData`: the extension sent none of the three fields.
    #[default]
    NoData,
    /// `Clipboard::Urls`.
    Urls(Vec<String>),
    /// `Clipboard::Text`.
    Text(String),
    /// `Clipboard::Html`, which may carry a plain-text alternative.
    Html {
        /// The markup.
        html: String,
        /// The plain-text alternative, when the extension sent one.
        text: Option<String>,
    },
}

/// `Clipboard::CopyOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopyOptions {
    /// Marks the entry as concealed, so the history does not show it.
    pub concealed: bool,
}

/// What is on the clipboard now. Mirrors `Clipboard::ReadContent`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadContent {
    /// The plain text, which is empty rather than absent when there is none.
    pub text: String,
    /// The markup, when the selection has any.
    pub html: Option<String>,
    /// The URLs, when the selection is a list of them.
    pub urls: Vec<String>,
}

/// The clipboard an extension can reach.
pub trait Clipboard {
    /// `ClipboardService::copyContent`.
    fn copy(&self, content: Content, options: CopyOptions);

    /// `PasteService::pasteContent` — a different service in the C++, because
    /// pasting types into the focused window rather than filling a selection.
    fn paste(&self, content: Content);

    /// `QGuiApplication::clipboard()->clear()`.
    fn clear(&self);

    /// `ClipboardService::readContent`.
    fn read(&self) -> ReadContent;
}

/// Serves `Clipboard` from one backend.
#[derive(Debug)]
pub struct ClipboardService<C> {
    clipboard: C,
}

impl<C: Clipboard> ClipboardService<C> {
    /// Serves `clipboard`.
    pub const fn new(clipboard: C) -> Self {
        Self { clipboard }
    }

    /// The backend this serves.
    pub const fn clipboard(&self) -> &C {
        &self.clipboard
    }

    /// Answers `call`, or `None` if it is not a `Clipboard` call.
    ///
    /// Every method but `readContent` returns `void`, and so replies `null` —
    /// see the note in [`crate::storage_service`] on why not replying at all
    /// would hang the extension rather than fail it.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let result = match call.method.as_str() {
            "Clipboard/copy" => {
                // `ClipboardOptions::concealed` is a plain `bool` in the IDL, so
                // an omitted one is `false` -- the same value-initialisation the
                // C++ generator relies on.
                let concealed = call
                    .params
                    .get("options")
                    .and_then(|options| options.get("concealed"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                self.clipboard.copy(
                    parse_content(call.params.get("content")),
                    CopyOptions { concealed },
                );
                serde_json::Value::Null
            }
            "Clipboard/paste" => {
                self.clipboard
                    .paste(parse_content(call.params.get("content")));
                serde_json::Value::Null
            }
            "Clipboard/clear" => {
                self.clipboard.clear();
                serde_json::Value::Null
            }
            _ => read_content(&self.clipboard.read()),
        };

        Some(tsapi::reply(id, result))
    }
}

impl<C: Clipboard> tsapi::Service for ClipboardService<C> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The C++ `parseContent`, precedence and all.
///
/// Markup wins over URLs wins over text, and the URL branch is skipped when the
/// list is *empty* — an extension that sends `{urls: [], text: "x"}` copies the
/// text. Text, by contrast, is tested for presence, not emptiness: sending
/// `{text: ""}` copies an empty string, where sending `{}` copies nothing at
/// all.
#[must_use]
pub fn parse_content(content: Option<&serde_json::Value>) -> Content {
    let Some(content) = content else {
        return Content::NoData;
    };
    let field = |name: &str| {
        content
            .get(name)
            .filter(|value| !value.is_null())
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };

    if let Some(html) = field("html") {
        return Content::Html {
            html,
            text: field("text"),
        };
    }

    if let Some(urls) = content.get("urls").and_then(serde_json::Value::as_array)
        && !urls.is_empty()
    {
        return Content::Urls(
            urls.iter()
                .map(|url| url.as_str().unwrap_or_default().to_owned())
                .collect(),
        );
    }

    field("text").map_or(Content::NoData, Content::Text)
}

/// One `ClipboardContent` reply.
///
/// `text` is written unconditionally, as the C++ does — `result.text =
/// rc.text.toStdString()` is not guarded — so an empty clipboard answers
/// `{text: ""}` rather than `{}`. `html` and `urls` are omitted when absent and
/// empty respectively, which is the difference between "no markup" and "empty
/// markup" for an extension reading `content.html`.
fn read_content(content: &ReadContent) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert(
        "text".to_owned(),
        serde_json::Value::String(content.text.clone()),
    );
    if let Some(html) = &content.html {
        out.insert("html".to_owned(), serde_json::Value::String(html.clone()));
    }
    if !content.urls.is_empty() {
        out.insert(
            "urls".to_owned(),
            serde_json::Value::Array(
                content
                    .urls
                    .iter()
                    .map(|url| serde_json::Value::String(url.clone()))
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(out)
}
