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
/// title 1.0, subtitle 0.5, alias 1.0, each keyword 0.6.
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
}

/// A searchable entry in the root list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootItem {
    /// The entrypoint id, unique across providers.
    pub id: String,
    /// The primary label; weight 1.0.
    pub title: String,
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
        let boost = FRECENCY_WEIGHT * self.frecency(now);
        if query.is_empty() {
            return 100.0 - FRECENCY_WEIGHT + boost;
        }

        let alias = self.meta.alias.as_deref().unwrap_or("");
        let mut fields = vec![
            WeightedField::new(&self.title, 1.0),
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
            let score = item.fuzzy_score(&query, now);
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

    results
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
