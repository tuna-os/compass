//! A `UI/render` tree as `compass-extension-api`'s typed [`View`].
//!
//! [`crate::render`] keeps the wire tree untyped on purpose: it is a record of
//! what the reconciler sent, and inventing a second component model there
//! would drift from `jsx.d.ts`. A front end, though, draws a list, not a bag
//! of props, and PLAN §Phase 4 says the host consumes `compass-extension-api`
//! rather than defining its own model. This is that seam: one function from
//! the top [`RenderNode`] to a [`View`], so the Rhai tier and the TypeScript
//! tier hand the launcher the same thing.
//!
//! # What it covers
//!
//! `list` (sections, loose items, the list's own actions, the empty view),
//! `list-item` (title, subtitle, `id`, keywords, text and tag accessories,
//! actions, a Markdown detail) and `detail`, with `action-panel`,
//! `action-panel-section`, `action-panel-submenu` and `action` beneath them.
//! Anything else at the root is an [`Unsupported`] naming the tag, so a front
//! end can say which component it cannot draw yet instead of drawing nothing.
//! Props a covered component has but this does not read (icons, colours,
//! shortcuts) are dropped, not errors: a row without its icon is still the
//! row.

use compass_extension_api::action::{
    Action, ActionItem, ActionPanel, ActionSection, ActionSubmenu,
};
use compass_extension_api::view::{
    Accessory, Detail, EmptyState, ListItem, ListSection, ListView, View,
};
use serde_json::Value;

use crate::render::RenderNode;

/// A root component this build cannot turn into a [`View`] yet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Compass cannot draw the extension component <{tag}> yet")]
pub struct Unsupported {
    /// The component's tag, as `jsx.d.ts` names it.
    pub tag: String,
}

/// The view `root` describes.
///
/// # Errors
///
/// [`Unsupported`] for a root this does not cover (a `grid`, a `form`).
pub fn to_view(root: &RenderNode) -> Result<View, Unsupported> {
    match root.tag.as_str() {
        "list" => Ok(View::List(list(root))),
        "detail" => Ok(View::Detail(detail(root))),
        other => Err(Unsupported {
            tag: other.to_owned(),
        }),
    }
}

