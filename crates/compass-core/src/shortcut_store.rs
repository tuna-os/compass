//! Where quicklinks are kept: a JSON file, read whole and written whole.
//!
//! A port of `ShortcutDatabase`
//! (`src/server/src/services/shortcut/shortcut-db.cpp`). [`crate::shortcut`]
//! parses the link; this stores it.
//!
//! The file is the whole database — every mutation rewrites the array — so the
//! interesting behaviour is not SQL but what happens at the edges: a file that
//! is not there, a file that will not parse, a write that fails after the
//! in-memory list already changed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `ShortcutDatabase::MAX_SHORTCUTS`.
pub const MAX_SHORTCUTS: usize = 10_000;

/// The id prefix `generatePrefixedId("sct")` uses.
pub const ID_PREFIX: &str = "sct";

/// How many hex characters follow the prefix (`generatePrefixedId`'s default).
pub const ID_LENGTH: usize = 12;

/// One stored quicklink. Mirrors `shortcut::SerializedShortcut`, field names
/// included: the file is written by whichever engine ran last, so the keys have
/// to match exactly.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SerializedShortcut {
    /// `sct-` and twelve hex characters.
    pub id: String,
    /// What the user called it.
    pub name: String,
    /// Its icon, as an image reference.
    pub icon: String,
    /// The link, placeholders and all.
    pub url: String,
    /// The application that opens it, or `default`.
    pub app: String,
    /// How many times it has been opened.
    #[serde(rename = "openCount", default)]
    pub open_count: i64,
    /// When it was created, in unix seconds.
    #[serde(rename = "createdAt", default)]
    pub created_at: u64,
    /// When it was last edited.
    #[serde(rename = "updatedAt", default)]
    pub updated_at: u64,
    /// When it was last opened, if ever.
    #[serde(
        rename = "lastUsedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_used_at: Option<u64>,
}

/// What went wrong, in the C++'s own words where it has any.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// `std::format("Shortcut limit reached ({})", MAX_SHORTCUTS)`.
    #[error("Shortcut limit reached ({MAX_SHORTCUTS})")]
    LimitReached,
    /// What `updateShortcut` and `registerVisit` say.
    #[error("No shortcut with that ID")]
    NoSuchId,
    /// What `removeShortcut` says — a different sentence for the same
    /// situation, reproduced because an extension or a log may match on it.
    #[error("No such shortcut")]
    NoSuchShortcut,
    /// `std::format("Failed to save shortcuts on disk: {}", ...)`.
    #[error("Failed to save shortcuts on disk: {0}")]
    Write(String),
}

/// The store: the file, and the copy of it in memory.
#[derive(Debug)]
pub struct ShortcutStore {
    path: PathBuf,
    shortcuts: Vec<SerializedShortcut>,
}

