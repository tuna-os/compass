//! Rhai scripts, as root-search entries.
//!
//! The scripts themselves (discovery, the sandbox, running them) are
//! `compass-script`'s and the engine's; this is only what root search needs to
//! list one next to applications and extension commands. Not to be confused
//! with [`crate::script_scan`], which is Raycast-style *script commands*.
//!
//! # Ids
//!
//! A script's id is `script.<folder name>` (`compass_script::manifest`), and
//! its root entry is `rhai:<that id>`, so frecency and aliases name it the
//! same way across restarts and hot reloads.

use crate::root_items::{RootItem, RootItemMeta, entrypoint_id, split_entrypoint_id};

/// The root provider every Rhai script is listed under.
pub const RHAI_PROVIDER_ID: &str = "rhai";

/// One Rhai script, as root search lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhaiScriptItem {
    /// `script.<folder name>`.
    pub id: String,
    /// The manifest's `title`.
    pub title: String,
    /// The manifest's `description`, the row's subtitle.
    pub description: Option<String>,
    /// A builtin icon name.
    pub icon: Option<String>,
    /// Extra search terms.
    pub keywords: Vec<String>,
}

impl RhaiScriptItem {
    /// `rhai:<id>`, the id root search and the engine address it by.
    #[must_use]
    pub fn entrypoint_id(&self) -> String {
        entrypoint_id(RHAI_PROVIDER_ID, &self.id)
    }

    /// What the row says under the title.
    #[must_use]
    pub fn subtitle(&self) -> &str {
        self.description.as_deref().unwrap_or("Script")
    }

    /// The root-search entry.
    #[must_use]
    pub fn root_item(&self) -> RootItem {
        RootItem {
            id: self.entrypoint_id(),
            title: self.title.clone(),
            unlocalized_title: None,
            subtitle: self.subtitle().to_owned(),
            keywords: self.keywords.clone(),
            meta: RootItemMeta {
                provider_id: RHAI_PROVIDER_ID.to_owned(),
                enabled: true,
                ..RootItemMeta::default()
            },
        }
    }
}

/// The script id a `rhai:<id>` entrypoint id names, if it is one.
#[must_use]
pub fn script_id(entrypoint: &str) -> Option<&str> {
    match split_entrypoint_id(entrypoint)? {
        (RHAI_PROVIDER_ID, id) if !id.is_empty() => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_is_listed_under_its_own_provider_and_found_by_its_keywords() {
        let item = RhaiScriptItem {
            id: "script.web-search".to_owned(),
            title: "Web Search".to_owned(),
            description: None,
            icon: Some("globe".to_owned()),
            keywords: vec!["google".to_owned()],
        };
        let root = item.root_item();
        assert_eq!(root.id, "rhai:script.web-search");
        assert_eq!(root.subtitle, "Script");
        assert_eq!(script_id(&root.id), Some("script.web-search"));
        assert_eq!(script_id("scripts:script.web-search"), None);
        assert_eq!(script_id("rhai:"), None);

        let roots = [root];
        let hits = crate::root_items::search_with_frecency(
            &roots,
            "google",
            &crate::root_items::SearchOptions::default(),
            |_, _| 0.0,
        );
        assert_eq!(hits.len(), 1);
    }
}
