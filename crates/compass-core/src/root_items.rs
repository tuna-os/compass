//! The root search: every launchable thing, filtered, scored and ordered.
//!
//! A port of `RootItemManager::search` in
//! `src/server/src/services/root-item-manager/root-item-manager.cpp`. The
//! launcher's root list is not just a fuzzy match: an item carries metadata the
//! user has set (an alias, whether it is enabled, whether it is a favourite) and
//! metadata the launcher has recorded (visit count, last visit), and the search
//! reads all of it.
//!
//! [`crate::rank`] already joins a fuzzy match with frecency for applications.
//! This module adds what the root list has on top: the alias as a searchable
//! field, the enabled/provider/favourite filters, the "an empty query is a
//! frecency ranking" rule, and the stable sort that puts an item whose alias
//! prefixes the query above anything the matcher scored higher.

use compass_search::{
    FRECENCY_WEIGHT, MIN_QUALITY, Query, WeightedField, frecency, score_weighted,
};

/// What the user and the launcher know about a root item.
///
/// The field weights that [`RootItem`] feeds the matcher are the C++ ones:
/// title and unlocalized title 1.0, subtitle 0.5, alias 1.0, each keyword 0.6.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootItemMeta {
    /// The provider that contributed the item ("apps", an extension id, ...).
    pub provider_id: String,
    /// Whether the user has left the item in the root list.
    pub enabled: bool,
    /// The user's alias for the item, if they set one.
    pub alias: Option<String>,
    /// The item's position among the favourites, if it is one.
    pub favorite_idx: Option<usize>,
    /// How many times the item has been opened.
    pub visit_count: u32,
    /// When it was last opened, in unix seconds.
    pub last_visited_at: Option<u64>,
    /// Whether it is one of the fallback commands.
    pub fallback: bool,
    /// The keyboard shortcut the user gave it.
    pub shortcut: Option<String>,
}

/// A searchable entry in the root list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootItem {
    /// The entrypoint id, unique across providers.
    pub id: String,
    /// The primary label; weight 1.0.
    pub title: String,
    /// The untranslated title, when different; the same weight as the title.
    pub unlocalized_title: Option<String>,
    /// The secondary label; weight 0.5.
    pub subtitle: String,
    /// Extra search terms; weight 0.6 each.
    pub keywords: Vec<String>,
    /// Everything the user and the launcher know about it.
    pub meta: RootItemMeta,
}

/// Which items a search considers, and how it breaks ties.
///
/// The defaults are the C++ `RootItemPrefixSearchOptions` defaults: disabled
/// items are hidden, favourites are included, aliases win over score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// Keep items the user has disabled.
    pub include_disabled: bool,
    /// Keep items the user has favourited.
    pub include_favorites: bool,
    /// Put an item whose alias prefixes the query above any higher score.
    pub prioritize_aliased: bool,
    /// Keep only items from this provider.
    pub provider_id: Option<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            include_disabled: false,
            include_favorites: true,
            prioritize_aliased: true,
            provider_id: None,
        }
    }
}

/// An item that survived the filters, with the score that ordered it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredRootItem<'a> {
    /// The item.
    pub item: &'a RootItem,
    /// `match_score + FRECENCY_WEIGHT * frecency`, or the empty-query score.
    pub score: f64,
    /// The item's index in the input slice; the sort is stable in it.
    pub index: usize,
}

impl RootItem {
    /// The item's frecency in `[0, 1]` at `now` (unix seconds).
    #[must_use]
    pub fn frecency(&self, now: i64) -> f64 {
        frecency(self.meta.visit_count, self.meta.last_visited_at, now)
    }

    /// The C++ `SearchableRootItem::fuzzyScore`: 0 means "do not show".
    ///
    /// An empty query scores every item by frecency alone, on the same 0-100
    /// scale: `100 - FRECENCY_WEIGHT` for a never-opened item up to a full 100
    /// for one at maximal frecency. A non-empty query scores the weighted
    /// fields, drops anything below [`MIN_QUALITY`], and adds the same frecency
    /// boost on top.
    #[must_use]
    pub fn fuzzy_score(&self, query: &Query, now: i64) -> f64 {
        self.fuzzy_score_with_boost(query, FRECENCY_WEIGHT * self.frecency(now))
    }

