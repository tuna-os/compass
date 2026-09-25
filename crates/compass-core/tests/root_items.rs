//! The root search, read against
//! `src/server/src/services/root-item-manager/root-item-manager.cpp`.
//!
//! Every rule here is one line of `RootItemManager::search` or of
//! `SearchableRootItem::fuzzyScore`; each test was first shown to fail against a
//! deliberate mutation of the rule it pins.

use compass_core::root_items::{RootItem, RootItemMeta, SearchOptions, search};
use compass_search::{FRECENCY_WEIGHT, Query};

/// Unix seconds; a fixed "now" so frecency is not a moving target.
const NOW: i64 = 1_700_000_000;

fn item(id: &str, title: &str) -> RootItem {
    RootItem {
        id: id.to_owned(),
        title: title.to_owned(),
        subtitle: String::new(),
        keywords: Vec::new(),
        unlocalized_title: None,
        meta: RootItemMeta {
            provider_id: "apps".to_owned(),
            enabled: true,
            ..RootItemMeta::default()
        },
    }
}

fn ids<'a>(items: &'a [RootItem], pattern: &str, opts: &SearchOptions) -> Vec<&'a str> {
    search(items, pattern, opts, NOW)
        .into_iter()
        .map(|s| s.item.id.as_str())
        .collect()
}

#[test]
fn an_empty_query_keeps_everything_and_orders_it_by_frecency() {
    // `if (query.empty()) return 100.0 - FRECENCY_WEIGHT + FRECENCY_WEIGHT * frecency();`
    let mut cold = item("cold", "Cold");
    let mut warm = item("warm", "Warm");
    warm.meta.visit_count = 40;
    warm.meta.last_visited_at = Some(NOW as u64);
    cold.meta.visit_count = 0;

    let items = vec![cold, warm];
    assert_eq!(ids(&items, "", &SearchOptions::default()), ["warm", "cold"]);

    // And the scale is the C++ one: a never-opened item sits exactly
    // FRECENCY_WEIGHT below 100, not at 0 and not at 100.
    let scored = search(&items, "", &SearchOptions::default(), NOW);
    let cold = scored.iter().find(|s| s.item.id == "cold").unwrap();
    assert!(
        (cold.score - (100.0 - FRECENCY_WEIGHT)).abs() < 1e-9,
        "{cold:?}"
    );
    let warm = scored.iter().find(|s| s.item.id == "warm").unwrap();
    assert!(
        warm.score > 100.0 - FRECENCY_WEIGHT && warm.score <= 100.0,
        "{warm:?}"
    );
}

#[test]
fn a_disabled_item_is_hidden_unless_asked_for() {
    // `if (!item.meta->enabled && !opts.includeDisabled) continue;`
    let mut off = item("off", "Calculator");
    off.meta.enabled = false;
    let items = vec![off, item("on", "Calendar")];

    assert_eq!(ids(&items, "cal", &SearchOptions::default()), ["on"]);
    let opts = SearchOptions {
        include_disabled: true,
        ..SearchOptions::default()
    };
    let mut found = ids(&items, "cal", &opts);
    found.sort_unstable();
    assert_eq!(found, ["off", "on"]);
}

#[test]
fn a_favourite_is_included_by_default_and_droppable() {
    // `if (item.meta->favoriteIdx.has_value() && !opts.includeFavorites) continue;`
    // Note the polarity: unlike `enabled`, the default *keeps* favourites; the
    // caller that already rendered them at the top passes false to avoid
    // showing them twice.
    let mut fav = item("fav", "Firefox");
    fav.meta.favorite_idx = Some(0);
    let items = vec![fav, item("plain", "Files")];

    let mut both = ids(&items, "fi", &SearchOptions::default());
    both.sort_unstable();
    assert_eq!(both, ["fav", "plain"]);

    let opts = SearchOptions {
        include_favorites: false,
        ..SearchOptions::default()
    };
    assert_eq!(ids(&items, "fi", &opts), ["plain"]);
}

#[test]
fn a_provider_filter_keeps_only_that_provider() {
    // `if (opts.providerId && opts.providerId != item.meta->providerId) continue;`
    let mut ext = item("ext", "Clipboard History");
    ext.meta.provider_id = "extension".to_owned();
    let items = vec![ext, item("app", "Clipboard Manager")];

    let opts = SearchOptions {
        provider_id: Some("extension".to_owned()),
        ..SearchOptions::default()
    };
    assert_eq!(ids(&items, "clip", &opts), ["ext"]);
    assert!(ids(&items, "clip", &SearchOptions::default()).len() == 2);
}

#[test]
fn the_alias_is_a_searchable_field_in_its_own_right() {
    // `std::initializer_list<WS> ss = {{title, 1.0f}, {subtitle, 0.5f}, {alias, 1.0f}};`
    // "zz" appears nowhere in the title, so only the alias can match it.
    let mut aliased = item("aliased", "System Monitor");
    aliased.meta.alias = Some("zz".to_owned());
    let items = vec![aliased];

    assert_eq!(ids(&items, "zz", &SearchOptions::default()), ["aliased"]);
}

#[test]
fn keywords_match_but_weigh_less_than_a_title() {
    // `auto kws = keywords | std::views::transform([](auto &&kw) { return WS{kw, 0.6f}; });`
    let mut by_keyword = item("keyword", "System Monitor");
    by_keyword.keywords = vec!["processes".to_owned()];
    let by_title = item("title", "Processes");
    let items = vec![by_keyword, by_title];

    let found = ids(&items, "processes", &SearchOptions::default());
    assert_eq!(
        found,
        ["title", "keyword"],
        "the 1.0 title outranks the 0.6 keyword"
    );
}

