//! The font browser: which families are shown, under what heading, and what
//! each offers.
//!
//! A port of `font-grid-model.cpp` and `font-browser-view-host.hpp`
//! (`src/server/src/builtins/font/`), without the grid widget.
//!
//! The category *table* — thirty-three writing systems, their display names and
//! their specimen glyphs — belongs to `src/services/font-service`, which is a
//! row of its own. What is here is everything the browser does with it: the
//! filter list, the index arithmetic that turns a dropdown position into a
//! category, what is persisted between visits, and the two headings.

/// The grid's column count.
pub const COLUMNS: usize = 6;
/// The grid's aspect ratio.
pub const ASPECT_RATIO: f32 = 1.0;

/// The first entry of the category dropdown, meaning "do not filter".
pub const ALL_OPTION: &str = "All";
/// The storage key the chosen category is remembered under.
pub const STORAGE_KEY: &str = "fontCategory";

/// The action that opens the specimen view.
pub const PREVIEW_TITLE: &str = "Preview font";
/// The action that copies the family name.
pub const COPY_FAMILY_TITLE: &str = "Copy font family";
/// The action that makes this the launcher's own font.
pub const SET_APP_FONT_TITLE: &str = "Set as vicinae font";
/// `Keybind::CopyAction`'s config id, which "Copy font family" uses.
pub const COPY_KEYBIND: &str = "action.copy";

/// The glyph shown for a family with no specimen glyph of its own.
pub const MISSING_GLYPH: &str = "?";

/// A font family, as the browser sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontFamily {
    /// What to show and what to search.
    pub name: String,
    /// The family to render with, which is not always the display name.
    pub family: String,
    /// The specimen glyph; empty means [`MISSING_GLYPH`] is drawn instead.
    pub glyph: String,
    /// Whether the glyph has colour of its own — an emoji font does.
    pub color: bool,
    /// The categories it belongs to, by id.
    pub categories: Vec<String>,
}

impl FontFamily {
    /// Whether this family is in `category`.
    #[must_use]
    pub fn has(&self, category: &str) -> bool {
        self.categories.iter().any(|owned| owned == category)
    }
}

/// The dropdown's entries: "All", then every category that some installed
/// family actually belongs to, in the font service's own order.
///
/// A category nobody has a font for is not offered, which is why the list is
/// short on a minimal system and long on a full one.
#[must_use]
pub fn filter_options(ordered_categories: &[String], families: &[FontFamily]) -> Vec<String> {
    let mut options = vec![ALL_OPTION.to_owned()];
    options.extend(
        ordered_categories
            .iter()
            .filter(|category| families.iter().any(|family| family.has(category)))
            .cloned(),
    );
    options
}

/// The category a dropdown index selects: index 0 is "All".
///
/// `index <= 0 ? nullopt : index - 1` in the view host, then a bounds check in
/// the model — so an index past the end is "All" as well rather than a crash.
#[must_use]
pub fn category_for_index(options: &[String], index: i32) -> Option<String> {
    if index <= 0 {
        return None;
    }
    options.get(usize::try_from(index).ok()?).cloned()
}

/// The index to restore for a remembered category name.
///
/// `restoreCategoryFilter` looks the saved name up and applies it only when the
/// index is **greater than zero**: a saved "All" is a no-op, and a category
/// whose fonts have since been uninstalled is not found and is ignored.
#[must_use]
pub fn index_for_saved(options: &[String], saved: Option<&str>) -> Option<usize> {
    let saved = saved?;
    let index = options.iter().position(|option| option == saved)?;
    (index > 0).then_some(index)
}

/// Which list the browser is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Everything (subject to the category filter), under one heading.
    Root {
        /// The heading.
        title: String,
        /// The families, in the font service's order.
        families: Vec<FontFamily>,
    },
    /// Search results, best first.
    Search {
        /// The heading.
        title: String,
        /// The matches.
        families: Vec<FontFamily>,
    },
}

/// Builds the list for `query` and the current category filter.
///
/// The category filter applies in both modes, but by different means: the root
/// list filters before building, while the search scorer returns an empty match
/// for a family outside the category — which is the same outcome by a different
/// route, and is why a filtered search can come back empty while the same query
/// with "All" finds something.
#[must_use]
pub fn build(families: &[FontFamily], query: &str, category: Option<&str>) -> Mode {
    if query.is_empty() {
        let members: Vec<FontFamily> = families
            .iter()
            .filter(|family| category.is_none_or(|category| family.has(category)))
            .cloned()
            .collect();

        let title = match category {
            Some(category) => format!("{category} ({})", members.len()),
            None => format!("All Fonts ({})", members.len()),
        };

        return Mode::Root {
            title,
            families: members,
        };
    }

    let parsed = compass_search::Query::new(query);
    let mut scored: Vec<(u32, FontFamily)> = families
        .iter()
        .filter(|family| category.is_none_or(|category| family.has(category)))
        .filter_map(|family| {
            // Only the name is scored, at 1.0 -- not the family, not the
            // categories.
            let matched = compass_search::score_weighted(
                &[compass_search::WeightedField::new(&family.name, 1.0)],
                &parsed,
            );
            matched.accepted().then(|| (matched.score, family.clone()))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    Mode::Search {
        title: format!("Results ({})", scored.len()),
        families: scored.into_iter().map(|(_, family)| family).collect(),
    }
}

/// What a row draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The family to render with; empty for the missing-glyph placeholder.
    pub family: String,
    /// The glyph to draw.
    pub glyph: String,
    /// Whether to tint it with the foreground colour. A colour font is left
    /// alone so its own colours survive.
    pub fill_foreground: bool,
}

/// The preview for one family.
#[must_use]
pub fn preview(family: &FontFamily) -> Preview {
    if family.glyph.is_empty() {
        return Preview {
            family: String::new(),
            glyph: MISSING_GLYPH.to_owned(),
            fill_foreground: true,
        };
    }

    Preview {
        family: family.family.clone(),
        glyph: family.glyph.clone(),
        fill_foreground: !family.color,
    }
}

/// An action on a font row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontAction {
    /// Its title.
    pub title: &'static str,
    /// Its keybind's config id, when it has one.
    pub keybind: Option<&'static str>,
    /// Whether it is the primary action — the one Enter runs.
    pub primary: bool,
}