    fn fuzzy_score_with_boost(&self, query: &Query, boost: f64) -> f64 {
        if query.is_empty() {
            return 100.0 - FRECENCY_WEIGHT + boost;
        }

        let alias = self.meta.alias.as_deref().unwrap_or("");
        let mut fields = vec![
            WeightedField::new(&self.title, 1.0),
            WeightedField::new(self.unlocalized_title.as_deref().unwrap_or(""), 1.0),
            WeightedField::new(&self.subtitle, 0.5),
            WeightedField::new(alias, 1.0),
        ];
        fields.extend(self.keywords.iter().map(|kw| WeightedField::new(kw, 0.6)));

        let scored = score_weighted(&fields, query);
        if scored.quality < MIN_QUALITY {
            return 0.0;
        }
        f64::from(scored.score) + boost
    }
}

/// Searches `items`, in the order the C++ `RootItemManager::search` does.
///
/// Filters first (disabled, provider, favourites, then a zero score), then a
/// *stable* sort — the C++ comment says why: "we need stable sort to avoid
/// flickering when updating quickly", so two items that tie keep the order the
/// caller gave them, and a redraw does not shuffle them.
#[must_use]
pub fn search<'a>(
    items: &'a [RootItem],
    pattern: &str,
    opts: &SearchOptions,
    now: i64,
) -> Vec<ScoredRootItem<'a>> {
    search_with_frecency(items, pattern, opts, |_, item| item.frecency(now))
}

pub(crate) fn search_with_frecency<'a>(
    items: &'a [RootItem],
    pattern: &str,
    opts: &SearchOptions,
    frecency: impl Fn(usize, &RootItem) -> f64,
) -> Vec<ScoredRootItem<'a>> {
    let query = Query::new(pattern);

    let mut results: Vec<ScoredRootItem<'a>> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.meta.enabled || opts.include_disabled)
        .filter(|(_, item)| match &opts.provider_id {
            Some(id) => *id == item.meta.provider_id,
            None => true,
        })
        .filter(|(_, item)| item.meta.favorite_idx.is_none() || opts.include_favorites)
        .filter_map(|(index, item)| {
            let score =
                item.fuzzy_score_with_boost(&query, FRECENCY_WEIGHT * frecency(index, item));
            (score != 0.0).then_some(ScoredRootItem { item, score, index })
        })
        .collect();

    results.sort_by(|a, b| {
        if opts.prioritize_aliased {
            // The alias's length, when it is a non-empty prefix of the query.
            let aliased = |s: &ScoredRootItem<'_>| -> Option<usize> {
                let alias = s.item.meta.alias.as_deref()?;
                (!alias.is_empty() && alias.starts_with(pattern)).then_some(alias.len())
            };
            let (aa, ab) = (aliased(a), aliased(b));
            match (aa, ab) {
                // An aliased match beats any score, and the shorter alias wins.
                (Some(x), Some(y)) if x != y => return x.cmp(&y),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                _ => {}
            }
        }
        b.score.total_cmp(&a.score)
    });

    results.extend(typo_fallback(items, pattern, opts, &results, &frecency));
    results
}