fn text(node: &RenderNode, prop: &str) -> Option<String> {
    node.props
        .get(prop)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn flag(node: &RenderNode, prop: &str) -> bool {
    node.props
        .get(prop)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn handler(node: &RenderNode, prop: &str) -> Option<compass_extension_api::action::HandlerId> {
    text(node, prop).map(compass_extension_api::action::HandlerId::new)
}

fn list(node: &RenderNode) -> ListView {
    let mut view = ListView {
        navigation_title: text(node, "navigationTitle"),
        is_loading: flag(node, "isLoading"),
        show_detail: flag(node, "isShowingDetail"),
        on_selection_change: handler(node, "onSelectionChange"),
        ..ListView::default()
    };
    view.search.placeholder = text(node, "searchBarPlaceholder");
    view.search.on_change = handler(node, "onSearchTextChange");
    // `filtering` defaults to on when the extension does not take over the
    // search text, as Raycast's List does.
    view.search.host_filtering = node
        .props
        .get("filtering")
        .and_then(Value::as_bool)
        .unwrap_or(view.search.on_change.is_none());

    let mut loose = Vec::new();
    for child in node.children() {
        match child.tag.as_str() {
            "list-section" => {
                flush(&mut view.sections, &mut loose);
                view.sections.push(ListSection {
                    title: text(child, "title"),
                    subtitle: text(child, "subtitle"),
                    items: child
                        .children()
                        .iter()
                        .filter(|item| item.tag == "list-item")
                        .map(item)
                        .collect(),
                    ..ListSection::default()
                });
            }
            "list-item" => loose.push(item(child)),
            "action-panel" => view.actions = Some(panel(child)),
            "empty-view" => {
                view.empty_state = Some(EmptyState {
                    title: text(child, "title").unwrap_or_default(),
                    description: text(child, "description"),
                    actions: child
                        .children()
                        .iter()
                        .find(|c| c.tag == "action-panel")
                        .map(panel),
                    ..EmptyState::default()
                });
            }
            _ => {}
        }
    }
    flush(&mut view.sections, &mut loose);
    view
}

/// Loose items between sections become an untitled section in place.
fn flush(sections: &mut Vec<ListSection>, loose: &mut Vec<ListItem>) {
    if !loose.is_empty() {
        sections.push(ListSection::untitled(std::mem::take(loose)));
    }
}

fn item(node: &RenderNode) -> ListItem {
    let mut item = ListItem::new(text(node, "title").unwrap_or_default());
    item.key = text(node, "id");
    item.subtitle = text(node, "subtitle");
    item.keywords = node
        .props
        .get("keywords")
        .and_then(Value::as_array)
        .map(|words| {
            words
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    item.accessories = node
        .props
        .get("accessories")
        .and_then(Value::as_array)
        .map(|all| all.iter().filter_map(accessory).collect())
        .unwrap_or_default();
    for child in node.children() {
        match child.tag.as_str() {
            "action-panel" => item.actions = Some(panel(child)),
            "list-item-detail" => item.detail = Some(detail(child)),
            _ => {}
        }
    }
    item
}

/// `text` and `tag` are each a string or `{value, color}`; anything with
/// neither is an icon-only accessory, which this does not draw.
fn accessory(value: &Value) -> Option<Accessory> {
    let read = |key: &str| match value.get(key)? {
        Value::String(text) => Some(text.clone()),
        other => other.get("value")?.as_str().map(str::to_owned),
    };
    let text = read("text");
    let tag = read("tag");
    (text.is_some() || tag.is_some()).then(|| Accessory {
        text,
        tag,
        tooltip: value
            .get("tooltip")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Accessory::default()
    })
}

fn detail(node: &RenderNode) -> Detail {
    Detail {
        markdown: text(node, "markdown"),
        navigation_title: text(node, "navigationTitle"),
        is_loading: flag(node, "isLoading"),
        actions: node
            .children()
            .iter()
            .find(|c| c.tag == "action-panel")
            .map(panel),
        ..Detail::default()
    }
}

fn panel(node: &RenderNode) -> ActionPanel {
    let mut panel = ActionPanel {
        title: text(node, "title"),
        ..ActionPanel::default()
    };
    let mut loose = Vec::new();
    for child in node.children() {
        match child.tag.as_str() {
            "action-panel-section" => {
                if !loose.is_empty() {
                    panel.sections.push(ActionSection {
                        items: std::mem::take(&mut loose),
                        ..ActionSection::default()
                    });
                }
                panel.sections.push(ActionSection {
                    title: text(child, "title"),
                    items: child.children().iter().filter_map(action_item).collect(),
                    ..ActionSection::default()
                });
            }
            _ => loose.extend(action_item(child)),
        }
    }
    if !loose.is_empty() {
        panel.sections.push(ActionSection {
            items: loose,
            ..ActionSection::default()
        });
    }
    panel
}

fn action_item(node: &RenderNode) -> Option<ActionItem> {
    match node.tag.as_str() {
        "action" => {
            let handler = text(node, "onAction")?;
            let mut action = Action::new(text(node, "title").unwrap_or_default(), handler);
            action.key = text(node, "stableId");
            Some(ActionItem::Action(action))
        }
        "action-panel-submenu" => Some(ActionItem::Submenu(ActionSubmenu {
            id: compass_extension_api::id::NodeId::ROOT,
            key: text(node, "stableId"),
            title: text(node, "title").unwrap_or_default(),
            icon: None,
            shortcut: None,
            on_open: handler(node, "onOpen"),
            items: node.children().iter().filter_map(action_item).collect(),
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(json: Value) -> RenderNode {
        serde_json::from_value(json).expect("a render node")
    }

    #[test]
    fn a_list_with_sections_loose_items_accessories_and_actions() {
        let root = node(serde_json::json!({
            "$t": "list",
            "navigationTitle": "Repos",
            "searchBarPlaceholder": "Filter",
            "onSelectionChange": "cb-1",
            "children": [
                {"$t": "list-item", "title": "loose", "id": "l"},
                {"$t": "list-section", "title": "Mine", "children": [
                    {"$t": "list-item", "title": "compass", "subtitle": "tuna-os", "id": "c",
                     "keywords": ["launcher"],
                     "accessories": [{"text": "12"}, {"tag": {"value": "rust", "color": "red"}},
                                     {"icon": "star"}],
                     "children": [
                        {"$t": "action-panel", "children": [
                            {"$t": "action", "title": "Copy URL", "onAction": "cb-7"},
                            {"$t": "action-panel-section", "title": "More", "children": [
                                {"$t": "action", "title": "Open", "onAction": "cb-8",
                                 "stableId": "open"}
                            ]}
                        ]},
                        {"$t": "list-item-detail", "markdown": "# compass"}
                     ]}
                ]}
            ]
        }));
        let View::List(list) = to_view(&root).expect("a list") else {
            panic!("not a list");
        };
        assert_eq!(list.navigation_title.as_deref(), Some("Repos"));
        assert_eq!(list.search.placeholder.as_deref(), Some("Filter"));
        assert!(
            list.search.host_filtering,
            "no onSearchTextChange: Compass filters"
        );
        assert_eq!(
            list.on_selection_change.as_ref().map(|h| h.0.as_str()),
            Some("cb-1")
        );
        assert_eq!(list.sections.len(), 2);
        assert_eq!(
            list.sections[0].title, None,
            "loose items become an untitled section"
        );
        assert_eq!(list.sections[0].items[0].title, "loose");

        let compass = &list.sections[1].items[0];
        assert_eq!(list.sections[1].title.as_deref(), Some("Mine"));
        assert_eq!(compass.key.as_deref(), Some("c"));
        assert_eq!(compass.subtitle.as_deref(), Some("tuna-os"));
        assert_eq!(compass.keywords, ["launcher"]);
        assert_eq!(
            compass.accessories.len(),
            2,
            "the icon-only accessory is not drawn"
        );
        assert_eq!(compass.accessories[0].text.as_deref(), Some("12"));
        assert_eq!(compass.accessories[1].tag.as_deref(), Some("rust"));
        assert_eq!(
            compass.detail.as_ref().and_then(|d| d.markdown.as_deref()),
            Some("# compass")
        );

        let actions = compass.actions.as_ref().expect("actions").actions();
        let handlers: Vec<(&str, &str)> = actions
            .iter()
            .map(|a| (a.title.as_str(), a.handler.0.as_str()))
            .collect();
        assert_eq!(handlers, [("Copy URL", "cb-7"), ("Open", "cb-8")]);
        assert_eq!(actions[1].key.as_deref(), Some("open"));
    }

    #[test]
    fn a_list_that_owns_its_search_text_is_not_filtered_by_compass() {
        let root = node(serde_json::json!({"$t": "list", "onSearchTextChange": "cb-2"}));
        let View::List(list) = to_view(&root).expect("a list") else {
            panic!("not a list");
        };
        assert!(!list.search.host_filtering);
        assert_eq!(
            list.search.on_change.as_ref().map(|h| h.0.as_str()),
            Some("cb-2")
        );
    }

    #[test]
    fn a_detail_and_an_unsupported_root() {
        let detail = node(
            serde_json::json!({"$t": "detail", "markdown": "hello", "children": [
                {"$t": "action-panel", "children": [{"$t": "action", "title": "Go", "onAction": "cb-3"}]}
            ]}),
        );
        let View::Detail(detail) = to_view(&detail).expect("a detail") else {
            panic!("not a detail");
        };
        assert_eq!(detail.markdown.as_deref(), Some("hello"));
        assert_eq!(
            detail.actions.expect("actions").actions()[0].handler.0,
            "cb-3"
        );

        let grid = node(serde_json::json!({"$t": "grid"}));
        assert_eq!(
            to_view(&grid),
            Err(Unsupported {
                tag: "grid".to_owned()
            })
        );
    }

    #[test]
    fn an_action_without_a_handler_is_left_out_rather_than_inert() {
        let root = node(serde_json::json!({"$t": "detail", "children": [
            {"$t": "action-panel", "children": [{"$t": "action", "title": "Nothing"}]}
        ]}));
        let View::Detail(detail) = to_view(&root).expect("a detail") else {
            panic!("not a detail");
        };
        assert!(detail.actions.expect("a panel").actions().is_empty());
    }
}
