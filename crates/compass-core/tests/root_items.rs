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