#[test]
fn an_unlocalized_title_scores_like_a_display_title_not_a_keyword() {
    let mut translated = item("translated", "Dateien");
    translated.unlocalized_title = Some("Files".to_owned());
    let display = item("display", "Files");
    let mut keyword = item("keyword", "Other");
    keyword.keywords.push("Files".to_owned());
    let query = Query::new("files");
    assert_eq!(
        translated.fuzzy_score(&query, NOW),
        display.fuzzy_score(&query, NOW)
    );
    assert!(translated.fuzzy_score(&query, NOW) > keyword.fuzzy_score(&query, NOW));
    assert_eq!(
        ids(
            &[keyword, translated, display],
            "files",
            &SearchOptions::default()
        ),
        ["translated", "display", "keyword"]
    );
}

#[test]
fn absent_unlocalized_title_does_not_invent_a_match() {
    assert!(
        ids(
            &[item("translated", "Dateien")],
            "files",
            &SearchOptions::default()
        )
        .is_empty()
    );
}

#[test]
fn a_low_quality_match_is_dropped_rather_than_ranked_low() {
    // `if (score.quality < fuzzy::MIN_QUALITY) return 0;` followed by
    // `if (!fuzzyScore) { continue; }` -- a poor match leaves the list entirely.
    let items = vec![item("gimp", "GNU Image Manipulation Program")];
    assert!(
        ids(&items, "xylophone", &SearchOptions::default()).is_empty(),
        "an unrelated query should match nothing"
    );

    // And the gate is quality, not score: every letter of "imnp" occurs in the
    // title in order, and the matcher scores that 73 -- but the alignment is
    // incoherent, so quality is 0 and the item is dropped rather than shown
    // three quarters of the way up the list.
    let scattered = item("gimp", "GNU Image Manipulation Program");
    let score = scattered.fuzzy_score(&Query::new("imnp"), NOW);
    assert_eq!(score, 0.0, "an incoherent alignment is below MIN_QUALITY");
}

#[test]
fn frecency_boosts_a_match_but_cannot_resurrect_a_non_match() {
    // `return score.score + fuzzy::FRECENCY_WEIGHT * frecency();`
    let mut used = item("used", "Text Editor");
    used.meta.visit_count = 50;
    used.meta.last_visited_at = Some(NOW as u64);
    let fresh = item("fresh", "Text Editor");
    let items = vec![fresh, used];

    assert_eq!(
        ids(&items, "text editor", &SearchOptions::default()),
        ["used", "fresh"],
        "identical titles are separated by history alone"
    );

    // The boost is bounded by FRECENCY_WEIGHT, and it is applied after the
    // quality gate -- a heavily used item that does not match stays out.
    let mut used_elsewhere = item("elsewhere", "Text Editor");
    used_elsewhere.meta.visit_count = 10_000;
    used_elsewhere.meta.last_visited_at = Some(NOW as u64);
    assert!(ids(&[used_elsewhere], "spreadsheet", &SearchOptions::default()).is_empty());
}

#[test]
fn an_alias_prefix_beats_any_score() {
    // `if (aa != ab) { return aa > ab; } // always prioritize matching aliases over score`
    let mut aliased = item("aliased", "Zzz Obscure Thing");
    aliased.meta.alias = Some("ff".to_owned());
    // The rival matches the query exactly *and* has a history, so it outscores
    // an alias match, which tops out at 100 for a never-opened item.
    let mut popular = item("popular", "ff");
    popular.meta.visit_count = 500;
    popular.meta.last_visited_at = Some(NOW as u64);

    let items = vec![popular, aliased];
    let scored = search(&items, "ff", &SearchOptions::default(), NOW);
    assert_eq!(scored[0].item.id, "aliased");
    assert!(
        scored[0].score < scored[1].score,
        "the alias won despite scoring lower: {scored:?}"
    );

    // ... and only when asked. With prioritizeAliased off, score decides.
    let opts = SearchOptions {
        prioritize_aliased: false,
        ..SearchOptions::default()
    };
    assert_eq!(ids(&items, "ff", &opts)[0], "popular");
}

#[test]
fn the_shorter_alias_wins_between_two_aliased_matches() {
    // `if (aa && ab && ...) return a.meta->alias->size() < b.meta->alias->size();`
    let mut long = item("long", "Alpha");
    long.meta.alias = Some("ffx".to_owned());
    let mut short = item("short", "Beta");
    short.meta.alias = Some("ff".to_owned());

    // Give the longer alias the better score, so only the length rule can
    // explain the order.
    long.meta.visit_count = 500;
    long.meta.last_visited_at = Some(NOW as u64);

    let items = vec![long, short];
    assert_eq!(
        ids(&items, "ff", &SearchOptions::default()),
        ["short", "long"]
    );
}

#[test]
fn an_empty_alias_is_not_a_prefix_of_anything() {
    // `!a.meta->alias.value_or("").empty() && a.meta->alias->starts_with(pattern)`
    //
    // The emptiness check only bites on an empty pattern -- every string starts
    // with "" -- and that is exactly the root list a launcher shows before
    // anything is typed. Without it, an item whose alias is set-but-blank is
    // hoisted above the frecency order that view is supposed to be.
    let mut blank = item("blank", "Zzz Obscure Thing");
    blank.meta.alias = Some(String::new());
    let mut popular = item("popular", "Firefox");
    popular.meta.visit_count = 500;
    popular.meta.last_visited_at = Some(NOW as u64);

    let items = vec![blank, popular];
    assert_eq!(
        ids(&items, "", &SearchOptions::default()),
        ["popular", "blank"]
    );
}

