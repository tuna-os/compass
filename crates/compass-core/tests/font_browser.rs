//! The font browser, read against `font-grid-model.cpp` and
//! `font-browser-view-host.hpp` (`src/server/src/builtins/font/`).

use compass_core::font_browser::{
    ALL_OPTION, COLUMNS, COPY_FAMILY_TITLE, COPY_KEYBIND, FontFamily, MISSING_GLYPH, Mode,
    PREVIEW_TITLE, Preview, SET_APP_FONT_TITLE, STORAGE_KEY, action_panel, build,
    category_for_index, filter_options, index_for_saved, navigation_title, preview,
};

fn family(name: &str, categories: &[&str]) -> FontFamily {
    FontFamily {
        name: name.to_owned(),
        family: name.to_owned(),
        glyph: "Aa".to_owned(),
        color: false,
        categories: categories.iter().map(|c| (*c).to_owned()).collect(),
    }
}

fn ordered() -> Vec<String> {
    ["Latin", "Cyrillic", "Monospace", "Emoji", "Thai"]
        .iter()
        .map(|c| (*c).to_owned())
        .collect()
}

fn titles(mode: &Mode) -> (String, Vec<String>) {
    match mode {
        Mode::Root { title, families } | Mode::Search { title, families } => (
            title.clone(),
            families.iter().map(|f| f.name.clone()).collect(),
        ),
    }
}

#[test]
fn only_categories_somebody_has_a_font_for_are_offered() {
    // `buildFilterOptions` ORs every family's categories together and keeps
    // the ones present, in the service's own order.
    let families = [
        family("Inter", &["Latin"]),
        family("Noto Color Emoji", &["Emoji"]),
    ];

    assert_eq!(
        filter_options(&ordered(), &families),
        ["All", "Latin", "Emoji"],
        "Cyrillic, Monospace and Thai have no fonts and are not listed"
    );
    assert_eq!(ALL_OPTION, "All");
}

#[test]
fn index_zero_is_all_and_the_rest_are_offset_by_one() {
    // `index <= 0 ? nullopt : index - 1`, then a bounds check.
    let options: Vec<String> = ["All", "Latin", "Emoji"]
        .iter()
        .map(|o| (*o).to_owned())
        .collect();

    assert_eq!(category_for_index(&options, 0), None);
    assert_eq!(
        category_for_index(&options, -1),
        None,
        "negative is All too"
    );
    assert_eq!(category_for_index(&options, 1).as_deref(), Some("Latin"));
    assert_eq!(category_for_index(&options, 2).as_deref(), Some("Emoji"));
    assert_eq!(
        category_for_index(&options, 9),
        None,
        "past the end falls back to All rather than failing"
    );
}

#[test]
fn a_remembered_category_is_restored_and_a_remembered_all_is_not() {
    // `if (index > 0) setCategoryFilter(index)` -- "All" is index 0, so
    // restoring it would be a no-op anyway, and the guard makes that explicit.
    let options: Vec<String> = ["All", "Latin", "Emoji"]
        .iter()
        .map(|o| (*o).to_owned())
        .collect();

    assert_eq!(index_for_saved(&options, Some("Emoji")), Some(2));
    assert_eq!(index_for_saved(&options, Some("All")), None);
    assert_eq!(
        index_for_saved(&options, Some("Cyrillic")),
        None,
        "a category whose fonts were uninstalled is ignored"
    );
    assert_eq!(index_for_saved(&options, None), None);
    assert_eq!(STORAGE_KEY, "fontCategory");
}

#[test]
fn the_root_heading_counts_what_it_shows() {
    let families = [
        family("Inter", &["Latin"]),
        family("Fira Code", &["Latin", "Monospace"]),
        family("Noto Color Emoji", &["Emoji"]),
    ];

    let (title, names) = titles(&build(&families, "", None));
    assert_eq!(title, "All Fonts (3)");
    assert_eq!(names, ["Inter", "Fira Code", "Noto Color Emoji"]);

    let (title, names) = titles(&build(&families, "", Some("Latin")));
    assert_eq!(title, "Latin (2)", "the category's name, then its count");
    assert_eq!(names, ["Inter", "Fira Code"]);
}

#[test]
fn the_root_list_keeps_the_font_services_order() {
    // There is no sort in `rebuildRoot`: whatever order the font service
    // returned is what the grid shows.
    let families = [
        family("Zapfino", &["Latin"]),
        family("Arial", &["Latin"]),
        family("Menlo", &["Latin"]),
    ];

    let (_, names) = titles(&build(&families, "", None));
    assert_eq!(names, ["Zapfino", "Arial", "Menlo"]);
}

