//! What a person has done with an emoji, and what that does to the list.
//!
//! A port of `GlyphService` (`src/server/src/services/glyph-service/`), minus
//! the fuzzy scorer and the legacy database migration.
//!
//! # The table is generated; only the metadata is a person's
//!
//! Which glyphs exist is [`crate::glyph`], generated from the Unicode data.
//! This is the part that belongs to whoever is using the launcher: how often
//! they picked an emoji, which ones they pinned, what skin tone they meant,
//! and any keyword they added to find it by. Losing it is not a crash — it is
//! an emoji picker that has forgotten them, which is worse than one that never
//! knew.

use serde::{Deserialize, Serialize};

/// The weight a person's own keyword carries when scoring a search.
///
/// Twice the glyph's own name: someone who typed a keyword onto an emoji was
/// saying what they will look for it by.
pub const USER_KEYWORD_WEIGHT: f32 = 2.0;

/// The weight of the glyph's name.
pub const NAME_WEIGHT: f32 = 1.0;

/// The weight of each of the glyph's built-in keywords.
pub const BUILTIN_KEYWORD_WEIGHT: f32 = 0.7;

/// The weight of the glyph's category label.
///
/// Lowest, because it matches a great many glyphs at once — a query of
/// "smileys" should not put every smiley above an exact name match.
pub const CATEGORY_WEIGHT: f32 = 0.5;

/// One glyph's stored metadata.
///
/// # The JSON key is `emoji`, not `character`
///
/// The C++ comment says why: "field key kept for backward compat with existing
/// user data". The service now covers symbols as well as emoji, but renaming
/// the key would make every existing file unreadable and silently reset
/// everyone's pins and counts.
///
/// # The other keys are camelCase
///
/// Glaze writes a struct's members as they are declared, so the C++ file
/// holds `visitCount`, `pinnedAt`, `lastVisitedAt` and `skinTone`. This port
/// first wrote them snake_case, which made the two engines unable to read
/// each other's file; the snake_case spellings are still accepted so a file
/// that build wrote is not lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SerializedEmojiMetadata {
    /// The character this is about.
    pub emoji: String,
    /// How many times it has been picked.
    #[serde(default, alias = "visit_count")]
    pub visit_count: u32,
    /// When it was pinned, in seconds since the epoch.
    #[serde(default, alias = "pinned_at", skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<u64>,
    /// When it was last picked.
    #[serde(
        default,
        alias = "last_visited_at",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_visited_at: Option<u64>,
    /// The skin tone chosen for it, by id.
    #[serde(default, alias = "skin_tone", skip_serializing_if = "Option::is_none")]
    pub skin_tone: Option<String>,
    /// The person's own keywords, space separated as one string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
}

/// The metadata store: a flat list, searched linearly.
///
/// A `Vec` rather than a map, as the C++ has: the list holds only glyphs
/// somebody has actually touched, which is tens of entries, and keeping the
/// file's order stable makes it something a person can read and edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlyphService {
    /// Every entry, in file order.
    entries: Vec<SerializedEmojiMetadata>,
}

impl GlyphService {
    /// A store with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store holding `entries`.
    #[must_use]
    pub fn from_entries(entries: Vec<SerializedEmojiMetadata>) -> Self {
        Self { entries }
    }

