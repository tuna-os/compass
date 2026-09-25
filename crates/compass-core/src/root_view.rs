//! The root search view's own behaviour.
//!
//! Ports `src/server/src/builtins/root/root-view-host.cpp` and the alias form
//! beside it — the clock in the title bar, the space-bar alias shortcut, and
//! cycling back through past searches with the up arrow.

/// When the clock in the title bar should next be redrawn.
///
/// The delay is `interval - (now % interval)`, which lands the next tick on a
/// multiple of the interval rather than one interval from now. A clock showing
/// minutes therefore updates *on* the minute instead of drifting to whenever
/// the window happened to open, and a clock that has just been re-enabled
/// falls back into step with one short tick rather than being permanently
/// offset.
#[must_use]
pub fn next_clock_tick_secs(now_secs: i64, interval_secs: i64) -> i64 {
    if interval_secs <= 0 {
        return 0;
    }
    interval_secs - now_secs.rem_euclid(interval_secs)
}

/// What the title bar shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockState {
    /// No clock: the title is cleared and the timer stopped.
    ///
    /// Clearing matters as much as stopping. Leaving the last time on screen
    /// would show a clock that had silently stopped, which is worse than no
    /// clock.
    Off,
    /// A clock, redrawn after this many seconds.
    On {
        /// Seconds until the next redraw.
        next_tick_secs: i64,
        /// Whether the user supplied their own format string.
        custom_format: bool,
    },
}

/// Decide what the clock does.
#[must_use]
pub fn clock_state(
    enabled: bool,
    format: Option<&str>,
    interval_secs: i64,
    now_secs: i64,
) -> ClockState {
    if !enabled {
        return ClockState::Off;
    }
    ClockState::On {
        next_tick_secs: next_clock_tick_secs(now_secs, interval_secs),
        custom_format: format.is_some(),
    }
}

/// What pressing space over the root list does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceOutcome {
    /// Run the selected item.
    Activate,
    /// Move the cursor into the command's first input.
    FocusCompleter,
    /// Do nothing special; the space is typed into the search box.
    TypeSpace,
}

/// Decide what space does.
///
/// The shortcut fires only when what has been typed *is* the selected item's
/// alias — compared lowercased, because an alias is stored lowercased and
/// someone typing it with a capital still means it.
///
/// With a completer open the space goes to the completer instead, but only
/// while every completion field is still empty: once something has been typed
/// into one, space is a space, and stealing it would make the arguments
/// unwritable.
///
/// Without a completer the item has to support the shortcut. A no-view command
/// does not, because there would be nothing to show for it.
#[must_use]
pub fn space_outcome(
    query: &str,
    alias: Option<&str>,
    has_completer: bool,
    completion_values_all_empty: bool,
    supports_alias_space_shortcut: bool,
) -> SpaceOutcome {
    let Some(alias) = alias else {
        return SpaceOutcome::TypeSpace;
    };
    if alias != query.to_lowercase() {
        return SpaceOutcome::TypeSpace;
    }
    if has_completer {
        return if completion_values_all_empty {
            SpaceOutcome::FocusCompleter
        } else {
            SpaceOutcome::TypeSpace
        };
    }
    if supports_alias_space_shortcut {
        return SpaceOutcome::Activate;
    }
    SpaceOutcome::TypeSpace
}

/// Whether the up arrow reaches back into the search history.
///
/// Two conditions, and the first is a conflict rather than a preference:
/// wrapping navigation makes the up arrow at the top of the list jump to the
/// bottom, so it cannot also mean "previous search". Where wrapping is on,
/// history is unreachable by design rather than merely unavailable.
///
/// The second is that the selection is already on the first selectable row —
/// so the arrow is not being used to move up through the list, because there
/// is nowhere above to move to.
#[must_use]
pub fn up_cycles_history(
    wrap_navigation: bool,
    selected_index: i32,
    first_selectable: i32,
) -> bool {
    !wrap_navigation && selected_index == first_selectable
}

/// The next history offset for a press of the up arrow.
///
/// The first press takes offset 0 — the most recent search — rather than 1, so
/// one press reaches the last thing typed.
#[must_use]
pub const fn next_history_offset(current: Option<usize>) -> usize {
    match current {
        None => 0,
        Some(offset) => offset + 1,
    }
}