#[test]
fn ties_keep_the_input_order() {
    // "we need stable sort to avoid flickering when updating quickly"
    let items: Vec<RootItem> = (0..8)
        .map(|i| item(&format!("item{i}"), "Identical Title"))
        .collect();

    let found = ids(&items, "identical", &SearchOptions::default());
    let expected: Vec<String> = (0..8).map(|i| format!("item{i}")).collect();
    assert_eq!(found, expected);

    // The reported index is the input index, which is what makes the order
    // reproducible for a caller that re-sorts.
    let scored = search(&items, "identical", &SearchOptions::default(), NOW);
    assert_eq!(
        scored.iter().map(|s| s.index).collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
}

// --- `searchGroupedByProvider` -------------------------------------------

use compass_core::root_items::{Provider, ProviderGroup, search_grouped_by_provider};

fn provider(id: &str, display_name: &str) -> Provider {
    Provider {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
        transient: false,
    }
}

fn from(provider_id: &str, id: &str, title: &str) -> RootItem {
    let mut it = item(id, title);
    it.meta.provider_id = provider_id.to_owned();
    it
}

fn group_ids<'a>(groups: &'a [ProviderGroup<'a>]) -> Vec<&'a str> {
    groups.iter().map(|g| g.provider.id.as_str()).collect()
}

#[test]
fn a_transient_provider_and_its_items_are_left_out() {
    // `if (provider->isTransient()) continue;` -- and then
    // `if (!providerById.contains(providerId)) continue;`, which drops the
    // items too, since the transient provider never entered the map.
    let providers = vec![
        Provider {
            transient: true,
            ..provider("fallback", "Fallback")
        },
        provider("apps", "Applications"),
    ];
    let items = vec![
        from("fallback", "f1", "Search Google"),
        from("apps", "a1", "Search Tool"),
    ];

    let groups =
        search_grouped_by_provider(&items, &providers, "search", &SearchOptions::default(), NOW);
    assert_eq!(group_ids(&groups), ["apps"]);
}

#[test]
fn an_item_from_an_unknown_provider_is_dropped() {
    // Same line, other cause: metadata naming a provider that no longer exists.
    let providers = vec![provider("apps", "Applications")];
    let items = vec![
        from("ghost", "g1", "Ghost Tool"),
        from("apps", "a1", "Ghost Writer"),
    ];

    let groups =
        search_grouped_by_provider(&items, &providers, "ghost", &SearchOptions::default(), NOW);
    assert_eq!(group_ids(&groups), ["apps"]);
    assert_eq!(groups[0].items.len(), 1);
}

#[test]
fn a_provider_whose_name_matches_contributes_all_of_its_items() {
    // `if (titleScore <= 0 && !providerMatched) continue;` -- typing the
    // provider's name lists its contents, not nothing.
    let providers = vec![provider("clock", "Clock")];
    let items = vec![
        from("clock", "stopwatch", "Stopwatch"),
        from("clock", "timer", "Timer"),
    ];

    // Neither title matches "clock", so without the provider-name rule the
    // group would be empty.
    let plain = search(&items, "clock", &SearchOptions::default(), NOW);
    assert!(plain.is_empty(), "no title matches the query: {plain:?}");

    let groups =
        search_grouped_by_provider(&items, &providers, "clock", &SearchOptions::default(), NOW);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].items.len(), 2);
    assert!(
        groups[0].items.iter().all(|i| i.score == 0.0),
        "carried on the provider's score alone"
    );
}

#[test]
fn a_group_can_score_on_its_providers_name_alone() {
    // `max(titleScore, providerMatched ? nameScore : 0)` -- the name half.
    // "gmp" scores 100 against the provider name "GMP" and 0 against every one
    // of its item titles, so without the name half the group would rank last.
    let providers = vec![provider("gmp", "GMP"), provider("gnu", "Zzz Other")];
    let items = vec![
        from("gmp", "g1", "Zzz One"),
        // The matcher scores this 71 for "gmp": below the GMP group's 100.
        from("gnu", "n1", "GNU Image Manipulation Program"),
    ];

    let groups =
        search_grouped_by_provider(&items, &providers, "gmp", &SearchOptions::default(), NOW);
    assert_eq!(group_ids(&groups), ["gmp", "gnu"]);
    assert_eq!(groups[0].score, 100.0);
    assert_eq!(groups[1].score, 71.0);
}

#[test]
fn a_group_scores_on_its_best_item() {
    // The same `max`, the other half: neither provider's name matches, so only
    // the item scores can order these groups.
    let providers = vec![provider("weak", "Zzz A"), provider("strong", "Zzz B")];
    let items = vec![
        from("weak", "w1", "GNU Image Manipulation Program"),
        from("strong", "s1", "GMP Tool"),
    ];

    let groups =
        search_grouped_by_provider(&items, &providers, "gmp", &SearchOptions::default(), NOW);
    assert_eq!(group_ids(&groups), ["strong", "weak"]);
    assert_eq!((groups[0].score, groups[1].score), (100.0, 71.0));
}

#[test]
fn items_are_ordered_within_a_group_by_their_own_score() {
    // `std::ranges::stable_sort(bucket.entries, [](a, b) { return a.score > b.score; })`
    let providers = vec![provider("apps", "Applications")];
    let items = vec![
        // 71 against "gmp", where the other scores 100.
        from("apps", "partial", "GNU Image Manipulation Program"),
        from("apps", "exact", "GMP Tool"),
    ];

    let groups =
        search_grouped_by_provider(&items, &providers, "gmp", &SearchOptions::default(), NOW);
    let ids: Vec<&str> = groups[0].items.iter().map(|i| i.item.id.as_str()).collect();
    assert_eq!(ids, ["exact", "partial"]);
}

#[test]
fn the_provider_filter_is_not_applied_to_a_grouped_search() {
    // The C++ reads `includeDisabled` and `includeFavorites` here but never
    // `providerId` -- a view that is already one group per provider does not
    // need it, and a caller that passes it must not silently get one group.
    let providers = vec![
        provider("apps", "Applications"),
        provider("ext", "Extensions"),
    ];
    let items = vec![from("apps", "a1", "Notes"), from("ext", "e1", "Notes Sync")];

    let opts = SearchOptions {
        provider_id: Some("apps".to_owned()),
        ..SearchOptions::default()
    };
    let groups = search_grouped_by_provider(&items, &providers, "notes", &opts, NOW);
    assert_eq!(groups.len(), 2, "providerId is ignored here: {groups:?}");
}