/// Items one slip away from `pattern` that the matcher did not reach: fewest
/// edits first, then by frecency, then the shorter title — the one the slip is
/// most of. They come after every real match, so a typo
/// can add an answer but never displace one (#204).
fn typo_fallback<'a>(
    items: &'a [RootItem],
    pattern: &str,
    opts: &SearchOptions,
    matched: &[ScoredRootItem<'a>],
    frecency: &impl Fn(usize, &RootItem) -> f64,
) -> Vec<ScoredRootItem<'a>> {
    if pattern.trim().chars().count() < compass_search::MIN_TYPO_QUERY_CHARS {
        return Vec::new();
    }
    let already: std::collections::HashSet<usize> = matched.iter().map(|hit| hit.index).collect();
    let mut slips: Vec<(usize, f64, ScoredRootItem<'a>)> = items
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            !already.contains(index)
                && (item.meta.enabled || opts.include_disabled)
                && opts
                    .provider_id
                    .as_ref()
                    .is_none_or(|id| *id == item.meta.provider_id)
                && (item.meta.favorite_idx.is_none() || opts.include_favorites)
        })
        .filter_map(|(index, item)| {
            let edits = [
                Some(item.title.as_str()),
                item.unlocalized_title.as_deref(),
                item.meta.alias.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter_map(|text| compass_search::typo_distance(text, pattern))
            .min()?;
            let boost = frecency(index, item);
            // Nonzero, so it reads as a hit, but orders of magnitude below any
            // real match — which clears the quality gate and scores in the tens
            // — even for a caller that merges result lists by score.
            let score = f64::EPSILON * (1.0 + boost);
            Some((edits, boost, ScoredRootItem { item, score, index }))
        })
        .collect();
    slips.sort_by(|a, b| {
        a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)).then_with(|| {
            a.2.item
                .title
                .chars()
                .count()
                .cmp(&b.2.item.title.chars().count())
        })
    });
    slips.into_iter().map(|(_, _, hit)| hit).collect()
}

/// A provider of root items: the "Applications" list, an extension, and so on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provider {
    /// The id items refer to in [`RootItemMeta::provider_id`].
    pub id: String,
    /// The label shown above the group, and matched against the query.
    pub display_name: String,
    /// Transient providers are left out of a grouped search entirely — and so
    /// are their items, because the C++ drops any item whose provider is not in
    /// the map it just built.
    pub transient: bool,
}

/// An item in a provider group, with the enabled flag the group view renders.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupedItem<'a> {
    /// The item.
    pub item: &'a RootItem,
    /// Whether the user has it enabled; carried because a grouped search may
    /// be asked for disabled items and still needs to show them greyed out.
    pub enabled: bool,
    /// The item's own score, which ordered it within the group. Zero here is
    /// normal: an item can be in a group because the *provider's* name matched.
    /// The C++ `ScoredEntry` drops this on the way into the group; keeping it
    /// costs nothing and saves rescoring to explain an order.
    pub score: f64,
}

/// One provider's matches, and the score that ordered the group.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderGroup<'a> {
    /// The provider.
    pub provider: &'a Provider,
    /// The best of every item score and, if the provider's own name matched,
    /// that name's score.
    pub score: f64,
    /// The provider's matching items, best first.
    pub items: Vec<GroupedItem<'a>>,
}

/// Searches `items`, grouped by provider, as `searchGroupedByProvider` does.
///
/// This is a different query from [`search`], not a regrouping of it. Two rules
/// differ and both are deliberate:
///
/// - a provider whose *display name* matches the query contributes **all** of
///   its items, including ones that score zero — typing "applications" lists
///   the applications rather than nothing;
/// - `provider_id` is not applied. The C++ reads `includeDisabled` and
///   `includeFavorites` here but never `providerId`, which makes sense for a
///   view that is already one group per provider.
#[must_use]
pub fn search_grouped_by_provider<'a>(
    items: &'a [RootItem],
    providers: &'a [Provider],
    pattern: &str,
    opts: &SearchOptions,
    now: i64,
) -> Vec<ProviderGroup<'a>> {
    let query = Query::new(pattern);

    // Provider id -> (provider, its display name's score if it matched).
    let scored_providers: Vec<(&Provider, Option<f64>)> = providers
        .iter()
        .filter(|p| !p.transient)
        .map(|p| {
            let m = score_weighted(&[WeightedField::new(&p.display_name, 1.0)], &query);
            (p, m.accepted().then(|| f64::from(m.score)))
        })
        .collect();
    let find = |id: &str| scored_providers.iter().find(|(p, _)| p.id == id);

    // The C++ buckets into an `unordered_map`, so its group order before the
    // final stable sort is a hash order -- two groups with the same score come
    // out in an order that depends on the ids present. Bucketing in
    // first-appearance order instead makes the tie deterministic; see
    // PARITY.md.
    let mut buckets: Vec<ProviderGroup<'a>> = Vec::new();

    for item in items {
        if !item.meta.enabled && !opts.include_disabled {
            continue;
        }
        if item.meta.favorite_idx.is_some() && !opts.include_favorites {
            continue;
        }
        let Some((provider, provider_score)) = find(&item.meta.provider_id) else {
            continue;
        };

        let title_score = item.fuzzy_score(&query, now);
        if title_score <= 0.0 && provider_score.is_none() {
            continue;
        }

        let best = title_score.max(provider_score.unwrap_or(0.0));
        let group = match buckets.iter_mut().find(|g| g.provider.id == provider.id) {
            Some(group) => group,
            None => {
                buckets.push(ProviderGroup {
                    provider,
                    score: 0.0,
                    items: Vec::new(),
                });
                buckets.last_mut().expect("just pushed")
            }
        };
        group.score = group.score.max(best);
        group.items.push(GroupedItem {
            item,
            enabled: item.meta.enabled,
            score: title_score,
        });
    }

    // Stable again, and for the same reason as in `search`.
    for group in &mut buckets {
        group.items.sort_by(|a, b| b.score.total_cmp(&a.score));
    }
    buckets.sort_by(|a, b| b.score.total_cmp(&a.score));

    buckets
}

