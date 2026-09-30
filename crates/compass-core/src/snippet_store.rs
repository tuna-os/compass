//! Where snippets are kept: a JSON file, read whole and written whole.
//!
//! A port of `SnippetDatabase`
//! (`src/server/src/services/snippet/snippet-db.cpp`). The file's shape is the
//! one glaze writes for `snippet::SerializedSnippet`: the text or file is an
//! object with one key (`{"text": …}` or `{"file": …}`, glaze's untagged
//! variant), and an absent `updatedAt` or `expansion` is left out rather than
//! written as `null`.
//!
//! As with [`crate::shortcut_store`], the interesting behaviour is at the
//! edges: a missing file is created empty, a corrupt one reads as no snippets,
//! and a keyword belongs to at most one snippet.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `SnippetDatabase::MAX_SNIPPETS`.
pub const MAX_SNIPPETS: usize = 10_000;

/// The id prefix `generatePrefixedId("snp")` uses.
pub const ID_PREFIX: &str = "snp";

/// What a snippet holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SnippetData {
    /// Text, with placeholders.
    Text {
        /// The text.
        text: String,
    },
    /// A file, copied as a file.
    File {
        /// Its path.
        file: String,
    },
}

impl Default for SnippetData {
    fn default() -> Self {
        Self::Text {
            text: String::new(),
        }
    }
}

/// A snippet's keyboard expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredExpansion {
    /// What typing expands.
    pub keyword: String,
    /// The applications it is limited to; empty for everywhere.
    #[serde(default)]
    pub apps: Vec<String>,
    /// Whether it waits for a word boundary.
    #[serde(default = "default_word")]
    pub word: bool,
}

const fn default_word() -> bool {
    true
}

/// One stored snippet, `snippet::SerializedSnippet` field for field.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SerializedSnippet {
    /// `snp-` and twelve hex characters.
    pub id: String,
    /// What the user called it.
    pub name: String,
    /// Its text or file.
    pub data: SnippetData,
    /// When it was created, in Unix seconds.
    #[serde(rename = "createdAt", default)]
    pub created_at: u64,
    /// When it was last edited, if it ever was.
    #[serde(rename = "updatedAt", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// Its keyboard expansion, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<StoredExpansion>,
}

impl SerializedSnippet {
    /// Its text, for a text snippet.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match &self.data {
            SnippetData::Text { text } => Some(text),
            SnippetData::File { .. } => None,
        }
    }

    /// Its keyword, if it has an expansion.
    #[must_use]
    pub fn keyword(&self) -> Option<&str> {
        self.expansion
            .as_ref()
            .map(|expansion| expansion.keyword.as_str())
    }
}

/// `snippet::SnippetPayload`: what creating or editing sets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnippetPayload {
    /// The name.
    pub name: String,
    /// The text or file.
    pub data: SnippetData,
    /// The expansion, if any.
    pub expansion: Option<StoredExpansion>,
}

/// What went wrong, in the C++'s words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// `tr("Snippet limit reached (%1)")`.
    #[error("Snippet limit reached ({MAX_SNIPPETS})")]
    LimitReached,
    /// `tr("keyword already assigned to \"%1\"")`, naming the snippet that
    /// has it.
    #[error("keyword already assigned to \"{0}\"")]
    KeywordTaken(String),
    /// What `updateSnippet` says.
    #[error("No snippet with that ID")]
    NoSuchId,
    /// What `removeSnippet` says.
    #[error("No such snippet")]
    NoSuchSnippet,
    /// `tr("Failed to save snippets on disk: %1")`.
    #[error("Failed to save snippets on disk: {0}")]
    Write(String),
}

/// The store: the file, and the copy of it in memory.
#[derive(Debug)]
pub struct SnippetStore {
    path: PathBuf,
    snippets: Vec<SerializedSnippet>,
}