#[test]
fn a_disabled_item_is_hidden_from_a_group_but_keeps_its_flag_when_asked_for() {
    // `if (!item.meta->enabled && !opts.includeDisabled) continue;` and
    // `group.items.push_back({item, entry.enabled})`.
    let providers = vec![provider("apps", "Applications")];
    let mut off = from("apps", "off", "Notes");
    off.meta.enabled = false;
    let items = vec![off, from("apps", "on", "Notebook")];

    let groups =
        search_grouped_by_provider(&items, &providers, "note", &SearchOptions::default(), NOW);
    assert_eq!(groups[0].items.len(), 1);

    let opts = SearchOptions {
        include_disabled: true,
        ..SearchOptions::default()
    };
    let groups = search_grouped_by_provider(&items, &providers, "note", &opts, NOW);
    let flags: Vec<bool> = groups[0].items.iter().map(|i| i.enabled).collect();
    assert_eq!(flags.len(), 2);
    assert!(
        flags.contains(&false),
        "the disabled item is shown, flagged: {groups:?}"
    );
}

#[test]
fn groups_that_tie_keep_first_appearance_order() {
    // A declared divergence: the C++ buckets into an `unordered_map`, so a tie
    // resolves in hash order. Bucketing in the order the items arrive makes the
    // same tie reproducible.
    let providers = vec![
        provider("p0", "P0"),
        provider("p1", "P1"),
        provider("p2", "P2"),
    ];
    let items = vec![
        from("p1", "b", "Identical"),
        from("p2", "c", "Identical"),
        from("p0", "a", "Identical"),
    ];

    let groups = search_grouped_by_provider(
        &items,
        &providers,
        "identical",
        &SearchOptions::default(),
        NOW,
    );
    assert_eq!(group_ids(&groups), ["p1", "p2", "p0"]);
    assert!(
        groups.windows(2).all(|w| w[0].score == w[1].score),
        "the scores do tie"
    );
}

// --- `mergeConfigWithMetadata`, `registerVisit`, `resetRanking` ----------

use compass_core::root_items::{ItemConfig, ProviderConfig, RootConfig, entrypoint_id};

fn config() -> RootConfig {
    RootConfig::default()
}

fn with_provider(mut config: RootConfig, id: &str, provider: ProviderConfig) -> RootConfig {
    config.providers.insert(id.to_owned(), provider);
    config
}

fn item_config(enabled: Option<bool>, alias: Option<&str>, shortcut: Option<&str>) -> ItemConfig {
    ItemConfig {
        enabled,
        alias: alias.map(str::to_owned),
        shortcut: shortcut.map(str::to_owned),
    }
}

/// An item whose id is already `provider:entrypoint`.
fn addressable(provider: &str, entrypoint: &str) -> RootItem {
    let mut it = item(&entrypoint_id(provider, entrypoint), "Title");
    it.meta.provider_id = provider.to_owned();
    it
}

#[test]
fn an_item_starts_from_its_own_default_and_the_user_overrides_it() {
    // `meta.enabled = !item.item->isDefaultDisabled();` then
    // `if (auto enabled = itemConfig->enabled) { meta.enabled = ...; }`.
    let mut off_by_default = addressable("apps", "hidden");
    off_by_default.merge_config(&config(), true);
    assert!(
        !off_by_default.meta.enabled,
        "the item's own default applies"
    );

    let mut turned_on = addressable("apps", "hidden");
    turned_on.merge_config(
        &with_provider(
            config(),
            "apps",
            ProviderConfig {
                enabled: None,
                entrypoints: [("hidden".to_owned(), item_config(Some(true), None, None))].into(),
            },
        ),
        true,
    );
    assert!(
        turned_on.meta.enabled,
        "and the user's setting wins over it"
    );
}

#[test]
fn a_disabled_provider_disables_an_item_the_user_enabled() {
    // `if (auto enabled = providerConfig->enabled; enabled.has_value() &&
    // !enabled.value()) { meta.enabled = false; }` runs *after* the per-item
    // override, so turning a provider off wins.
    let mut it = addressable("ext", "search");
    it.merge_config(
        &with_provider(
            config(),
            "ext",
            ProviderConfig {
                enabled: Some(false),
                entrypoints: [("search".to_owned(), item_config(Some(true), None, None))].into(),
            },
        ),
        false,
    );

    assert!(!it.meta.enabled);
}

#[test]
fn an_enabled_provider_does_not_re_enable_an_item_the_user_turned_off() {
    // The same line only tests for `false`: `enabled == true` does nothing at
    // all, in either direction.
    let provider = |item_enabled: bool| {
        with_provider(
            config(),
            "ext",
            ProviderConfig {
                enabled: Some(true),
                entrypoints: [(
                    "search".to_owned(),
                    item_config(Some(item_enabled), None, None),
                )]
                .into(),
            },
        )
    };

    let mut off = addressable("ext", "search");
    off.merge_config(&provider(false), false);
    assert!(
        !off.meta.enabled,
        "an enabled provider does not turn it back on"
    );

    let mut on = addressable("ext", "search");
    on.merge_config(&provider(true), false);
    assert!(on.meta.enabled, "nor does it turn an enabled one off");
}

#[test]
fn the_alias_and_shortcut_come_from_the_item_config() {
    let mut it = addressable("apps", "firefox.desktop");
    it.merge_config(
        &with_provider(
            config(),
            "apps",
            ProviderConfig {
                enabled: None,
                entrypoints: [(
                    "firefox.desktop".to_owned(),
                    item_config(None, Some("ff"), Some("Ctrl+Shift+F")),
                )]
                .into(),
            },
        ),
        false,
    );

    assert_eq!(it.meta.alias.as_deref(), Some("ff"));
    assert_eq!(it.meta.shortcut.as_deref(), Some("Ctrl+Shift+F"));
}

