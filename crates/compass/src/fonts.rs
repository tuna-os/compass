//! Browse Fonts: the installed families, grouped and classified.
//!
//! The grouping, classification and specimens are
//! `compass_core::font_service`; this reads the font database. `fontdb`
//! finds the system's fonts the way the renderer does (the fontconfig
//! directories), and `ttf-parser` answers which scripts each covers from its
//! character map. Both are already what the launcher draws text with.

use compass_core::font_service::{
    self, BrowsedFamily, FontCategory, InstalledFamily, WritingSystem,
};
use compass_ipc::FontEntry;

/// Every installed family, sorted by name as `QFontDatabase::families()`
/// lists them, with its pitch and scripts (read from its first face).
#[must_use]
pub fn installed_families(database: &fontdb::Database) -> Vec<InstalledFamily> {
    let mut first_face: std::collections::BTreeMap<String, (fontdb::ID, bool)> =
        std::collections::BTreeMap::new();
    for face in database.faces() {
        let Some((family, _)) = face.families.first() else {
            continue;
        };
        first_face
            .entry(family.clone())
            .or_insert((face.id, face.monospaced));
    }
    let mut families: Vec<(String, fontdb::ID, bool)> = first_face
        .into_iter()
        .map(|(family, (id, mono))| (family, id, mono))
        .collect();
    families.sort_by_key(|(family, _, _)| family.to_lowercase());
    families
        .into_iter()
        .map(|(family, id, fixed_pitch)| {
            let systems = database
                .with_face_data(id, |data, index| {
                    ttf_parser::Face::parse(data, index).ok().map(|face| {
                        font_service::writing_systems(|c| face.glyph_index(c).is_some())
                    })
                })
                .flatten()
                .unwrap_or_default();
            InstalledFamily {
                family,
                fixed_pitch,
                systems,
            }
        })
        .collect()
}

/// The browser's entries for the system's fonts.
#[must_use]
pub fn browse() -> Vec<BrowsedFamily> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    font_service::browse_families(&installed_families(&database))
}

/// An entry as the wire carries it.
#[must_use]
pub fn entry(family: &BrowsedFamily) -> FontEntry {
    FontEntry {
        name: family.name.clone(),
        family: family.family.clone(),
        glyph: family.classified.glyph.map(str::to_owned),
        color: family.classified.color,
        primary: font_service::category_name(family.classified.primary).to_owned(),
        categories: font_service::ordered_categories()
            .into_iter()
            .filter(|category| family.classified.has(*category))
            .map(|category| font_service::category_name(category).to_owned())
            .collect(),
    }
}

/// The specimen shown when a font is previewed, as `specimenMarkdown`
/// writes it for the family's category and scripts.
#[must_use]
pub fn specimen(family: &BrowsedFamily) -> String {
    font_service::specimen_markdown(family.classified.primary, &family.systems, |system| {
        (system == WritingSystem::Latin).then(|| "The quick brown fox".to_owned())
    })
}

/// The category names in the browser's order, for its filter.
#[must_use]
pub fn category_names() -> Vec<String> {
    font_service::ordered_categories()
        .into_iter()
        .filter(|category| *category != FontCategory::Symbols)
        .map(|category| font_service::category_name(category).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_file_is_found_and_classified_by_what_it_covers() {
        // The renderer's own bundled font is not on the test machine's font
        // path; any font fontdb can read from the system proves the path.
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let installed = installed_families(&database);
        let browsed = font_service::browse_families(&installed);
        for family in &browsed {
            let entry = entry(family);
            assert!(!entry.name.is_empty());
            assert!(entry.categories.contains(&entry.primary), "{entry:?}");
        }
        assert!(category_names().iter().any(|name| name == "Latin"));
    }
}