/// The action panel for a font row, in order.
#[must_use]
pub fn action_panel() -> Vec<FontAction> {
    vec![
        FontAction {
            title: PREVIEW_TITLE,
            keybind: None,
            // `preview->setPrimary(true)` -- Enter opens the specimen rather
            // than copying or applying anything.
            primary: true,
        },
        FontAction {
            title: COPY_FAMILY_TITLE,
            keybind: Some(COPY_KEYBIND),
            primary: false,
        },
        FontAction {
            title: SET_APP_FONT_TITLE,
            keybind: None,
            primary: false,
        },
    ]
}

/// The navigation title while a family is selected.
///
/// `"%1 - %2"` of the command's name and the family's, and **empty** when
/// nothing is selected — not the command's name on its own.
#[must_use]
pub fn navigation_title(command: &str, selected: Option<&str>) -> String {
    match selected.filter(|name| !name.is_empty()) {
        Some(name) => format!("{command} - {name}"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn families() -> Vec<FontFamily> {
        vec![
            FontFamily {
                name: "Cantarell".to_owned(),
                family: "Cantarell".to_owned(),
                glyph: "A".to_owned(),
                color: false,
                categories: vec!["latin".to_owned()],
            },
            FontFamily {
                name: "Noto Color Emoji".to_owned(),
                family: "Noto Color Emoji".to_owned(),
                glyph: "😀".to_owned(),
                color: true,
                categories: vec!["emoji".to_owned(), "latin".to_owned()],
            },
            FontFamily {
                name: "Missing".to_owned(),
                family: "Missing".to_owned(),
                glyph: String::new(),
                color: false,
                categories: vec![],
            },
        ]
    }

    #[test]
    fn filter_options_starts_with_all_and_only_offered_categories() {
        let ordered = vec![
            "latin".to_owned(),
            "emoji".to_owned(),
            "cyrillic".to_owned(),
        ];
        let opts = filter_options(&ordered, &families());
        assert_eq!(opts[0], ALL_OPTION);
        assert!(opts.contains(&"latin".to_owned()));
        assert!(opts.contains(&"emoji".to_owned()));
        assert!(!opts.contains(&"cyrillic".to_owned()));
    }

    #[test]
    fn category_for_index_zero_and_out_of_bounds_is_all() {
        let opts = vec!["All".to_owned(), "latin".to_owned()];
        assert_eq!(category_for_index(&opts, 0), None);
        assert_eq!(category_for_index(&opts, -1), None);
        assert_eq!(category_for_index(&opts, 1), Some("latin".to_owned()));
        assert_eq!(category_for_index(&opts, 99), None);
    }

    #[test]
    fn index_for_saved_only_when_found_and_gt_zero() {
        let opts = vec!["All".to_owned(), "latin".to_owned(), "emoji".to_owned()];
        assert_eq!(index_for_saved(&opts, Some("latin")), Some(1));
        assert_eq!(index_for_saved(&opts, Some("All")), None);
        assert_eq!(index_for_saved(&opts, Some("missing")), None);
        assert_eq!(index_for_saved(&opts, None), None);
    }

    #[test]
    fn build_root_and_search_with_category_filter() {
        let fam = families();
        let root_all = build(&fam, "", None);
        match root_all {
            Mode::Root { title, families } => {
                assert!(title.contains("All Fonts"));
                assert_eq!(families.len(), 3);
            }
            _ => panic!("expected root"),
        }
        let root_emoji = build(&fam, "", Some("emoji"));
        match root_emoji {
            Mode::Root { families, .. } => assert_eq!(families.len(), 1),
            _ => panic!("expected root"),
        }
        let search = build(&fam, "Cant", None);
        match search {
            Mode::Search { families, .. } => {
                assert_eq!(families.len(), 1);
                assert_eq!(families[0].name, "Cantarell");
            }
            _ => panic!("expected search"),
        }
        let search_filtered = build(&fam, "Cant", Some("emoji"));
        match search_filtered {
            Mode::Search { families, .. } => assert!(families.is_empty()),
            _ => panic!("expected search"),
        }
    }

    #[test]
    fn preview_handles_missing_glyph_and_color() {
        let miss = FontFamily {
            glyph: String::new(),
            ..families()[0].clone()
        };
        let p = preview(&miss);
        assert_eq!(p.glyph, MISSING_GLYPH);
        assert!(p.fill_foreground);
        let color = preview(&families()[1]);
        assert!(!color.fill_foreground);
        let plain = preview(&families()[0]);
        assert!(plain.fill_foreground);
    }

    #[test]
    fn action_panel_and_navigation_title() {
        let panel = action_panel();
        assert_eq!(panel.len(), 3);
        assert_eq!(panel[0].title, PREVIEW_TITLE);
        assert!(panel[0].primary);
        assert_eq!(panel[1].keybind, Some(COPY_KEYBIND));
        assert_eq!(
            navigation_title("Fonts", Some("Cantarell")),
            "Fonts - Cantarell"
        );
        assert_eq!(navigation_title("Fonts", Some("")), "");
        assert_eq!(navigation_title("Fonts", None), "");
    }
}