#[test]
fn a_favourite_carries_its_position_and_a_fallback_its_flag() {
    // `meta.favoriteIdx = std::distance(cfg.favorites.begin(), it);` and
    // `meta.fallback = fallbackSet.contains(entrypointId);`
    let mut config = config();
    config.favorites = vec![
        entrypoint_id("apps", "a.desktop"),
        entrypoint_id("apps", "b.desktop"),
    ];
    config.fallbacks = vec![entrypoint_id("ext", "search")];

    let mut second = addressable("apps", "b.desktop");
    second.merge_config(&config, false);
    assert_eq!(second.meta.favorite_idx, Some(1), "its place in the list");
    assert!(!second.meta.fallback);

    let mut fallback = addressable("ext", "search");
    fallback.merge_config(&config, false);
    assert_eq!(fallback.meta.favorite_idx, None);
    assert!(fallback.meta.fallback);
}

#[test]
fn unfavouriting_clears_the_index_rather_than_leaving_it_behind() {
    // A declared divergence. The C++ only assigns `favoriteIdx` when the id is
    // in the list, and `m_metadata` outlives the merge -- so an item removed
    // from favourites keeps its old index until the launcher restarts, and
    // every search that drops favourites keeps dropping it.
    let mut it = addressable("apps", "a.desktop");
    let mut config = config();
    config.favorites = vec![entrypoint_id("apps", "a.desktop")];
    it.merge_config(&config, false);
    assert_eq!(it.meta.favorite_idx, Some(0));

    config.favorites.clear();
    it.merge_config(&config, false);
    assert_eq!(
        it.meta.favorite_idx, None,
        "the item is no longer a favourite and the metadata says so"
    );
}

#[test]
fn a_fallback_that_is_removed_stops_being_one() {
    // This one the C++ already recomputes every merge, because it assigns
    // unconditionally from the set.
    let mut it = addressable("ext", "search");
    let mut config = config();
    config.fallbacks = vec![entrypoint_id("ext", "search")];
    it.merge_config(&config, false);
    assert!(it.meta.fallback);

    config.fallbacks.clear();
    it.merge_config(&config, false);
    assert!(!it.meta.fallback);
}

#[test]
fn an_id_splits_on_its_first_colon() {
    // `EntrypointId::fromSerialized` uses `find(':')`, so an entrypoint may
    // contain colons and a provider may not.
    let mut it = addressable("ext", "search:recent");
    it.merge_config(
        &with_provider(
            config(),
            "ext",
            ProviderConfig {
                enabled: None,
                entrypoints: [(
                    "search:recent".to_owned(),
                    item_config(None, Some("sr"), None),
                )]
                .into(),
            },
        ),
        false,
    );

    assert_eq!(it.meta.provider_id, "ext");
    assert_eq!(it.meta.alias.as_deref(), Some("sr"));
}

#[test]
fn a_visit_is_counted_and_timed_and_can_be_forgotten() {
    // `++m_metadata[id].visitCount; ... lastVisitedAt = now;` and
    // `resetRanking`, which zeroes both.
    let mut it = addressable("apps", "a.desktop");
    assert_eq!(it.meta.visit_count, 0);
    assert_eq!(it.meta.last_visited_at, None);

    it.register_visit(NOW as u64);
    it.register_visit(NOW as u64 + 60);
    assert_eq!(it.meta.visit_count, 2);
    assert_eq!(it.meta.last_visited_at, Some(NOW as u64 + 60));

    // And it shows: frecency is non-zero, then zero again.
    assert!(it.frecency(NOW + 60) > 0.0);
    it.reset_ranking();
    assert_eq!(it.meta.visit_count, 0);
    assert_eq!(it.meta.last_visited_at, None);
    assert_eq!(it.frecency(NOW + 60), 0.0);
}

// --- writing the user's config -----------------------------------------
//
// Ported from `RootItemManager::{setAlias, setShortcut, setItemEnabled,
// setProviderEnabled}` and `config::Manager::{mergeEntrypointWithUser,
// mergeProviderWithUser}`.

mod config_writes {
    use compass_core::root_items::{
        ItemConfigPatch, ProviderConfigPatch, RootConfig, RootItem, RootItemMeta, set_alias,
        set_item_enabled, set_provider_enabled, set_shortcut,
    };

    fn item(provider: &str, entrypoint: &str) -> RootItem {
        RootItem {
            id: format!("{provider}:{entrypoint}"),
            meta: RootItemMeta {
                provider_id: provider.to_owned(),
                ..RootItemMeta::default()
            },
            ..RootItem::default()
        }
    }

