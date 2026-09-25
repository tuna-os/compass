//! Two of the Vicinae extension's inspection views: Show Installed
//! Extensions (`InstalledExtensionsViewHost`) and Search Builtin Icons
//! (`BuiltinIconsViewHost`). Each is a filtered list; the launcher draws it
//! and runs its panel ([`crate::app`]).

use compass_core::manifest::{ExtensionManifest, Provenance};
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

/// Show Installed Extensions' placeholder.
pub const EXTENSIONS_PLACEHOLDER: &str = "Search extensions...";
/// Search Builtin Icons' placeholder.
pub const ICONS_PLACEHOLDER: &str = "Search icons...";

/// The positions in `0..len` whose fields match `query`, best first; every
/// position, in order, for an empty query (`FuzzySection`'s filter).
fn filter<'a>(
    len: usize,
    query: &str,
    fields: impl Fn(usize) -> Vec<(&'a str, f32)>,
) -> Vec<usize> {
    let query = Query::new(query);
    if query.is_empty() {
        return (0..len).collect();
    }
    let mut scored: Vec<(u32, usize)> = (0..len)
        .filter_map(|index| {
            let weighted: Vec<WeightedField<'_>> = fields(index)
                .into_iter()
                .map(|(text, weight)| WeightedField::new(text, weight))
                .collect();
            let found = score_weighted(&weighted, &query);
            (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, index)| index).collect()
}

/// The badge beside an installed extension (`displayAccessories`): where it
/// came from, with its colour's name.
#[must_use]
pub const fn provenance_badge(provenance: Provenance) -> &'static str {
    match provenance {
        Provenance::Raycast => "Raycast",
        Provenance::Vicinae => "Vicinae",
        Provenance::Local => "Local",
    }
}

/// Show Installed Extensions' state.
#[derive(Debug, Clone, Default)]
pub struct ExtensionsPage {
    /// The filter text.
    pub query: String,
    /// Every installed extension, as its manifest says.
    pub all: Vec<ExtensionManifest>,
    /// Positions in `all` that match, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the last action said, or why it did not happen.
    pub notice: Option<String>,
}

impl ExtensionsPage {
    /// The view over `all`.
    #[must_use]
    pub fn new(all: Vec<ExtensionManifest>) -> Self {
        let mut page = Self {
            all,
            ..Self::default()
        };
        page.refilter();
        page
    }

    /// Recomputes `shown` for the query, over the title and description.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let all = &self.all;
        self.shown = filter(all.len(), &self.query, |index| {
            vec![
                (all[index].title.as_str(), 1.0),
                (all[index].description.as_str(), 0.5),
            ]
        });
    }

    /// The selected extension.
    #[must_use]
    pub fn selected_extension(&self) -> Option<&ExtensionManifest> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// Forgets an uninstalled extension.
    pub fn remove(&mut self, id: &str) {
        self.all.retain(|extension| extension.id != id);
        let selected = self.selected;
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }
}

/// Search Builtin Icons' state.
#[derive(Debug, Clone, Default)]
pub struct IconsPage {
    /// The filter text.
    pub query: String,
    /// Positions in [`compass_core::builtin_icon::names`] that match.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
}

impl IconsPage {
    /// Every builtin icon.
    #[must_use]
    pub fn new() -> Self {
        let mut page = Self::default();
        page.refilter();
        page
    }

    /// Recomputes `shown` for the query, over the icon's name.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let names = compass_core::builtin_icon::names();
        self.shown = filter(names.len(), &self.query, |index| vec![(names[index], 1.0)]);
    }

    /// The selected icon's name.
    #[must_use]
    pub fn selected_name(&self) -> Option<&'static str> {
        compass_core::builtin_icon::names()
            .get(*self.shown.get(self.selected)?)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, title: &str, description: &str) -> ExtensionManifest {
        ExtensionManifest {
            path: std::path::PathBuf::from("/extensions").join(id),
            id: id.into(),
            name: id.into(),
            title: title.into(),
            description: description.into(),
            icon: String::new(),
            author: "someone".into(),
            categories: Vec::new(),
            preferences: Vec::new(),
            commands: Vec::new(),
            needs_raycast_api: false,
            provenance: Provenance::from_id(id),
        }
    }

    #[test]
    fn extensions_filter_on_title_and_description_and_forget_an_uninstalled_one() {
        let mut page = ExtensionsPage::new(vec![
            manifest("store.raycast.github", "GitHub", "Work with issues"),
            manifest("notes", "Notes", "Write things down"),
        ]);
        assert_eq!(page.shown, [0, 1]);
        page.query = "write".into();
        page.refilter();
        assert_eq!(
            page.selected_extension().map(|e| e.id.as_str()),
            Some("notes")
        );
        page.remove("notes");
        assert!(page.selected_extension().is_none());
        assert_eq!(
            provenance_badge(page.all[0].provenance),
            "Raycast",
            "the store an id names"
        );
    }

    #[test]
    fn every_icon_is_listed_and_typing_finds_one() {
        let mut page = IconsPage::new();
        assert_eq!(page.shown.len(), compass_core::builtin_icon::names().len());
        page.query = "copy-clipboard".into();
        page.refilter();
        assert_eq!(page.selected_name(), Some("copy-clipboard"));
    }
}
