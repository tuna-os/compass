//! The quicklink list the launcher actually reads from.
//!
//! Ports `ShortcutService`. The store ([`crate::shortcut_store`]) owns the
//! file; this owns the in-memory list built from it, where each entry carries
//! its link already parsed rather than as text.
//!
//! The one rule the whole type is arranged around: **the file is written
//! first, and the list in memory changes only if that succeeded.** A list that
//! shows an edit the file does not have is a launcher that forgets the edit at
//! the next start and cannot say why.

use std::path::Path;

use crate::shortcut::{Link, parse_link};
use crate::shortcut_store::{SerializedShortcut, ShortcutStore};

/// A quicklink as the launcher holds it, with its link parsed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CachedShortcut {
    /// Its stored id.
    pub id: String,
    /// What the user called it.
    pub name: String,
    /// Its icon, as an image reference.
    pub icon: String,
    /// The link, parsed into text and placeholders.
    pub link: Link,
    /// The application that opens it, or `default`.
    pub app: String,
    /// How many times it has been opened.
    pub open_count: i64,
    /// When it was created, in unix seconds.
    pub created_at: u64,
    /// When it was last edited.
    pub updated_at: u64,
    /// When it was last opened, if it ever was.
    pub last_opened_at: Option<u64>,
}

/// `ShortcutService::fromSerialized`.
///
/// The link is parsed here rather than kept as text, which is why the list
/// exists at all: every row that draws a quicklink needs its arguments, and
/// re-parsing on each keystroke of a search would parse every link in the
/// store on every keystroke.
///
/// `lastUsedAt` stays absent when the stored value is absent. Turning it into
/// 0 would date a quicklink nobody has opened to the epoch, which sorts and
/// reads as "opened, a long time ago" rather than "never opened".
#[must_use]
pub fn from_serialized(stored: &SerializedShortcut) -> CachedShortcut {
    CachedShortcut {
        id: stored.id.clone(),
        name: stored.name.clone(),
        icon: stored.icon.clone(),
        link: parse_link(&stored.url),
        app: stored.app.clone(),
        open_count: stored.open_count,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
        last_opened_at: stored.last_used_at,
    }
}

/// The store plus the list built from it.
#[derive(Debug)]
pub struct ShortcutService {
    store: ShortcutStore,
    shortcuts: Vec<CachedShortcut>,
}

impl ShortcutService {
    /// Opens the store and builds the list.
    ///
    /// Any migration from the old database belongs *before* this
    /// ([`crate::shortcut_store::should_migrate`] and
    /// [`crate::shortcut_store::migrate_from_legacy`]), as it does in the C++
    /// constructor: a list built first would be the empty one the migration
    /// was about to fill.
    #[must_use]
    pub fn open(path: &Path) -> Self {
        let store = ShortcutStore::open(path);
        let shortcuts = store.shortcuts().iter().map(from_serialized).collect();

        Self { store, shortcuts }
    }

    /// Everything in the list, in file order.
    #[must_use]
    pub fn shortcuts(&self) -> &[CachedShortcut] {
        &self.shortcuts
    }

    /// The quicklink with this id.
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&CachedShortcut> {
        self.shortcuts.iter().find(|item| item.id == id)
    }

    /// Rebuilds the list from the file.
    pub fn reload(&mut self) {
        self.store.reload();
        self.shortcuts = self.store.shortcuts().iter().map(from_serialized).collect();
    }

    /// `ShortcutService::createShortcut`.
    ///
    /// Answers whether it happened — in the C++ that is also whether the
    /// `shortcutSaved` signal was emitted, and the two are the same question.
    pub fn create(&mut self, name: &str, icon: &str, url: &str, app: &str, now: u64) -> bool {
        let Ok(stored) = self.store.add(name, icon, url, app, now) else {
            return false;
        };

        self.shortcuts.push(from_serialized(&stored));
        true
    }

    /// `ShortcutService::updateShortcut`: the four editable fields.
    ///
    /// `openCount` and `createdAt` are not among them, here or in the store:
    /// editing a quicklink is not using it.
    pub fn update(
        &mut self,
        id: &str,
        name: &str,
        icon: &str,
        url: &str,
        app: &str,
        now: u64,
    ) -> bool {
        let Some(index) = self.shortcuts.iter().position(|item| item.id == id) else {
            return false;
        };

        if self.store.update(id, name, icon, url, app, now).is_err() {
            return false;
        }

        let shortcut = &mut self.shortcuts[index];
        shortcut.name = name.to_owned();
        shortcut.icon = icon.to_owned();
        shortcut.link = parse_link(url);
        shortcut.app = app.to_owned();
        shortcut.updated_at = now;
        true
    }

    /// `ShortcutService::removeShortcut`.
    pub fn remove(&mut self, id: &str) -> bool {
        if self.store.remove(id).is_err() {
            return false;
        }

        self.shortcuts.retain(|item| item.id != id);
        true
    }

    /// `ShortcutService::registerVisit`.
    ///
    /// The count comes back *from the store* rather than being incremented
    /// here, so the two can never drift apart by a missed write: whatever the
    /// file now says is what the list says.
    ///
    /// Both entries are located before anything is written, which is what lets
    /// them be read back without a second check. The C++ looks them up
    /// afterwards and returns false if either has gone — a check for its
    /// service and its database having drifted apart. Here the service owns
    /// the store, so there is nothing to drift, and no mutation could make
    /// that check fail.
    pub fn register_visit(&mut self, id: &str, now: u64) -> bool {
        let (Some(cached), Some(on_disk)) = (
            self.shortcuts.iter().position(|item| item.id == id),
            self.store.shortcuts().iter().position(|item| item.id == id),
        ) else {
            return false;
        };

        if self.store.register_visit(id, now).is_err() {
            return false;
        }

        let stored = &self.store.shortcuts()[on_disk];
        let (open_count, last_used_at) = (stored.open_count, stored.last_used_at);

        let shortcut = &mut self.shortcuts[cached];
        shortcut.open_count = open_count;
        shortcut.last_opened_at = last_used_at;
        true
    }
}