/// A provider's slice of the config file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfig {
    /// `enabled` for the whole provider, when the user has set it.
    pub enabled: Option<bool>,
    /// Per-entrypoint settings, keyed by the entrypoint half of the id.
    pub entrypoints: std::collections::BTreeMap<String, ItemConfig>,
}

/// One entrypoint's slice of the config file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemConfig {
    /// `enabled`, when the user has set it.
    pub enabled: Option<bool>,
    /// The alias the user gave it.
    pub alias: Option<String>,
    /// The keyboard shortcut the user gave it.
    pub shortcut: Option<String>,
}

/// The parts of the config `mergeConfigWithMetadata` reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootConfig {
    /// Per-provider settings.
    pub providers: std::collections::BTreeMap<String, ProviderConfig>,
    /// Favourites, in the order the user arranged them.
    pub favorites: Vec<String>,
    /// Fallback commands, in order.
    pub fallbacks: Vec<String>,
}

/// The heading over the fallback commands: `Use "<query>" with...`, the
/// query cut at 30 characters with `...` after it, as
/// `RootFallbackSection::sectionName` (which counts bytes, and so can cut a
/// character in half).
#[must_use]
pub fn fallback_heading(query: &str) -> String {
    const MAX_QUERY_LEN: usize = 30;
    let shown = if query.chars().count() > MAX_QUERY_LEN {
        let cut: String = query.chars().take(MAX_QUERY_LEN).collect();
        format!("{cut}...")
    } else {
        query.to_owned()
    };
    format!("Use \"{shown}\" with...")
}

/// An entrypoint id: `provider:entrypoint`.
///
/// The C++ `EntrypointId` serialises with a colon and splits on the **first**
/// one, so an entrypoint may contain colons and a provider may not.
#[must_use]
pub fn entrypoint_id(provider: &str, entrypoint: &str) -> String {
    format!("{provider}:{entrypoint}")
}

/// Splits `provider:entrypoint`, on the first colon.
#[must_use]
pub fn split_entrypoint_id(id: &str) -> Option<(&str, &str)> {
    id.split_once(':')
}