impl ShortcutStore {
    /// Opens the store at `path`, creating an empty one if there is none.
    ///
    /// Two C++ behaviours worth knowing:
    ///
    /// * a path that is not a regular file gets its parent created and an empty
    ///   array written — the constructor only *logs* a failure to do that;
    /// * `loadShortcuts().value_or({})` swallows a parse error, so a corrupt
    ///   file reads as no shortcuts. The next write then overwrites it. Both
    ///   are reproduced: a launcher that refused to start because one file was
    ///   truncated would be worse, and this is the behaviour a user's file has
    ///   already been through.
    #[must_use]
    pub fn open(path: &Path) -> Self {
        if !path.is_file() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, "[]");
        }

        let shortcuts = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();

        Self {
            path: path.to_owned(),
            shortcuts,
        }
    }

    /// Everything in the store, in file order.
    #[must_use]
    pub fn shortcuts(&self) -> &[SerializedShortcut] {
        &self.shortcuts
    }

    /// The shortcut with this id.
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&SerializedShortcut> {
        self.shortcuts.iter().find(|item| item.id == id)
    }

    /// `ShortcutDatabase::addShortcut`.
    ///
    /// On a failed write the new entry is removed again, so the list in memory
    /// never claims something the file does not have.
    pub fn add(
        &mut self,
        name: &str,
        icon: &str,
        url: &str,
        app: &str,
        now: u64,
    ) -> Result<SerializedShortcut, Error> {
        self.add_with_id(&generate_id(), name, icon, url, app, now)
    }

    /// [`Self::add`] with the id supplied, for a caller that needs a
    /// reproducible one.
    pub fn add_with_id(
        &mut self,
        id: &str,
        name: &str,
        icon: &str,
        url: &str,
        app: &str,
        now: u64,
    ) -> Result<SerializedShortcut, Error> {
        if self.shortcuts.len() >= MAX_SHORTCUTS {
            return Err(Error::LimitReached);
        }

        let shortcut = SerializedShortcut {
            id: id.to_owned(),
            name: name.to_owned(),
            icon: icon.to_owned(),
            url: url.to_owned(),
            app: app.to_owned(),
            open_count: 0,
            created_at: now,
            updated_at: now,
            last_used_at: None,
        };

        self.shortcuts.push(shortcut.clone());
        if let Err(error) = self.save() {
            self.shortcuts.pop();
            return Err(error);
        }

        Ok(shortcut)
    }

    /// `ShortcutDatabase::updateShortcut`: the four fields and `updatedAt`.
    ///
    /// `openCount`, `createdAt` and `lastUsedAt` are left alone — editing a
    /// quicklink does not reset how often it has been used.
    pub fn update(
        &mut self,
        id: &str,
        name: &str,
        icon: &str,
        url: &str,
        app: &str,
        now: u64,
    ) -> Result<(), Error> {
        let Some(shortcut) = self.shortcuts.iter_mut().find(|item| item.id == id) else {
            return Err(Error::NoSuchId);
        };

        shortcut.name = name.to_owned();
        shortcut.icon = icon.to_owned();
        shortcut.url = url.to_owned();
        shortcut.app = app.to_owned();
        shortcut.updated_at = now;

        self.save()
    }

    /// `ShortcutDatabase::removeShortcut`, which answers with what it removed.
    pub fn remove(&mut self, id: &str) -> Result<SerializedShortcut, Error> {
        let Some(index) = self.shortcuts.iter().position(|item| item.id == id) else {
            return Err(Error::NoSuchShortcut);
        };

        let removed = self.shortcuts.remove(index);
        // No rollback here, unlike `add`: the C++ returns the error and leaves
        // the entry gone from memory, so a failed write loses it until the next
        // reload puts it back.
        self.save()?;
        Ok(removed)
    }

    /// `ShortcutDatabase::registerVisit`.
    pub fn register_visit(&mut self, id: &str, now: u64) -> Result<(), Error> {
        let Some(shortcut) = self.shortcuts.iter_mut().find(|item| item.id == id) else {
            return Err(Error::NoSuchId);
        };

        shortcut.open_count += 1;
        shortcut.last_used_at = Some(now);

        self.save()
    }

    /// `ShortcutDatabase::reload`: the file wins, and a corrupt one empties the
    /// list rather than keeping what was in memory.
    pub fn reload(&mut self) {
        self.shortcuts = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
    }

    /// `ShortcutDatabase::setShortcuts`: the whole array, every time.
    fn save(&self) -> Result<(), Error> {
        let text = serde_json::to_string(&self.shortcuts)
            .map_err(|error| Error::Write(error.to_string()))?;
        std::fs::write(&self.path, text).map_err(|error| Error::Write(error.to_string()))
    }
}

/// `generatePrefixedId("sct")`: the prefix, a dash, and twelve hex characters.
#[must_use]
pub fn generate_id() -> String {
    let mut bytes = [0u8; ID_LENGTH];
    // A failure here would mean the OS has no entropy source at all. The C++
    // uses a `std::random_device`-seeded mt19937 and cannot report one either;
    // falling back to the clock keeps ids unique-ish rather than identical.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (nanos >> (index % 4 * 8)) as u8;
        }
    }

    let mut id = String::with_capacity(ID_PREFIX.len() + 1 + ID_LENGTH);
    id.push_str(ID_PREFIX);
    id.push('-');
    for byte in bytes {
        id.push(char::from_digit(u32::from(byte % 16), 16).unwrap_or('0'));
    }
    id
}