    /// Parse a metadata file.
    ///
    /// A file that does not parse yields an empty store, as `load()` does:
    /// it clears `m_entries` rather than keeping what it managed to read,
    /// because half-read metadata would rank the picker by a history that
    /// never happened. `serde_json` is all-or-nothing, so here the type
    /// already rules the partial case out and what the test pins is the other
    /// half — that a bad file is an empty store and not an error.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).map_or_else(|_| Self::new(), Self::from_entries)
    }

    /// Reads the metadata file at `path`.
    ///
    /// A missing file is an empty store, and so is one that does not parse
    /// (see [`Self::from_json`]). The C++ constructor creates the file when it
    /// is missing; here it is created by the first [`Self::save_file`], which
    /// is the first moment there is something to keep.
    #[must_use]
    pub fn load_file(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::from_json(&text))
            .unwrap_or_default()
    }

    /// Writes the store to `path`, creating its directory, through a
    /// temporary file renamed over the old one so a crash mid-write cannot
    /// leave half a file that would then load as nothing.
    ///
    /// # Errors
    ///
    /// The I/O error, when the directory or the file cannot be written.
    pub fn save_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = self.to_json().map_err(std::io::Error::other)?;
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, path)
    }

    /// The entries, for saving.
    #[must_use]
    pub fn entries(&self) -> &[SerializedEmojiMetadata] {
        &self.entries
    }

    /// Serialise the store.
    ///
    /// # Errors
    ///
    /// Returns the serialisation error, which cannot happen for this shape.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.entries)
    }

    /// The entry for `character`, if there is one.
    #[must_use]
    pub fn find(&self, character: &str) -> Option<&SerializedEmojiMetadata> {
        self.entries.iter().find(|entry| entry.emoji == character)
    }

    /// The entry for `character`, creating an empty one if needed.
    fn entry_for(&mut self, character: &str) -> &mut SerializedEmojiMetadata {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.emoji == character)
        {
            return &mut self.entries[index];
        }
        self.entries.push(SerializedEmojiMetadata {
            emoji: character.to_owned(),
            ..SerializedEmojiMetadata::default()
        });
        self.entries.last_mut().expect("just pushed")
    }

    /// The entry for `character` if it exists, for the operations that do not
    /// create one.
    fn find_mut(&mut self, character: &str) -> Option<&mut SerializedEmojiMetadata> {
        self.entries
            .iter_mut()
            .find(|entry| entry.emoji == character)
    }

    /// Record that `character` was picked at `now`.
    pub fn register_visit(&mut self, character: &str, now: u64) {
        let entry = self.entry_for(character);
        entry.visit_count += 1;
        entry.last_visited_at = Some(now);
    }

    /// Pin `character`, creating an entry for it.
    pub fn pin(&mut self, character: &str, now: u64) {
        self.entry_for(character).pinned_at = Some(now);
    }

    /// Unpin `character`.
    ///
    /// Creates nothing: a glyph that was never pinned has nothing to unpin,
    /// and writing an empty entry for it would put it in the file for no
    /// reason. Returns whether anything changed.
    pub fn unpin(&mut self, character: &str) -> bool {
        match self.find_mut(character) {
            Some(entry) => {
                entry.pinned_at = None;
                true
            }
            None => false,
        }
    }

    /// Forget how often `character` was picked, keeping its pin and tone.
    ///
    /// Creates nothing, for the same reason as [`Self::unpin`].
    pub fn reset_ranking(&mut self, character: &str) -> bool {
        match self.find_mut(character) {
            Some(entry) => {
                entry.visit_count = 0;
                entry.last_visited_at = None;
                true
            }
            None => false,
        }
    }

    /// Remember that `character` should be shown in `tone`.
    ///
    /// The id must be one of [`crate::emoji_grid::SKIN_TONES`]' — the C++
    /// takes the `SkinTone` enum, so an unknown id cannot be produced there.
    /// An unknown id is rejected and stores nothing: keeping it would persist
    /// a tone [`crate::emoji_grid::skin_tone_by_id`] resolves to `None`,
    /// silently dropping the person's choice on the next load. Returns whether
    /// anything was stored.
    pub fn set_skin_tone(&mut self, character: &str, tone_id: &str) -> bool {
        if crate::emoji_grid::skin_tone_by_id(tone_id).is_none() {
            return false;
        }
        self.entry_for(character).skin_tone = Some(tone_id.to_owned());
        true
    }

    /// Go back to the default tone.
    pub fn reset_skin_tone(&mut self, character: &str) -> bool {
        match self.find_mut(character) {
            Some(entry) => {
                entry.skin_tone = None;
                true
            }
            None => false,
        }
    }

    /// Set the person's own keywords for `character`.
    ///
    /// Empty text *clears* the keyword rather than storing an empty string, so
    /// a cleared field weighs nothing in the search instead of matching
    /// everything with a zero-length term.
    pub fn set_keywords(&mut self, character: &str, text: &str) {
        let entry = self.entry_for(character);
        entry.keyword = if text.is_empty() {
            None
        } else {
            Some(text.to_owned())
        };
    }

    /// Every glyph worth showing in the "recently used" list.
    ///
    /// Pinned first, most recently pinned before the rest; then by how often
    /// each was picked; then by how recently. `is_known` drops entries whose
    /// character is no longer a glyph this build knows — an old file can name
    /// one, and a row with nothing to draw is worse than a missing row.
    #[must_use]
    pub fn visited(&self, is_known: impl Fn(&str) -> bool) -> Vec<&SerializedEmojiMetadata> {
        let mut sorted: Vec<&SerializedEmojiMetadata> = self
            .entries
            .iter()
            .filter(|entry| entry.pinned_at.is_some() || entry.visit_count > 0)
            .collect();

        sorted.sort_by(|left, right| {
            right
                .pinned_at
                .unwrap_or(0)
                .cmp(&left.pinned_at.unwrap_or(0))
                .then_with(|| right.visit_count.cmp(&left.visit_count))
                .then_with(|| {
                    right
                        .last_visited_at
                        .unwrap_or(0)
                        .cmp(&left.last_visited_at.unwrap_or(0))
                })
        });

        sorted.retain(|entry| is_known(&entry.emoji));
        sorted
    }
}