impl RootItem {
    /// Applies `config` to this item's metadata, as `mergeConfigWithMetadata`
    /// does for one item.
    ///
    /// The order is the C++'s and it matters: the item's own default first,
    /// then the user's per-item setting, then the provider's — so a disabled
    /// provider disables an item the user had enabled, while an *enabled*
    /// provider does not re-enable one the user turned off.
    ///
    /// `default_disabled` is `RootItem::isDefaultDisabled()`, which the item
    /// itself answers.
    pub fn merge_config(&mut self, config: &RootConfig, default_disabled: bool) {
        let id = entrypoint_id_of(self);
        let (provider_id, entrypoint) = split_entrypoint_id(&id)
            .map_or((self.meta.provider_id.clone(), String::new()), |(p, e)| {
                (p.to_owned(), e.to_owned())
            });

        let provider_config = config.providers.get(&provider_id);
        let item_config =
            provider_config.and_then(|provider| provider.entrypoints.get(&entrypoint));

        self.meta.provider_id = provider_id;
        self.meta.enabled = !default_disabled;

        // A divergence, declared in PARITY.md: the C++ only *assigns*
        // `favoriteIdx` when the id is in the list, and its metadata map
        // outlives the merge — so unfavouriting an item leaves the old index
        // behind until the launcher restarts, and every search that drops
        // favourites keeps dropping it. Clearing it first is the fix.
        self.meta.favorite_idx = config.favorites.iter().position(|fav| *fav == id);
        self.meta.fallback = config.fallbacks.contains(&id);
        // Cleared first for the same reason: a shortcut taken off an item
        // must not survive until a restart.
        self.meta.shortcut = None;

        if let Some(item) = item_config {
            if let Some(enabled) = item.enabled {
                self.meta.enabled = enabled;
            }
            if let Some(alias) = &item.alias {
                self.meta.alias = Some(alias.clone());
            }
            if let Some(shortcut) = item.shortcut.as_ref().filter(|s| !s.is_empty()) {
                self.meta.shortcut = Some(shortcut.clone());
            }
        }

        // `if (enabled.has_value() && !enabled.value())` -- only a false here
        // has any effect.
        if provider_config.and_then(|provider| provider.enabled) == Some(false) {
            self.meta.enabled = false;
        }
    }

    /// Records an opening, as `RootItemManager::registerVisit` does.
    pub fn register_visit(&mut self, now: u64) {
        self.meta.visit_count += 1;
        self.meta.last_visited_at = Some(now);
    }

    /// Forgets the history, as `RootItemManager::resetRanking` does.
    pub fn reset_ranking(&mut self) {
        self.meta.visit_count = 0;
        self.meta.last_visited_at = None;
    }
}

/// The item's entrypoint id.
fn entrypoint_id_of(item: &RootItem) -> String {
    if item.id.contains(':') {
        item.id.clone()
    } else {
        entrypoint_id(&item.meta.provider_id, &item.id)
    }
}

/// One change to write into the user's config for an entrypoint.
///
/// Every field is optional and only the present ones are written, which is
/// what makes these calls *merges* rather than replacements: setting an alias
/// must not clear a shortcut set earlier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemConfigPatch {
    /// Whether the item is offered.
    pub enabled: Option<bool>,
    /// The word that selects it directly.
    pub alias: Option<String>,
    /// The key that runs it.
    pub shortcut: Option<String>,
}

/// One change to write into the user's config for a provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfigPatch {
    /// Whether the provider's items are offered.
    pub enabled: Option<bool>,
}

impl RootConfig {
    /// Apply a patch to one entrypoint, creating the provider and entrypoint
    /// entries if they are not there yet.
    ///
    /// The config file holds only what the user changed, so most items have no
    /// entry at all until the first time one is written.
    pub fn merge_entrypoint(&mut self, provider: &str, entrypoint: &str, patch: &ItemConfigPatch) {
        let provider_entry = self.providers.entry(provider.to_owned()).or_default();
        let item = provider_entry
            .entrypoints
            .entry(entrypoint.to_owned())
            .or_default();
        if let Some(enabled) = patch.enabled {
            item.enabled = Some(enabled);
        }
        if let Some(alias) = &patch.alias {
            item.alias = Some(alias.clone());
        }
        if let Some(shortcut) = &patch.shortcut {
            item.shortcut = Some(shortcut.clone());
        }
    }

    /// Apply a patch to one provider.
    pub fn merge_provider(&mut self, provider: &str, patch: &ProviderConfigPatch) {
        let entry = self.providers.entry(provider.to_owned()).or_default();
        if let Some(enabled) = patch.enabled {
            entry.enabled = Some(enabled);
        }
    }
}