    fn stored<'a>(
        config: &'a RootConfig,
        provider: &str,
        entrypoint: &str,
    ) -> Option<&'a compass_core::root_items::ItemConfig> {
        config.providers.get(provider)?.entrypoints.get(entrypoint)
    }

    #[test]
    fn setting_an_alias_updates_the_metadata_and_the_config() {
        // Doing only the first is the bug this pairing exists to prevent: the
        // change shows immediately and then vanishes on restart.
        let mut it = item("apps", "firefox");
        let mut config = RootConfig::default();
        set_alias(&mut it, &mut config, "ff");
        assert_eq!(it.meta.alias.as_deref(), Some("ff"));
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.alias.as_deref()),
            Some("ff")
        );
    }

    #[test]
    fn a_write_creates_the_provider_and_entrypoint_entries() {
        // The config holds only what the user changed, so most items have no
        // entry at all until the first write.
        let mut config = RootConfig::default();
        assert!(config.providers.is_empty());
        set_item_enabled(&mut config, "apps", "firefox", false);
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.enabled),
            Some(false)
        );
    }

    #[test]
    fn a_write_is_a_merge_and_not_a_replacement() {
        // Setting an alias must not clear a shortcut set earlier.
        let mut config = RootConfig::default();
        config.merge_entrypoint(
            "apps",
            "firefox",
            &ItemConfigPatch {
                shortcut: Some("ctrl+f".to_owned()),
                ..ItemConfigPatch::default()
            },
        );
        config.merge_entrypoint(
            "apps",
            "firefox",
            &ItemConfigPatch {
                alias: Some("ff".to_owned()),
                ..ItemConfigPatch::default()
            },
        );
        let entry = stored(&config, "apps", "firefox").expect("an entry");
        assert_eq!(entry.shortcut.as_deref(), Some("ctrl+f"));
        assert_eq!(entry.alias.as_deref(), Some("ff"));
    }

    #[test]
    fn an_absent_field_leaves_what_is_stored_alone() {
        let mut config = RootConfig::default();
        set_item_enabled(&mut config, "apps", "firefox", false);
        config.merge_entrypoint("apps", "firefox", &ItemConfigPatch::default());
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.enabled),
            Some(false)
        );
    }

    #[test]
    fn writing_one_entrypoint_leaves_its_siblings_alone() {
        let mut config = RootConfig::default();
        set_item_enabled(&mut config, "apps", "firefox", false);
        set_item_enabled(&mut config, "apps", "chromium", true);
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.enabled),
            Some(false)
        );
        assert_eq!(
            stored(&config, "apps", "chromium").and_then(|c| c.enabled),
            Some(true)
        );
    }

    #[test]
    fn a_shortcut_is_written_and_remembered() {
        let mut it = item("apps", "firefox");
        let mut config = RootConfig::default();
        set_shortcut(&mut it, &mut config, "ctrl+f");
        assert_eq!(it.meta.shortcut.as_deref(), Some("ctrl+f"));
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.shortcut.as_deref()),
            Some("ctrl+f")
        );
    }

    #[test]
    fn clearing_a_shortcut_clears_it_in_the_config_too() {
        // A declared divergence. The C++ resets the metadata but writes an
        // empty string into the config, and the next merge reads that empty
        // string back as a shortcut — so clearing one looks as though it
        // worked until the launcher restarts, and then the item has an empty
        // shortcut instead of none.
        let mut it = item("apps", "firefox");
        let mut config = RootConfig::default();
        set_shortcut(&mut it, &mut config, "ctrl+f");
        set_shortcut(&mut it, &mut config, "");
        assert_eq!(it.meta.shortcut, None);
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.shortcut.clone()),
            None,
            "an empty shortcut must not survive as Some(\"\")"
        );
    }

    #[test]
    fn enabling_an_item_whose_provider_is_off_does_not_make_it_appear() {
        // `set_item_enabled` writes only the config; the metadata follows on
        // the next merge. That is right rather than an oversight, and this is
        // why: the merge applies the provider's setting *after* the item's, so
        // a disabled provider still wins.
        let mut config = RootConfig::default();
        set_provider_enabled(&mut config, "apps", false);
        set_item_enabled(&mut config, "apps", "firefox", true);

        let mut it = item("apps", "firefox");
        it.merge_config(&config, false);
        assert!(
            !it.meta.enabled,
            "the provider's setting must outlast the item's"
        );
    }

    #[test]
    fn enabling_an_item_under_an_untouched_provider_does_make_it_appear() {
        let mut config = RootConfig::default();
        set_item_enabled(&mut config, "apps", "firefox", true);

        let mut it = item("apps", "firefox");
        it.merge_config(&config, true);
        assert!(it.meta.enabled);
    }

    #[test]
    fn a_written_alias_survives_the_round_trip_through_a_merge() {
        // The pairing is only worth anything if what was written comes back.
        let mut it = item("apps", "firefox");
        let mut config = RootConfig::default();
        set_alias(&mut it, &mut config, "ff");

        let mut reloaded = item("apps", "firefox");
        reloaded.merge_config(&config, false);
        assert_eq!(reloaded.meta.alias.as_deref(), Some("ff"));
    }

    #[test]
    fn a_cleared_shortcut_stays_cleared_through_a_merge() {
        // The divergence again, from the other side: with the C++'s empty
        // string in the config this comes back as `Some("")`.
        let mut it = item("apps", "firefox");
        let mut config = RootConfig::default();
        set_shortcut(&mut it, &mut config, "ctrl+f");
        set_shortcut(&mut it, &mut config, "");

        let mut reloaded = item("apps", "firefox");
        reloaded.merge_config(&config, false);
        assert_eq!(reloaded.meta.shortcut, None);
    }

    #[test]
    fn turning_a_provider_off_is_stored_on_the_provider() {
        let mut config = RootConfig::default();
        set_provider_enabled(&mut config, "apps", false);
        assert_eq!(
            config.providers.get("apps").and_then(|p| p.enabled),
            Some(false)
        );
    }

    #[test]
    fn a_provider_write_leaves_its_entrypoints_alone() {
        let mut config = RootConfig::default();
        set_item_enabled(&mut config, "apps", "firefox", false);
        set_provider_enabled(&mut config, "apps", true);
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.enabled),
            Some(false)
        );
    }

    #[test]
    fn a_provider_patch_with_nothing_in_it_changes_nothing() {
        let mut config = RootConfig::default();
        set_provider_enabled(&mut config, "apps", true);
        config.merge_provider("apps", &ProviderConfigPatch::default());
        assert_eq!(
            config.providers.get("apps").and_then(|p| p.enabled),
            Some(true)
        );
    }

    #[test]
    fn a_stored_provider_setting_survives_a_write_to_a_different_provider() {
        let mut config = RootConfig::default();
        set_provider_enabled(&mut config, "apps", false);
        set_provider_enabled(&mut config, "extensions", true);
        assert_eq!(
            config.providers.get("apps").and_then(|p| p.enabled),
            Some(false)
        );
        assert_eq!(
            config.providers.get("extensions").and_then(|p| p.enabled),
            Some(true)
        );
    }

    #[test]
    fn an_item_whose_id_has_no_separator_is_written_under_its_provider() {
        // A bare id is not malformed: `entrypoint_id_of` joins it to the
        // item's provider, so the write lands where the merge will look for
        // it.
        let mut it = RootItem {
            id: "firefox".to_owned(),
            ..item("apps", "firefox")
        };
        let mut config = RootConfig::default();
        set_alias(&mut it, &mut config, "ff");
        assert_eq!(
            stored(&config, "apps", "firefox").and_then(|c| c.alias.as_deref()),
            Some("ff")
        );
    }

    #[test]
    fn an_item_with_no_provider_writes_under_an_empty_provider_key() {
        // Faithful to the C++, whose `EntrypointId` does the same. It is not
        // useful — nothing reads that key — but inventing a guard the C++ does
        // not have would make the two configs diverge in a way nobody asked
        // for. Pinned so the behaviour is known rather than discovered.
        let mut it = RootItem {
            id: "orphan".to_owned(),
            meta: RootItemMeta::default(),
            ..item("apps", "firefox")
        };
        let mut config = RootConfig::default();
        set_alias(&mut it, &mut config, "x");
        assert_eq!(
            stored(&config, "", "orphan").and_then(|c| c.alias.as_deref()),
            Some("x")
        );
    }
}