/// One field a search scores against, and what it is worth.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedField<'a> {
    /// The text.
    pub text: &'a str,
    /// Its weight.
    pub weight: f32,
}

/// The fields one glyph is searched by, in the C++'s order.
///
/// The order matters only for reading; the weights are what decide. A person's
/// own keyword outranks the glyph's name, which outranks its built-in keywords,
/// which outrank its category — because each is a broader thing to have typed
/// than the one before it.
#[must_use]
pub fn search_fields<'a>(
    user_keyword: &'a str,
    name: &'a str,
    category_label: &'a str,
    builtin_keywords: &'a [&'a str],
) -> Vec<WeightedField<'a>> {
    let mut fields = vec![
        WeightedField {
            text: user_keyword,
            weight: USER_KEYWORD_WEIGHT,
        },
        WeightedField {
            text: name,
            weight: NAME_WEIGHT,
        },
        WeightedField {
            text: category_label,
            weight: CATEGORY_WEIGHT,
        },
    ];
    fields.extend(builtin_keywords.iter().map(|keyword| WeightedField {
        text: keyword,
        weight: BUILTIN_KEYWORD_WEIGHT,
    }));
    fields
}

/// Where the C++ keeps the metadata, `$XDG_DATA_HOME/vicinae/emojis/emojis.json`, which the
/// startup migration moves under `compass`
/// (`Omnicast::dataDir() / "emojis" / "emojis.json"`). Shared with it, so a
/// person moving between the engines keeps their pins and counts.
#[must_use]
pub fn default_path() -> Option<std::path::PathBuf> {
    Some(
        crate::xdg_dirs::data_home()?
            .join("compass")
            .join("emojis")
            .join("emojis.json"),
    )
}

/// One glyph's score for `query`, as `GlyphService::search` computes it: the
/// weighted fields of [`search_fields`], then the frecency boost of the
/// glyph's entry, if it has one. `None` when the glyph does not match.
///
/// The boost is added only to something that already matched, so a much-used
/// glyph rises among the results and never appears in results it does not
/// belong to.
#[must_use]
pub fn score(
    glyph: &crate::glyph::Glyph,
    entry: Option<&SerializedEmojiMetadata>,
    query: &compass_search::Query,
    now: i64,
) -> Option<u32> {
    let keyword = entry
        .and_then(|entry| entry.keyword.as_deref())
        .unwrap_or("");
    let fields = search_fields(
        keyword,
        glyph.name,
        glyph.category.label(),
        glyph.keywords(),
    );
    let fields: Vec<compass_search::WeightedField<'_>> = fields
        .iter()
        .filter(|field| !field.text.is_empty())
        .map(|field| compass_search::WeightedField::new(field.text, field.weight))
        .collect();
    let found = compass_search::score_weighted(&fields, query);
    if found.quality < compass_search::MIN_QUALITY || found.score == 0 {
        return None;
    }
    let boost = entry.map_or(0.0, |entry| {
        compass_search::FRECENCY_WEIGHT
            * compass_search::frecency(entry.visit_count, entry.last_visited_at, now)
    });
    // At most 100 plus a boost of at most `FRECENCY_WEIGHT`.
    Some((f64::from(found.score) + boost).round() as u32)
}