/// Set an item's alias, in memory and in the config.
///
/// Both halves happen: the metadata is updated so the change shows without
/// reloading, and the config is written so it survives a restart. Doing only
/// the first is the bug this pairing exists to prevent.
pub fn set_alias(item: &mut RootItem, config: &mut RootConfig, alias: &str) {
    item.meta.alias = Some(alias.to_owned());
    if let Some((provider, entrypoint)) = split_entrypoint_id(&entrypoint_id_of(item)) {
        config.merge_entrypoint(
            provider,
            entrypoint,
            &ItemConfigPatch {
                alias: Some(alias.to_owned()),
                ..ItemConfigPatch::default()
            },
        );
    }
}

/// Set an item's shortcut, in memory and in the config.
///
/// # A divergence, declared in PARITY.md
///
/// The C++ is asymmetric here and the asymmetry is a bug. An empty shortcut
/// *resets* the metadata — `m_metadata[id].shortcut.reset()` — but still
/// writes `std::string{""}` into the config. On the next merge that stored
/// empty string is present, so `if (auto shortcut = itemConfig->shortcut)`
/// takes it and the metadata comes back as `Some("")` rather than `None`.
///
/// Clearing a shortcut therefore looks as though it worked until the launcher
/// restarts, and then the item has an empty shortcut instead of none. This
/// port writes `None` for an empty shortcut, so clearing one clears it.
pub fn set_shortcut(item: &mut RootItem, config: &mut RootConfig, shortcut: &str) {
    let value = if shortcut.is_empty() {
        None
    } else {
        Some(shortcut.to_owned())
    };
    item.meta.shortcut = value.clone();
    if let Some((provider, entrypoint)) = split_entrypoint_id(&entrypoint_id_of(item)) {
        let provider_entry = config.providers.entry(provider.to_owned()).or_default();
        let entry = provider_entry
            .entrypoints
            .entry(entrypoint.to_owned())
            .or_default();
        entry.shortcut = value;
    }
}

/// What the root row's action panel changes about one item
/// (`RootSearchActionGenerator`'s actions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootEdit {
    /// Add it to the favourites (at the top, as `setItemAsFavorite` inserts
    /// it) or take it out.
    Favorite(bool),
    /// Swap it with its neighbour in the favourites, below when `down`.
    MoveFavorite {
        /// Towards the end of the list.
        down: bool,
    },
    /// Set the word that selects it directly.
    Alias(String),
    /// Take it out of root search (`disableItem`).
    Disable,
    /// Forget its visits (`resetRanking`); nothing in the configuration.
    ResetRanking,
    /// Give it a keyboard shortcut, in `KeyCombo::to_config_string`'s
    /// spelling, or take it away with an empty one (`setShortcut`).
    Shortcut(String),
    /// Put it in root search or take it out (`setItemEnabled`), as the
    /// settings window's switch does both ways.
    Enabled(bool),
    /// Make it a fallback (first, as `enableFallback` inserts it) or stop it
    /// being one (`disableFallback`). Lives in the configuration's
    /// `fallbacks`, not the root config.
    Fallback(bool),
}

/// `enableFallback` and `disableFallback` over the `fallbacks` list: an
/// enabled one goes first, and enabling one already there or disabling one
/// that is not changes nothing. Returns whether the list changed.
pub fn set_fallback(fallbacks: &mut Vec<String>, id: &str, enabled: bool) -> bool {
    let at = fallbacks.iter().position(|known| known == id);
    match (enabled, at) {
        (true, None) => {
            fallbacks.insert(0, id.to_owned());
            true
        }
        (false, Some(at)) => {
            fallbacks.remove(at);
            true
        }
        _ => false,
    }
}