// --- an application as a root item --------------------------------------
//
// Ported from `AppRootItem` / `AppRootProvider`
// (`src/server/src/root-search/apps/app-root-provider.cpp`).

mod from_applications {
    use compass_core::root_items::{APPS_PROVIDER_ID, app_entrypoint_id, app_root_item};

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn the_provider_is_spelled_applications() {
        // It is half of every application's entrypoint id and so is written
        // into the config file: the spelling is a stored format, not a label.
        assert_eq!(APPS_PROVIDER_ID, "applications");
    }

    #[test]
    fn the_desktop_suffix_comes_off_the_entrypoint() {
        assert_eq!(app_entrypoint_id("konsole.desktop"), "konsole");
    }

    #[test]
    fn a_nested_id_keeps_its_dots_and_loses_only_the_suffix() {
        assert_eq!(app_entrypoint_id("kde4.konsole.desktop"), "kde4.konsole");
    }

    #[test]
    fn every_occurrence_goes_not_just_the_last() {
        // `QString::remove` takes out all of them. For an ordinary id that is
        // the same thing; for this one it is not, and the C++'s answer is the
        // one that got stored.
        assert_eq!(app_entrypoint_id("my.desktop.desktop"), "my");
        assert_eq!(app_entrypoint_id(".desktop"), "");
    }

    #[test]
    fn an_id_without_the_suffix_is_left_alone() {
        assert_eq!(app_entrypoint_id("konsole"), "konsole");
    }

    #[test]
    fn the_item_is_addressed_by_provider_and_entrypoint() {
        let item = app_root_item("konsole.desktop", "Konsole", &[], None);
        assert_eq!(item.id, "applications:konsole");
    }

    #[test]
    fn the_title_is_the_display_name() {
        let item = app_root_item("konsole.desktop", "Konsole", &[], None);
        assert_eq!(item.title, "Konsole");
    }

    #[test]
    fn the_subtitle_is_deliberately_empty() {
        // An application's comment is its description in the settings, not a
        // second line in the launcher. Filling it would give every row a
        // paragraph.
        let item = app_root_item("konsole.desktop", "Konsole", &[], None);
        assert_eq!(item.subtitle, "");
    }

    #[test]
    fn the_desktop_files_keywords_are_searchable() {
        let item = app_root_item(
            "konsole.desktop",
            "Konsole",
            &words(&["shell", "prompt"]),
            None,
        );
        assert_eq!(item.keywords, ["shell", "prompt"]);
    }

    #[test]
    fn the_unlocalized_name_is_a_separate_title() {
        // So someone who knows an application by its English name finds it on
        // a localised desktop where the title is something else.
        let item = app_root_item(
            "files.desktop",
            "Dateien",
            &words(&["folder"]),
            Some("Files"),
        );
        assert_eq!(item.keywords, ["folder"]);
        assert_eq!(item.unlocalized_title.as_deref(), Some("Files"));
    }

    #[test]
    fn an_application_with_no_unlocalized_name_gains_no_extra_term() {
        let item = app_root_item("konsole.desktop", "Konsole", &words(&["shell"]), None);
        assert_eq!(item.keywords, ["shell"]);
        assert_eq!(item.unlocalized_title, None);
    }

    #[test]
    fn an_identical_unlocalized_name_is_not_duplicated() {
        let item = app_root_item("files.desktop", "Files", &[], Some("Files"));
        assert_eq!(item.unlocalized_title, None);
    }

    #[test]
    fn the_unlocalized_name_does_not_change_keywords() {
        let item = app_root_item("a.desktop", "A", &words(&["one", "two"]), Some("English"));
        assert_eq!(item.keywords, ["one", "two"]);
        assert_eq!(item.unlocalized_title.as_deref(), Some("English"));
    }

    #[test]
    fn a_converted_application_starts_enabled() {
        // The root item manager's merge is what turns it off; an application
        // is not disabled by being converted.
        let item = app_root_item("konsole.desktop", "Konsole", &[], None);
        assert!(item.meta.enabled);
        assert_eq!(item.meta.provider_id, "applications");
    }

    #[test]
    fn a_converted_application_carries_no_user_state() {
        // Alias, favourite position and visit count all come from the config
        // and the visit log, not from the desktop file.
        let item = app_root_item("konsole.desktop", "Konsole", &[], None);
        assert_eq!(item.meta.alias, None);
        assert_eq!(item.meta.favorite_idx, None);
        assert_eq!(item.meta.visit_count, 0);
        assert_eq!(item.meta.last_visited_at, None);
        assert!(!item.meta.fallback);
    }
}

#[test]
fn a_one_slip_query_finds_an_item_the_matcher_cannot_reach() {
    // #204: "blneder" is not a subsequence of "Blender", so the matcher alone
    // returns nothing for it.
    let items = vec![item("blender", "Blender"), item("files", "Files")];
    assert_eq!(
        ids(&items, "blneder", &SearchOptions::default()),
        ["blender"]
    );
    assert!(ids(&items, "blnedxr", &SearchOptions::default()).is_empty());
}

#[test]
fn a_typo_hit_never_outranks_a_real_match() {
    // However often the slipped-on item has been opened, an item the matcher
    // really reached comes first.
    let mut slipped = item("slipped", "Blender");
    slipped.meta.visit_count = 1000;
    slipped.meta.last_visited_at = Some(NOW as u64);
    let mut real = item("real", "Modeller");
    real.keywords = vec!["blneder".to_owned()];
    let items = vec![slipped, real];
    let scored = search(&items, "blneder", &SearchOptions::default(), NOW);
    let order: Vec<_> = scored.iter().map(|s| s.item.id.as_str()).collect();
    assert_eq!(order, ["real", "slipped"], "{scored:?}");
    assert!(scored[0].score > scored[1].score, "{scored:?}");
}

