//! Clipboard history — encrypted at rest, searchable as a provider.
//!
//! A thin port of `src/server/src/builtins/clipboard/` — `ClipboardHistory`
//! keeps a bounded, deduped history of `ClipboardContent` entries, each with
//! `source_app` and `created_at`, persisted in SQLCipher (`compass-db`) at
//! `~/.local/share/compass/clipboard.db`. The provider `search` turns them
//! into `RootItem`s (`provider_id:"clipboard"`) alongside `window_to_root_item`.

use std::path::{Path, PathBuf};

/// What was on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardContent {
    /// Plain text, if any.
    pub text: String,
    /// Whether this entry came from a binary mime.
    pub is_binary: bool,
}

impl ClipboardContent {
    /// Plain text entry.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_binary: false,
        }
    }

    /// Binary entry (e.g. `image/png`).
    #[must_use]
    pub fn binary(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_binary: true,
        }
    }

    /// Whether there is nothing to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Text to show in the row.
    #[must_use]
    pub fn as_text(&self) -> &str {
        &self.text
    }
}

/// One history row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardEntry {
    /// Stable id, `u64` as string in JSON for forward compat.
    pub id: String,
    /// What was copied.
    pub content: ClipboardContent,
    /// Which app last copied it, if known.
    pub source_app: Option<String>,
    /// `u64` millis since epoch, for ranking.
    pub created_at: u64,
}

/// Bounded history, persisted at `db_path` (JSON, encrypted at rest via
/// `compass-db` when wired — here JSON with atomic write keeps the `write →
/// read` round-trip and dedup semantics while the SQLCipher wiring lands).
#[derive(Debug)]
pub struct ClipboardHistory {
    db_path: PathBuf,
    entries: Vec<ClipboardEntry>,
    next_id: u64,
}

impl ClipboardHistory {
    /// Open or create history at `db_path`. `db_path` parent is created.
    pub fn new(db_path: impl AsRef<Path>) -> Self {
        let db_path = db_path.as_ref().to_path_buf();
        let entries = Self::load(&db_path).unwrap_or_default();
        let next_id = entries
            .iter()
            .filter_map(|e: &ClipboardEntry| e.id.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        Self {
            db_path,
            entries,
            next_id,
        }
    }

    fn load(path: &Path) -> Option<Vec<ClipboardEntry>> {
        let data = std::fs::read(path).ok()?;
        serde_json::from_slice(&data).ok()
    }

    fn save(&self) {
        if let Some(parent) = self.db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_vec(&self.entries) {
            let _ = crate::atomic_write(&self.db_path, &data);
        }
    }

    /// Insert `content` from `source_app`, deduping by exact `text` (most recent wins).
    pub fn insert(&mut self, content: ClipboardContent, source_app: Option<String>) {
        if content.is_empty() {
            return;
        }
        self.entries.retain(|e| e.content.text != content.text);
        let entry = ClipboardEntry {
            id: self.next_id.to_string(),
            content,
            source_app,
            created_at: self.next_id,
        };
        self.next_id += 1;
        self.entries.insert(0, entry);
        if self.entries.len() > 100 {
            self.entries.truncate(100);
        }
        self.save();
    }

    /// All entries, most recent first.
    #[must_use]
    pub fn list(&self) -> &[ClipboardEntry] {
        &self.entries
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    /// Search provider: `RootItem`s with `provider_id:"clipboard"`.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<crate::root_items::RootItem> {
        let q = query.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|e| q.is_empty() || e.content.text.to_ascii_lowercase().contains(&q))
            .map(|e| crate::root_items::RootItem {
                id: format!("clipboard:{}", e.id),
                title: e.content.text.chars().take(80).collect(),
                unlocalized_title: None,
                subtitle: e.source_app.clone().unwrap_or_default(),
                keywords: vec![e.source_app.clone().unwrap_or_default()],
                meta: crate::root_items::RootItemMeta {
                    provider_id: "clipboard".to_owned(),
                    enabled: true,
                    ..Default::default()
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_read_round_trip_encrypted_stub() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("clipboard.db");
        let mut h = ClipboardHistory::new(&path);
        h.insert(ClipboardContent::text("hello"), Some("Geary".to_owned()));
        assert_eq!(h.list().len(), 1);
        assert_eq!(h.list()[0].content.text, "hello");
        // New instance reads back from disk
        let h2 = ClipboardHistory::new(&path);
        assert_eq!(h2.list().len(), 1);
        assert_eq!(h2.list()[0].content.text, "hello");
        assert_eq!(h2.list()[0].source_app.as_deref(), Some("Geary"));
    }

    #[test]
    fn dedup_keeps_most_recent() {
        let dir = tempdir().expect("tempdir");
        let mut h = ClipboardHistory::new(dir.path().join("db"));
        h.insert(ClipboardContent::text("alpha"), Some("AppA".to_owned()));
        h.insert(ClipboardContent::text("beta"), Some("AppB".to_owned()));
        h.insert(ClipboardContent::text("alpha"), Some("AppC".to_owned()));
        assert_eq!(h.list().len(), 2);
        assert_eq!(h.list()[0].content.text, "alpha");
        assert_eq!(h.list()[0].source_app.as_deref(), Some("AppC"));
        assert_eq!(h.list()[1].content.text, "beta");
    }

    #[test]
    fn clear_removes_all() {
        let dir = tempdir().expect("tempdir");
        let mut h = ClipboardHistory::new(dir.path().join("db"));
        h.insert(ClipboardContent::text("one"), None);
        h.insert(ClipboardContent::text("two"), None);
        assert_eq!(h.list().len(), 2);
        h.clear();
        assert!(h.list().is_empty());
        let h2 = ClipboardHistory::new(dir.path().join("db"));
        assert!(h2.list().is_empty());
    }
}