/// Applies `edit` to `config` for the item `id`, as the C++ root item
/// manager writes it: the favourites list for the first two, the item's
/// entry under its provider for the alias and the switch. Returns whether
/// anything changed; [`RootEdit::ResetRanking`] never changes the config.
///
/// Moving the first favourite up or the last one down changes nothing, as
/// `moveFavoriteUp` and `moveFavoriteDown` refuse; so does removing an item
/// that is not a favourite, or adding one twice.
pub fn apply_edit(config: &mut RootConfig, id: &str, edit: &RootEdit) -> bool {
    match edit {
        RootEdit::Favorite(true) => {
            if config.favorites.iter().any(|favorite| favorite == id) {
                return false;
            }
            config.favorites.insert(0, id.to_owned());
            true
        }
        RootEdit::Favorite(false) => {
            let before = config.favorites.len();
            config.favorites.retain(|favorite| favorite != id);
            config.favorites.len() != before
        }
        RootEdit::MoveFavorite { down } => {
            let Some(at) = config.favorites.iter().position(|favorite| favorite == id) else {
                return false;
            };
            let to = if *down {
                at + 1
            } else {
                let Some(to) = at.checked_sub(1) else {
                    return false;
                };
                to
            };
            if to >= config.favorites.len() {
                return false;
            }
            config.favorites.swap(at, to);
            true
        }
        RootEdit::Alias(alias) => {
            let Some((provider, entrypoint)) = split_entrypoint_id(id) else {
                return false;
            };
            config.merge_entrypoint(
                provider,
                entrypoint,
                &ItemConfigPatch {
                    alias: Some(alias.clone()),
                    ..ItemConfigPatch::default()
                },
            );
            true
        }
        RootEdit::Disable => {
            let Some((provider, entrypoint)) = split_entrypoint_id(id) else {
                return false;
            };
            set_item_enabled(config, provider, entrypoint, false);
            true
        }
        RootEdit::ResetRanking | RootEdit::Fallback(_) => false,
        RootEdit::Shortcut(shortcut) => {
            let Some((provider, entrypoint)) = split_entrypoint_id(id) else {
                return false;
            };
            let entry = config
                .providers
                .entry(provider.to_owned())
                .or_default()
                .entrypoints
                .entry(entrypoint.to_owned())
                .or_default();
            entry.shortcut = (!shortcut.is_empty()).then(|| shortcut.clone());
            true
        }
        RootEdit::Enabled(enabled) => {
            let Some((provider, entrypoint)) = split_entrypoint_id(id) else {
                return false;
            };
            set_item_enabled(config, provider, entrypoint, *enabled);
            true
        }
    }
}

/// The deeplink that launches an item (`CopyItemDeeplink`):
/// `compass://launch/<provider>/<entrypoint>`.
#[must_use]
pub fn deeplink(id: &str) -> Option<String> {
    let (provider, entrypoint) = split_entrypoint_id(id)?;
    Some(format!("compass://launch/{provider}/{entrypoint}"))
}

/// A `compass://launch/...` deeplink (or its `vicinae:` and Raycast
/// spellings), read as `IpcCommandHandler` reads the
/// `launch` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchLink {
    /// The path after `launch/`, percent-decoded, without a trailing slash:
    /// a provider id, or `<provider>/<entrypoint>`.
    pub path: String,
    /// `fallbackText`, typed into the view that opens; `None` when absent or
    /// empty.
    pub fallback_text: Option<String>,
    /// `toggle=true`: close the window instead when it is open.
    pub toggle: bool,
}

/// What a [`LaunchLink`] opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// A search over one provider's items (`ProviderSearchViewHost`).
    Provider(String),
    /// One item, by its `provider:entrypoint` id.
    Entrypoint(String),
}

/// The C++'s answer to a launch path with no `/` that names no provider.
pub const INVALID_LAUNCH_LINK: &str = "Invalid format for launch deeplink";

/// Reads a launch deeplink. `None` for a URL that is not one (another
/// scheme, or another command).
#[must_use]
pub fn parse_launch_link(link: &str) -> Option<LaunchLink> {
    let url = url::Url::parse(link).ok()?;
    if !matches!(
        url.scheme(),
        "compass" | "vicinae" | "raycast" | "com.raycast"
    ) || url.host_str() != Some("launch")
    {
        return None;
    }
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8_lossy()
        .into_owned();
    let path = path.strip_prefix('/').unwrap_or(&path);
    let path = path.strip_suffix('/').unwrap_or(path).to_owned();
    let mut fallback_text = None;
    let mut toggle = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "fallbackText" if !value.is_empty() => fallback_text = Some(value.into_owned()),
            "toggle" => toggle = value == "true",
            _ => {}
        }
    }
    Some(LaunchLink {
        path,
        fallback_text,
        toggle,
    })
}