impl SnippetStore {
    /// Opens the store at `path`, creating an empty one if there is none; a
    /// file that does not parse reads as no snippets, as `loadSnippets()
    /// .value_or({})` does.
    #[must_use]
    pub fn open(path: &Path) -> Self {
        if !path.is_file() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, "[]");
        }
        let snippets = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path: path.to_owned(),
            snippets,
        }
    }

    /// Every snippet, in file order.
    #[must_use]
    pub fn snippets(&self) -> &[SerializedSnippet] {
        &self.snippets
    }

    /// The snippet with this id.
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&SerializedSnippet> {
        self.snippets.iter().find(|snippet| snippet.id == id)
    }

    /// The snippet this keyword expands.
    #[must_use]
    pub fn find_by_keyword(&self, keyword: &str) -> Option<&SerializedSnippet> {
        self.snippets
            .iter()
            .find(|snippet| snippet.keyword() == Some(keyword))
    }

    /// `SnippetDatabase::addSnippet`. A failed write takes the new snippet
    /// back out, so memory never claims what the file does not have.
    ///
    /// # Errors
    ///
    /// The store is full, the keyword belongs to another snippet, or the
    /// file could not be written.
    pub fn add(&mut self, payload: SnippetPayload, now: u64) -> Result<SerializedSnippet, Error> {
        if self.snippets.len() >= MAX_SNIPPETS {
            return Err(Error::LimitReached);
        }
        if let Some(expansion) = &payload.expansion
            && let Some(existing) = self.find_by_keyword(&expansion.keyword)
        {
            return Err(Error::KeywordTaken(existing.name.clone()));
        }
        let snippet = SerializedSnippet {
            id: crate::shortcut_store::generate_prefixed_id(ID_PREFIX),
            name: payload.name,
            data: payload.data,
            created_at: now,
            updated_at: None,
            expansion: payload.expansion,
        };
        self.snippets.push(snippet.clone());
        if let Err(error) = self.save() {
            self.snippets.pop();
            return Err(error);
        }
        Ok(snippet)
    }

    /// `SnippetDatabase::updateSnippet`: name, data and expansion, and
    /// `updatedAt`; `createdAt` stays.
    ///
    /// # Errors
    ///
    /// The keyword belongs to another snippet, no snippet has `id`, or the
    /// file could not be written.
    pub fn update(&mut self, id: &str, payload: SnippetPayload, now: u64) -> Result<(), Error> {
        if let Some(expansion) = &payload.expansion
            && let Some(existing) = self.find_by_keyword(&expansion.keyword)
            && existing.id != id
        {
            return Err(Error::KeywordTaken(existing.name.clone()));
        }
        let Some(snippet) = self.snippets.iter_mut().find(|snippet| snippet.id == id) else {
            return Err(Error::NoSuchId);
        };
        snippet.name = payload.name;
        snippet.data = payload.data;
        snippet.expansion = payload.expansion;
        snippet.updated_at = Some(now);
        self.save()
    }

    /// `SnippetDatabase::removeSnippet`, answering with what it removed.
    ///
    /// # Errors
    ///
    /// No snippet has `id`, or the file could not be written (the snippet is
    /// then gone from memory until the next open, as in the C++).
    pub fn remove(&mut self, id: &str) -> Result<SerializedSnippet, Error> {
        let Some(index) = self.snippets.iter().position(|snippet| snippet.id == id) else {
            return Err(Error::NoSuchSnippet);
        };
        let removed = self.snippets.remove(index);
        self.save()?;
        Ok(removed)
    }

    fn save(&self) -> Result<(), Error> {
        let text = serde_json::to_string(&self.snippets)
            .map_err(|error| Error::Write(error.to_string()))?;
        std::fs::write(&self.path, text).map_err(|error| Error::Write(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(name: &str, text: &str, keyword: Option<&str>) -> SnippetPayload {
        SnippetPayload {
            name: name.into(),
            data: SnippetData::Text { text: text.into() },
            expansion: keyword.map(|keyword| StoredExpansion {
                keyword: keyword.into(),
                apps: Vec::new(),
                word: true,
            }),
        }
    }

    #[test]
    fn the_file_is_the_shape_glaze_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets/snippets.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"[{"id":"snp-0123456789ab","name":"Sig","data":{"text":"Best,\n{cursor}"},
                "createdAt":1700000000,"expansion":{"keyword":";sig","apps":[],"word":false}},
               {"id":"snp-ba9876543210","name":"Logo","data":{"file":"/home/me/logo.png"},
                "createdAt":1700000001,"updatedAt":1700000002}]"#,
        )
        .unwrap();
        let store = SnippetStore::open(&path);
        assert_eq!(store.snippets().len(), 2);
        assert_eq!(store.snippets()[0].text(), Some("Best,\n{cursor}"));
        assert_eq!(
            store.find_by_keyword(";sig").map(|s| s.name.as_str()),
            Some("Sig")
        );
        assert!(!store.snippets()[0].expansion.as_ref().unwrap().word);
        assert_eq!(
            store.snippets()[1].data,
            SnippetData::File {
                file: "/home/me/logo.png".into()
            }
        );

        let written: serde_json::Value =
            serde_json::to_value(&store.snippets()[0]).expect("serialises");
        assert_eq!(
            written,
            serde_json::json!({"id":"snp-0123456789ab","name":"Sig",
                "data":{"text":"Best,\n{cursor}"},"createdAt":1700000000,
                "expansion":{"keyword":";sig","apps":[],"word":false}}),
            "no null updatedAt"
        );
    }

    #[test]
    fn a_keyword_belongs_to_one_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.json");
        let mut store = SnippetStore::open(&path);
        assert!(store.snippets().is_empty());
        let sig = store.add(text("Sig", "Best", Some(";sig")), 10).unwrap();
        assert!(
            sig.id.starts_with("snp-") && sig.id.len() == 16,
            "{}",
            sig.id
        );
        assert_eq!(
            store.add(text("Other", "x", Some(";sig")), 11),
            Err(Error::KeywordTaken("Sig".into()))
        );
        let addr = store.add(text("Address", "1 Road", None), 12).unwrap();
        assert_eq!(
            store.update(&addr.id, text("Address", "1 Road", Some(";sig")), 13),
            Err(Error::KeywordTaken("Sig".into()))
        );
        store
            .update(&sig.id, text("Signature", "Best regards", Some(";sig")), 14)
            .expect("its own keyword is fine");
        assert_eq!(store.find_by_id(&sig.id).unwrap().updated_at, Some(14));
        assert_eq!(store.find_by_id(&sig.id).unwrap().created_at, 10);

        let reopened = SnippetStore::open(&path);
        assert_eq!(reopened.snippets().len(), 2);
        assert_eq!(reopened.snippets()[0].name, "Signature");

        assert_eq!(store.remove("snp-nothing"), Err(Error::NoSuchSnippet));
        assert_eq!(store.remove(&sig.id).unwrap().name, "Signature");
        assert!(store.find_by_keyword(";sig").is_none());
        assert_eq!(
            store.update(&sig.id, text("x", "y", None), 15),
            Err(Error::NoSuchId)
        );
    }

    #[test]
    fn a_corrupt_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.json");
        std::fs::write(&path, "{nope").unwrap();
        assert!(SnippetStore::open(&path).snippets().is_empty());
    }
}
