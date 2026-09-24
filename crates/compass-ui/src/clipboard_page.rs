//! The clipboard history view: its state, and the decisions it makes.
//!
//! Kept out of `app.rs` so that what the view *decides* — which row is
//! selected after a reload, whether content can be copied back as text,
//! what a row's subtitle says — is testable without a window.

use crate::backend::{ClipboardContent, ClipboardRow, ClipboardRowKind};

/// How many entries one request asks for.
pub const PAGE_SIZE: u32 = 100;

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// A request is in flight and nothing has arrived yet.
    Loading,
    /// Rows have arrived (possibly none).
    Ready,
    /// History cannot be shown, and why.
    Failed(String),
}

/// The clipboard history view's state.
#[derive(Debug, Clone)]
pub struct ClipboardPage {
    /// The filter text.
    pub query: String,
    /// Rows in presentation order.
    pub rows: Vec<ClipboardRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Bumped per request so a late answer to an old query is dropped.
    pub generation: u64,
    /// Why the last Enter did not copy, until the next keystroke.
    pub notice: Option<String>,
}

impl Default for ClipboardPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            status: Status::Loading,
            generation: 0,
            notice: None,
        }
    }
}

impl ClipboardPage {
    /// The selected row, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&ClipboardRow> {
        self.rows.get(self.selected)
    }

    /// Applies an answer. Returns `false` (and changes nothing) for a stale one.
    pub fn apply(&mut self, generation: u64, result: Result<Vec<ClipboardRow>, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.selected = 0;
                self.status = Status::Ready;
            }
            Err(error) => {
                self.rows.clear();
                self.selected = 0;
                self.status = Status::Failed(error);
            }
        }
        true
    }
}

/// The text to put back on the clipboard, or why there is none.
///
/// Text and links go back as text. Images and files need a clipboard write
/// that carries their MIME type, which this view does not do yet, so they are
/// refused with a reason rather than copied as a lossy text rendering.
///
/// # Errors
///
/// A sentence for the user when the content is not text.
pub fn copyable_text(content: &ClipboardContent) -> Result<String, String> {
    let is_text = content.mime_type.starts_with("text/")
        || content.mime_type == "application/x-sh"
        || content.mime_type.contains("uri-list");
    if !is_text {
        return Err(format!(
            "Copying {} back is not supported yet",
            describe_mime(&content.mime_type)
        ));
    }
    String::from_utf8(content.data.clone())
        .map_err(|_| "This entry is not valid text and cannot be copied back".to_owned())
}

fn describe_mime(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "images"
    } else {
        "this kind of entry"
    }
}

/// A row's second line.
#[must_use]
pub fn subtitle(row: &ClipboardRow) -> String {
    let kind = match row.kind {
        ClipboardRowKind::Text => "Text",
        ClipboardRowKind::Link => "Link",
        ClipboardRowKind::Image => "Image",
        ClipboardRowKind::File => "File",
        ClipboardRowKind::Unknown => "Other",
    };
    let mut line = match (&row.kind, &row.url_host) {
        (ClipboardRowKind::Link, Some(host)) => format!("{kind} · {host}"),
        _ => kind.to_owned(),
    };
    if row.pinned {
        line.push_str(" · Pinned");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: ClipboardRowKind) -> ClipboardRow {
        ClipboardRow {
            id: id.into(),
            preview: id.into(),
            kind,
            pinned: false,
            url_host: None,
        }
    }

    #[test]
    fn a_stale_answer_is_dropped() {
        let mut page = ClipboardPage {
            generation: 2,
            ..ClipboardPage::default()
        };
        assert!(!page.apply(1, Ok(vec![row("old", ClipboardRowKind::Text)])));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Loading);
        assert!(page.apply(2, Ok(vec![row("new", ClipboardRowKind::Text)])));
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("new"));
        assert_eq!(page.status, Status::Ready);
    }

    #[test]
    fn a_failure_clears_the_rows_and_says_why() {
        let mut page = ClipboardPage::default();
        page.apply(0, Ok(vec![row("a", ClipboardRowKind::Text)]));
        page.apply(0, Err("no keyring".into()));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Failed("no keyring".into()));
    }

    #[test]
    fn text_goes_back_whole_and_images_are_refused_by_name() {
        let text = ClipboardContent {
            mime_type: "text/plain;charset=utf-8".into(),
            data: "héllo 🚀".as_bytes().to_vec(),
        };
        assert_eq!(copyable_text(&text).as_deref(), Ok("héllo 🚀"));

        let image = ClipboardContent {
            mime_type: "image/png".into(),
            data: vec![0x89, b'P'],
        };
        let refusal = copyable_text(&image).expect_err("an image is not text");
        assert!(refusal.contains("images"), "{refusal}");

        let broken = ClipboardContent {
            mime_type: "text/plain".into(),
            data: vec![0xff, 0xfe],
        };
        assert!(copyable_text(&broken).is_err());
    }

    #[test]
    fn a_link_names_its_host_and_a_pin_is_shown() {
        let mut link = row("l", ClipboardRowKind::Link);
        link.url_host = Some("example.org".into());
        link.pinned = true;
        assert_eq!(subtitle(&link), "Link · example.org · Pinned");
        assert_eq!(subtitle(&row("t", ClipboardRowKind::Text)), "Text");
    }
}