impl LaunchLink {
    /// What the link opens: the provider its whole path names, else the item
    /// its last `/` splits it into, as `findProviderById` then
    /// `find_last_of('/')` decide.
    ///
    /// # Errors
    ///
    /// [`INVALID_LAUNCH_LINK`] for a path that is neither.
    pub fn target(&self, is_provider: impl Fn(&str) -> bool) -> Result<LaunchTarget, String> {
        if is_provider(&self.path) {
            return Ok(LaunchTarget::Provider(self.path.clone()));
        }
        match self.path.rsplit_once('/') {
            Some((provider, entrypoint)) => Ok(LaunchTarget::Entrypoint(entrypoint_id(
                provider, entrypoint,
            ))),
            None => Err(INVALID_LAUNCH_LINK.to_owned()),
        }
    }
}

/// Turn one item on or off.
///
/// Only the config is written; the metadata follows on the next merge. That is
/// the C++'s behaviour and it is right here rather than an oversight — enabling
/// an item whose *provider* is disabled must not make it appear, and only the
/// merge knows about the provider.
pub fn set_item_enabled(config: &mut RootConfig, provider: &str, entrypoint: &str, enabled: bool) {
    config.merge_entrypoint(
        provider,
        entrypoint,
        &ItemConfigPatch {
            enabled: Some(enabled),
            ..ItemConfigPatch::default()
        },
    );
}

/// Turn a whole provider on or off.
pub fn set_provider_enabled(config: &mut RootConfig, provider: &str, enabled: bool) {
    config.merge_provider(
        provider,
        &ProviderConfigPatch {
            enabled: Some(enabled),
        },
    );
}

/// The provider id applications are contributed under.
///
/// `"applications"`, not `"apps"` — it is half of every application's
/// entrypoint id and so is written into the config file, so the spelling is a
/// stored format rather than a label.
pub const APPS_PROVIDER_ID: &str = "applications";

/// An application's entrypoint half, from its desktop file id.
///
/// The C++ is `m_app->id().remove(".desktop")`, and `QString::remove` takes
/// out **every** occurrence rather than a suffix. For an ordinary id that is
/// the same thing; for one that contains the text twice it is not, and the
/// port keeps the C++'s answer because the result is a stored key.
///
/// With ids joined by dots (see `compass_xdg::scan::desktop_file_id`) that is
/// reachable: a desktop file named `desktop.desktop` in a directory called
/// `my` has the id `my.desktop.desktop`, and both occurrences go.
#[must_use]
pub fn app_entrypoint_id(desktop_id: &str) -> String {
    desktop_id.replace(".desktop", "")
}

/// What an application looks like in the root list.
///
/// Three things are deliberate. The **subtitle is empty**: an application's
/// comment is its description in the settings, not a second line in the
/// launcher, and filling it would give every row a paragraph. The
/// **unlocalized name retains title weight**, so someone who knows an
/// application by its English name finds it on a localised desktop where the
/// title is something else. And `enabled` starts true, because the root item
/// manager's merge is what turns it off — an application is not disabled by
/// being converted.
#[must_use]
pub fn app_root_item(
    desktop_id: &str,
    display_name: &str,
    keywords: &[String],
    unlocalized_name: Option<&str>,
) -> RootItem {
    RootItem {
        id: entrypoint_id(APPS_PROVIDER_ID, &app_entrypoint_id(desktop_id)),
        title: display_name.to_owned(),
        unlocalized_title: unlocalized_name
            .filter(|name| *name != display_name)
            .map(str::to_owned),
        subtitle: String::new(),
        keywords: keywords.to_vec(),
        meta: RootItemMeta {
            provider_id: APPS_PROVIDER_ID.to_owned(),
            enabled: true,
            ..RootItemMeta::default()
        },
    }
}