#[test]
fn the_typo_fallback_respects_the_same_filters_as_the_matcher() {
    let mut disabled = item("blender", "Blender");
    disabled.meta.enabled = false;
    assert!(ids(&[disabled.clone()], "blneder", &SearchOptions::default()).is_empty());
    let opts = SearchOptions {
        include_disabled: true,
        ..SearchOptions::default()
    };
    assert_eq!(ids(&[disabled], "blneder", &opts), ["blender"]);
}

#[test]
fn a_short_query_gets_no_typo_fallback() {
    // Four characters is one edit away from far too much of a real catalogue.
    let items = vec![item("gimp", "GIMP")];
    assert!(ids(&items, "gmip", &SearchOptions::default()).is_empty());
}

#[test]
fn favouriting_inserts_first_and_moving_swaps_within_the_list_only() {
    use compass_core::root_items::{RootConfig, RootEdit, apply_edit};
    let mut config = RootConfig {
        favorites: vec!["a:1".into(), "a:2".into()],
        ..RootConfig::default()
    };
    // `setItemAsFavorite` inserts at the beginning.
    assert!(apply_edit(&mut config, "a:3", &RootEdit::Favorite(true)));
    assert_eq!(config.favorites, ["a:3", "a:1", "a:2"]);
    assert!(!apply_edit(&mut config, "a:3", &RootEdit::Favorite(true)));
    // The first cannot move up, nor the last down.
    assert!(!apply_edit(
        &mut config,
        "a:3",
        &RootEdit::MoveFavorite { down: false }
    ));
    assert!(!apply_edit(
        &mut config,
        "a:2",
        &RootEdit::MoveFavorite { down: true }
    ));
    assert!(apply_edit(
        &mut config,
        "a:3",
        &RootEdit::MoveFavorite { down: true }
    ));
    assert_eq!(config.favorites, ["a:1", "a:3", "a:2"]);
    assert!(apply_edit(
        &mut config,
        "a:2",
        &RootEdit::MoveFavorite { down: false }
    ));
    assert_eq!(config.favorites, ["a:1", "a:2", "a:3"]);
    assert!(!apply_edit(
        &mut config,
        "b:9",
        &RootEdit::MoveFavorite { down: true }
    ));
    assert!(apply_edit(&mut config, "a:2", &RootEdit::Favorite(false)));
    assert!(!apply_edit(&mut config, "a:2", &RootEdit::Favorite(false)));
    assert_eq!(config.favorites, ["a:1", "a:3"]);
}

#[test]
fn an_alias_and_the_switch_are_written_under_the_items_provider_and_merged() {
    use compass_core::root_items::{RootConfig, RootEdit, apply_edit, deeplink};
    let mut config = RootConfig::default();
    assert!(apply_edit(
        &mut config,
        "applications:org.gnome.Nautilus",
        &RootEdit::Alias("files".into())
    ));
    assert!(apply_edit(&mut config, "scripts:hello", &RootEdit::Disable));
    assert_eq!(
        config.providers["applications"].entrypoints["org.gnome.Nautilus"]
            .alias
            .as_deref(),
        Some("files")
    );
    assert_eq!(
        config.providers["scripts"].entrypoints["hello"].enabled,
        Some(false)
    );

    let mut nautilus = item("applications:org.gnome.Nautilus", "Files");
    nautilus.merge_config(&config, false);
    assert_eq!(nautilus.meta.alias.as_deref(), Some("files"));
    let mut hello = item("scripts:hello", "Hello");
    hello.merge_config(&config, false);
    assert!(!hello.meta.enabled);

    assert!(!apply_edit(&mut config, "no-colon", &RootEdit::Disable));
    assert_eq!(
        deeplink("applications:org.gnome.Nautilus").as_deref(),
        Some("vicinae://launch/applications/org.gnome.Nautilus")
    );
}

// --- the `launch` deeplink ------------------------------------------------

use compass_core::root_items::{INVALID_LAUNCH_LINK, LaunchTarget, parse_launch_link};

#[test]
fn a_launch_link_names_a_provider_or_an_item_with_its_text() {
    let providers = ["applications", "@zoë/notes"];
    let is_provider = |id: &str| providers.contains(&id);

    let link =
        parse_launch_link("vicinae://launch/applications/?fallbackText=fire+fox&toggle=true")
            .expect("a launch link");
    assert_eq!(link.path, "applications");
    assert_eq!(link.fallback_text.as_deref(), Some("fire fox"));
    assert!(link.toggle);
    assert_eq!(
        link.target(is_provider),
        Ok(LaunchTarget::Provider("applications".into()))
    );

    // A provider id with a slash in it is still the provider; one more
    // segment is its item, split at the last slash.
    let link = parse_launch_link("vicinae://launch/@zo%C3%AB/notes").unwrap();
    assert_eq!(
        link.target(is_provider),
        Ok(LaunchTarget::Provider("@zoë/notes".into()))
    );
    let link = parse_launch_link("vicinae://launch/@zo%C3%AB/notes/list?fallbackText=").unwrap();
    assert_eq!(link.fallback_text, None, "an empty text is none");
    assert!(!link.toggle);
    assert_eq!(
        link.target(is_provider),
        Ok(LaunchTarget::Entrypoint("@zoë/notes:list".into()))
    );

    let link = parse_launch_link("vicinae://launch/nothing").unwrap();
    assert_eq!(
        link.target(is_provider),
        Err(INVALID_LAUNCH_LINK.to_owned())
    );

    assert_eq!(parse_launch_link("vicinae://extensions/a/b"), None);
    assert_eq!(parse_launch_link("https://launch/applications"), None);
    assert_eq!(parse_launch_link("not a url"), None);
}