/// Walk back through history, skipping anything equal to what is already there.
///
/// The skip is a loop, not a single step: the history can hold the same query
/// several times in a row, and stopping after one would leave the arrow doing
/// nothing on the second press. Running off the end returns nothing and the
/// box is left as it is.
#[must_use]
pub fn history_entry_at(
    history: &[String],
    offset: usize,
    current_text: &str,
) -> Option<(usize, String)> {
    let mut offset = offset;
    loop {
        let entry = history.get(offset)?;
        if entry != current_text {
            return Some((offset, entry.clone()));
        }
        offset += 1;
    }
}

/// Whether a change of search text should forget where in history we were.
///
/// Typing starts again from the newest entry; the view writing history into
/// the box does not, or the arrow would never get past the first entry.
#[must_use]
pub const fn text_change_resets_history(changed_by_history: bool) -> bool {
    !changed_by_history
}

/// What is recorded when an action runs from the root list.
///
/// The search text is added to the history *whatever* was run, so a search
/// that ended in a file or a calculation is still reachable by the arrow; the
/// visit is recorded only for a root item, because only those have a
/// frecency score to raise.
#[must_use]
pub fn on_action_executed(
    search_text: &str,
    selected_item_id: Option<&str>,
) -> (String, Option<String>) {
    (
        search_text.to_owned(),
        selected_item_id.map(ToOwned::to_owned),
    )
}

/// The alias form's title.
#[must_use]
pub fn alias_form_title(item_title: &str) -> String {
    format!("Set alias - {item_title}")
}

/// The alias the form opens with.
///
/// An item with no alias opens with an empty field rather than a placeholder,
/// because the field is what gets submitted and a placeholder would be
/// submitted as nothing either way.
#[must_use]
pub fn alias_form_initial_value(existing: Option<&str>) -> String {
    existing.unwrap_or_default().to_owned()
}

/// What the alias form says after submitting.
///
/// A failure leaves the form open. It is the only place the text still exists,
/// and popping the view on failure would lose what was typed.
#[must_use]
pub fn alias_submit_outcome(saved: bool) -> (&'static str, &'static str, bool) {
    if saved {
        ("Alias modified", "success", true)
    } else {
        ("Failed to modify alias", "danger", false)
    }
}

/// How many searches the history keeps (`MAX_HISTORY_SIZE`).
pub const MAX_HISTORY_SIZE: usize = 1000;

/// One remembered search.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    /// What was typed.
    pub q: String,
    /// When, in seconds since the epoch.
    #[serde(default)]
    pub ts: u64,
}

/// The root search's history (`SearchHistory`), newest first, in the C++'s
/// own file and shape: `{"entries":[{"q":"…","ts":…}]}` at
/// `$XDG_DATA_HOME/vicinae/search-history.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchHistory {
    /// The searches, newest first.
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
}

impl SearchHistory {
    /// Reads `path`; a missing or unreadable file is an empty history, as
    /// `loadFromDisk` leaves it.
    #[must_use]
    pub fn load_file(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the history to `path`, creating its directory.
    ///
    /// # Errors
    ///
    /// The I/O error when it cannot be written.
    pub fn save_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, path)
    }

    /// Remembers `query` as the newest search (`add`): an empty one is not
    /// kept, an earlier copy of the same text is dropped so it appears once,
    /// and the oldest go beyond [`MAX_HISTORY_SIZE`]. Returns whether
    /// anything changed.
    pub fn add(&mut self, query: &str, now: u64) -> bool {
        if query.is_empty() {
            return false;
        }
        self.entries.retain(|entry| entry.q != query);
        self.entries.insert(
            0,
            HistoryEntry {
                q: query.to_owned(),
                ts: now,
            },
        );
        self.entries.truncate(MAX_HISTORY_SIZE);
        true
    }

    /// The searches, newest first, for [`history_entry_at`].
    #[must_use]
    pub fn queries(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.q.clone()).collect()
    }
}

/// The default location of the search history.
#[must_use]
pub fn default_history_path() -> Option<std::path::PathBuf> {
    Some(
        crate::xdg_dirs::data_home()?
            .join("vicinae")
            .join("search-history.json"),
    )
}