#[test]
fn searching_scores_the_name_alone() {
    // `scoreWeighted({{f.name, 1.0}}, query)` -- one field. The render family
    // and the categories are not searchable.
    let mut odd = family("Display Name", &["Latin"]);
    odd.family = "InternalFamilyName".to_owned();

    let families = [odd];
    let (_, names) = titles(&build(&families, "display", None));
    assert_eq!(names, ["Display Name"]);

    let (_, names) = titles(&build(&families, "internalfamily", None));
    assert!(names.is_empty(), "the render family is not searched");

    let (_, names) = titles(&build(&families, "latin", None));
    assert!(names.is_empty(), "nor are the categories");
}

#[test]
fn the_search_heading_counts_the_matches() {
    let families = [
        family("Fira Code", &["Monospace"]),
        family("Fira Sans", &["Latin"]),
        family("Inter", &["Latin"]),
    ];

    let (title, names) = titles(&build(&families, "fira", None));
    assert_eq!(title, "Results (2)");
    assert_eq!(names.len(), 2);
}

#[test]
fn a_category_filter_narrows_the_search_as_well() {
    // The scorer returns an empty match for a family outside the category, so
    // a filtered search can come back empty where "All" would find something.
    let families = [
        family("Fira Code", &["Monospace"]),
        family("Fira Sans", &["Latin"]),
    ];

    let (title, names) = titles(&build(&families, "fira", Some("Monospace")));
    assert_eq!(names, ["Fira Code"]);
    assert_eq!(title, "Results (1)");

    let (_, names) = titles(&build(&families, "fira", Some("Emoji")));
    assert!(names.is_empty());
}

#[test]
fn a_family_with_no_glyph_draws_a_question_mark() {
    // `ImageURL::fontPreview(QString(), "?")` with the foreground fill -- and
    // note the family is *empty*, so the placeholder is drawn in the UI font
    // rather than in the font it stands for.
    let mut glyphless = family("Weird Font", &["Latin"]);
    glyphless.glyph = String::new();

    assert_eq!(
        preview(&glyphless),
        Preview {
            family: String::new(),
            glyph: MISSING_GLYPH.to_owned(),
            fill_foreground: true,
        }
    );
    assert_eq!(MISSING_GLYPH, "?");
}

#[test]
fn a_colour_font_keeps_its_own_colours() {
    // `if (!family->color) url.setFill(SemanticColor::Foreground)` -- tinting
    // an emoji font to the foreground colour would erase the point of it.
    let mut emoji = family("Noto Color Emoji", &["Emoji"]);
    emoji.glyph = "😀".to_owned();
    emoji.color = true;

    assert_eq!(
        preview(&emoji),
        Preview {
            family: "Noto Color Emoji".to_owned(),
            glyph: "😀".to_owned(),
            fill_foreground: false,
        }
    );
    assert!(preview(&family("Inter", &["Latin"])).fill_foreground);
}

#[test]
fn preview_is_the_primary_action_not_copy_or_apply() {
    // `preview->setPrimary(true)` -- Enter opens the specimen. Applying the
    // font to the launcher is deliberately not what Enter does.
    let actions = action_panel();

    assert_eq!(
        actions
            .iter()
            .map(|action| action.title)
            .collect::<Vec<_>>(),
        [PREVIEW_TITLE, COPY_FAMILY_TITLE, SET_APP_FONT_TITLE]
    );
    assert!(actions[0].primary);
    assert!(actions[1..].iter().all(|action| !action.primary));
    assert_eq!(actions[1].keybind, Some(COPY_KEYBIND));
    assert_eq!(COPY_KEYBIND, "action.copy");
    assert_eq!(PREVIEW_TITLE, "Preview font");
    assert_eq!(SET_APP_FONT_TITLE, "Set as Compass font");
}

#[test]
fn the_navigation_title_is_empty_with_nothing_selected() {
    // `name.isEmpty() ? name : "%1 - %2"` -- the command's name alone is not
    // shown, which is what makes the title disappear on an empty search.
    assert_eq!(
        navigation_title("Browse Fonts", Some("Inter")),
        "Browse Fonts - Inter"
    );
    assert_eq!(navigation_title("Browse Fonts", None), "");
    assert_eq!(navigation_title("Browse Fonts", Some("")), "");
}

#[test]
fn the_grid_is_six_columns() {
    assert_eq!(COLUMNS, 6);
}
