//! Browse Fonts: the families, the category filter, and the specimen.
//!
//! The engine reads and classifies the fonts; `compass_core::font_browser`
//! decides what a query and a category show. This keeps the view's state and
//! turns a specimen's Markdown into lines drawn in the font itself.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use compass_core::font_browser::{self, FontFamily, Mode};

use crate::backend::FontList;

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The fonts are on their way.
    Loading,
    /// They arrived.
    Ready,
    /// They could not be listed, and why.
    Failed(String),
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct FontsPage {
    /// The search text.
    pub query: String,
    /// Every family, in the engine's order.
    pub families: Vec<FontFamily>,
    /// Each family's section, by name.
    pub primaries: HashMap<String, String>,
    /// The category names in the engine's order.
    pub categories: Vec<String>,
    /// The filter's options: `All`, then every category some font has.
    pub options: Vec<String>,
    /// The category filtered by; `None` for all.
    pub category: Option<String>,
    /// What the current query and category show.
    pub mode: Mode,
    /// Position in the shown families.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last action did not happen.
    pub notice: Option<String>,
}

impl Default for FontsPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            families: Vec::new(),
            primaries: HashMap::new(),
            categories: Vec::new(),
            options: vec![font_browser::ALL_OPTION.to_owned()],
            category: None,
            mode: font_browser::build(&[], "", None),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

impl FontsPage {
    /// Takes the engine's list.
    pub fn apply(&mut self, result: Result<FontList, String>) {
        match result {
            Ok(list) => {
                self.primaries = list
                    .fonts
                    .iter()
                    .map(|font| (font.name.clone(), font.primary.clone()))
                    .collect();
                self.families = list
                    .fonts
                    .into_iter()
                    .map(|font| FontFamily {
                        name: font.name,
                        family: font.family,
                        glyph: font.glyph.unwrap_or_default(),
                        color: font.color,
                        categories: font.categories,
                    })
                    .collect();
                self.options = font_browser::filter_options(&list.categories, &self.families);
                self.categories = list.categories;
                self.status = Status::Ready;
            }
            Err(reason) => self.status = Status::Failed(reason),
        }
        self.refilter();
    }

    /// Recomputes what the query and category show, back at the top.
    pub fn refilter(&mut self) {
        self.selected = 0;
        self.mode = font_browser::build(&self.families, &self.query, self.category.as_deref());
    }

    /// The families shown, and the list's heading.
    #[must_use]
    pub fn shown(&self) -> (&str, &[FontFamily]) {
        match &self.mode {
            Mode::Root { title, families } | Mode::Search { title, families } => (title, families),
        }
    }

    /// The selected family, if any.
    #[must_use]
    pub fn selected_family(&self) -> Option<&FontFamily> {
        self.shown().1.get(self.selected)
    }

    /// Filters by the option titled `option`; `All` clears the filter.
    pub fn set_category(&mut self, option: &str) {
        self.category = (option != font_browser::ALL_OPTION).then(|| option.to_owned());
        self.refilter();
    }
}

/// A family name as the renderer takes it: `iced::Font` names a family by a
/// `&'static str`, so each family drawn is interned once for the life of the
/// process (a machine has a few hundred at most).
#[must_use]
pub fn static_family(family: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let interned = INTERNED.get_or_init(Mutex::default);
    let Ok(mut interned) = interned.lock() else {
        return "sans-serif";
    };
    if let Some(name) = interned.get(family) {
        return name;
    }
    let leaked: &'static str = Box::leak(family.to_owned().into_boxed_str());
    interned.insert(family.to_owned(), leaked);
    leaked
}

/// One line of a specimen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecimenLine {
    /// A large sample: `# …`.
    Heading(String),
    /// A plain sample.
    Regular(String),
    /// A bold sample: `**…**`.
    Bold(String),
    /// An italic sample: `*…*`.
    Italic(String),
    /// A rule between scripts: `---`.
    Rule,
}

/// Reads `compass_core::font_service::specimen_markdown`'s blocks back into
/// lines, so each is drawn in the font rather than by the Markdown renderer.
#[must_use]
pub fn specimen_lines(markdown: &str) -> Vec<SpecimenLine> {
    markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line == "---" {
                SpecimenLine::Rule
            } else if let Some(rest) = line.strip_prefix("# ") {
                SpecimenLine::Heading(rest.to_owned())
            } else if let Some(rest) = line
                .strip_prefix("**")
                .and_then(|rest| rest.strip_suffix("**"))
            {
                SpecimenLine::Bold(rest.to_owned())
            } else if let Some(rest) = line
                .strip_prefix('*')
                .and_then(|rest| rest.strip_suffix('*'))
            {
                SpecimenLine::Italic(rest.to_owned())
            } else {
                SpecimenLine::Regular(line.to_owned())
            }
        })
        .collect()
}

/// The preview of one family.
#[derive(Debug, Clone)]
pub struct FontPreviewPage {
    /// The typeface's name.
    pub name: String,
    /// The member drawn.
    pub family: String,
    /// The specimen, line by line.
    pub lines: Vec<SpecimenLine>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::FontListEntry;

    fn list() -> FontList {
        let font = |name: &str, primary: &str, categories: &[&str]| FontListEntry {
            name: name.into(),
            family: name.into(),
            glyph: Some("Aa".into()),
            color: false,
            primary: primary.into(),
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
        };
        FontList {
            fonts: vec![
                font("Inter", "Latin", &["Latin", "Cyrillic"]),
                font("JetBrains Mono", "Monospace", &["Latin", "Monospace"]),
                font("Noto Sans Thai", "Thai", &["Thai"]),
            ],
            categories: vec![
                "Latin".into(),
                "Cyrillic".into(),
                "Monospace".into(),
                "Thai".into(),
                "Emoji".into(),
            ],
        }
    }

    #[test]
    fn the_filter_offers_what_the_fonts_have_and_narrows_the_list() {
        let mut page = FontsPage::default();
        page.apply(Ok(list()));
        assert_eq!(
            page.options,
            ["All", "Latin", "Cyrillic", "Monospace", "Thai"]
        );
        assert_eq!(page.shown().0, "All Fonts (3)");
        page.set_category("Monospace");
        assert_eq!(page.shown().0, "Monospace (1)");
        assert_eq!(
            page.selected_family().map(|f| f.name.as_str()),
            Some("JetBrains Mono")
        );
        page.set_category("All");
        page.query = "sans thai".into();
        page.refilter();
        assert_eq!(page.shown().0, "Results (1)");
    }

    #[test]
    fn a_specimen_reads_back_as_lines() {
        let lines = specimen_lines("# Sample\n\nSample\n\n**Sample**\n\n*Sample*\n\n---\n\n");
        assert_eq!(
            lines,
            [
                SpecimenLine::Heading("Sample".into()),
                SpecimenLine::Regular("Sample".into()),
                SpecimenLine::Bold("Sample".into()),
                SpecimenLine::Italic("Sample".into()),
                SpecimenLine::Rule,
            ]
        );
        assert!(std::ptr::eq(static_family("Inter"), static_family("Inter")));
    }
}
