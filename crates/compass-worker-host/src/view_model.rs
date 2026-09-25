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
//! `grid` (sections, cells with their image or colour, and actions) and
//! `form` (its fields, their values and echo counts, descriptions,
//! separators, and `Action.SubmitForm`'s `onSubmit`) are covered the same way.
//! Anything else at the root is an [`Unsupported`] naming the tag, so a front
//! end can say which component it cannot draw yet instead of drawing nothing.
//! Props a covered component has but this does not read (accessory icons,
//! colours) are dropped, not errors: a row without its icon is still the
//! row.

use compass_extension_api::action::{
    Action, ActionItem, ActionPanel, ActionSection, ActionSubmenu, KeyModifier, Shortcut,
};
use compass_extension_api::input::Seq;
use compass_extension_api::view::{
    Accessory, Color, Detail, Dropdown, DropdownOption, DropdownSection, EmptyState, FieldKind,
    FieldValue, FormField, FormItem, FormView, GridContent, GridFit, GridInset, GridItem,
    GridSection, GridView, Image, ImageSource, ListItem, ListSection, ListView, View,
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
/// [`Unsupported`] for a root this does not cover.
pub fn to_view(root: &RenderNode) -> Result<View, Unsupported> {
    match root.tag.as_str() {
        "list" => Ok(View::List(list(root))),
        "detail" => Ok(View::Detail(detail(root))),
        "grid" => Ok(View::Grid(grid(root))),
        "form" => Ok(View::Form(form(root))),
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

fn grid(node: &RenderNode) -> GridView {
    let mut view = GridView {
        navigation_title: text(node, "navigationTitle"),
        is_loading: flag(node, "isLoading"),
        columns: columns(node),
        inset: inset(node).unwrap_or_default(),
        fit: fit(node).unwrap_or_default(),
        on_selection_change: handler(node, "onSelectionChange"),
        ..GridView::default()
    };
    view.search.placeholder = text(node, "searchBarPlaceholder");
    view.search.on_change = handler(node, "onSearchTextChange");
    view.search.host_filtering = node
        .props
        .get("filtering")
        .and_then(Value::as_bool)
        .unwrap_or(view.search.on_change.is_none());

    let mut loose = Vec::new();
    let flush = |sections: &mut Vec<GridSection>, loose: &mut Vec<GridItem>| {
        if !loose.is_empty() {
            sections.push(GridSection {
                items: std::mem::take(loose),
                ..GridSection::default()
            });
        }
    };
    for child in node.children() {
        match child.tag.as_str() {
            "grid-section" => {
                flush(&mut view.sections, &mut loose);
                view.sections.push(GridSection {
                    title: text(child, "title"),
                    subtitle: text(child, "subtitle"),
                    columns: columns(child),
                    inset: inset(child),
                    fit: fit(child),
                    items: child
                        .children()
                        .iter()
                        .filter(|item| item.tag == "grid-item")
                        .map(grid_item)
                        .collect(),
                    ..GridSection::default()
                });
            }
            "grid-item" => loose.push(grid_item(child)),
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

fn form(node: &RenderNode) -> FormView {
    let mut view = FormView {
        navigation_title: text(node, "navigationTitle"),
        is_loading: flag(node, "isLoading"),
        enable_drafts: flag(node, "enableDrafts"),
        ..FormView::default()
    };
    for child in node.children() {
        match child.tag.as_str() {
            "action-panel" => view.actions = Some(panel(child)),
            "separator" => view.items.push(FormItem::Separator {
                id: compass_extension_api::id::NodeId::ROOT,
            }),
            "form-description" => view.items.push(FormItem::Description {
                id: compass_extension_api::id::NodeId::ROOT,
                title: text(child, "title"),
                text: text(child, "text").unwrap_or_default(),
            }),
            _ => {
                if let Some(field) = form_field(child) {
                    view.items.push(FormItem::Field(Box::new(field)));
                }
            }
        }
    }
    view
}

/// A field's `value` is `{value, eventCount}` where the API counts echoes
/// (`useEventCounted`), the bare value where it does not; `defaultValue` is
/// the extension's starting value and never an echo.
fn field_value(node: &RenderNode) -> (Option<&Value>, Option<Seq>) {
    match node.props.get("value") {
        Some(counted) if counted.get("eventCount").is_some() => (
            counted.get("value"),
            counted
                .get("eventCount")
                .and_then(Value::as_u64)
                .map(Seq::from_raw),
        ),
        Some(Value::Null) | None => (node.props.get("defaultValue"), None),
        Some(bare) => (Some(bare), None),
    }
}

fn form_field(node: &RenderNode) -> Option<FormField> {
    let placeholder = text(node, "placeholder");
    let kind = match node.tag.as_str() {
        "text-field" => FieldKind::Text { placeholder },
        "password-field" => FieldKind::Password { placeholder },
        "text-area-field" => FieldKind::TextArea {
            placeholder,
            markdown: flag(node, "enableMarkdown"),
        },
        "checkbox-field" => FieldKind::Checkbox {
            label: text(node, "label"),
        },
        "dropdown-field" => FieldKind::Dropdown(Dropdown {
            placeholder,
            sections: dropdown_sections(node),
            filtering: flag(node, "filtering"),
            ..Dropdown::default()
        }),
        "date-picker-field" => FieldKind::DatePicker {
            min: None,
            max: None,
            precision: compass_extension_api::view::DatePrecision::default(),
        },
        "tag-picker-field" => FieldKind::TagPicker {
            options: dropdown_sections(node)
                .into_iter()
                .flat_map(|section| section.options)
                .collect(),
            placeholder,
        },
        "file-picker-field" => FieldKind::FilePicker {
            allow_multiple: flag(node, "allowMultipleSelection"),
            allow_directories: flag(node, "canChooseDirectories"),
            allow_files: node
                .props
                .get("canChooseFiles")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        },
        _ => return None,
    };
    let (value, echo) = field_value(node);
    let value = value.and_then(|value| match (&kind, value) {
        (_, Value::Null) => None,
        (_, Value::Bool(on)) => Some(FieldValue::Bool(*on)),
        (_, Value::String(text)) if matches!(kind, FieldKind::DatePicker { .. }) => {
            Some(FieldValue::Date(text.clone()))
        }
        (_, Value::String(text)) => Some(FieldValue::Text(text.clone())),
        (FieldKind::FilePicker { .. }, Value::Array(all)) => Some(FieldValue::Paths(
            all.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        )),
        (_, Value::Array(all)) => Some(FieldValue::Values(
            all.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        )),
        (_, Value::Number(n)) => n.as_i64().map(FieldValue::Integer),
        _ => None,
    });
    Some(FormField {
        id: compass_extension_api::id::NodeId::ROOT,
        name: text(node, "id")?,
        title: text(node, "title"),
        error: text(node, "error"),
        info: text(node, "info"),
        autofocus: flag(node, "autoFocus"),
        value,
        echo,
        on_change: handler(node, "onChange"),
        kind,
    })
}

/// `dropdown-item`s, loose or in `dropdown-section`s.
fn dropdown_sections(node: &RenderNode) -> Vec<DropdownSection> {
    let option = |item: &RenderNode| {
        (item.tag == "dropdown-item").then(|| DropdownOption {
            title: text(item, "title").unwrap_or_default(),
            value: text(item, "value").unwrap_or_default(),
            icon: None,
            keywords: Vec::new(),
        })
    };
    let mut sections = Vec::new();
    let mut loose = Vec::new();
    for child in node.children() {
        if child.tag == "dropdown-section" {
            if !loose.is_empty() {
                sections.push(DropdownSection {
                    title: None,
                    options: std::mem::take(&mut loose),
                });
            }
            sections.push(DropdownSection {
                title: text(child, "title"),
                options: child.children().iter().filter_map(option).collect(),
            });
        } else {
            loose.extend(option(child));
        }
    }
    if !loose.is_empty() {
        sections.push(DropdownSection {
            title: None,
            options: loose,
        });
    }
    sections
}

fn columns(node: &RenderNode) -> Option<u16> {
    node.props
        .get("columns")
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
}

fn inset(node: &RenderNode) -> Option<GridInset> {
    match text(node, "inset")?.as_str() {
        "small" => Some(GridInset::Small),
        "medium" => Some(GridInset::Medium),
        "large" => Some(GridInset::Large),
        _ => None,
    }
}

fn fit(node: &RenderNode) -> Option<GridFit> {
    match text(node, "fit")?.as_str() {
        "fill" => Some(GridFit::Fill),
        "contain" => Some(GridFit::Contain),
        _ => None,
    }
}

fn grid_item(node: &RenderNode) -> GridItem {
    GridItem {
        id: compass_extension_api::id::NodeId::ROOT,
        key: text(node, "id"),
        title: text(node, "title").unwrap_or_default(),
        subtitle: text(node, "subtitle"),
        content: node
            .props
            .get("content")
            .and_then(grid_content)
            .unwrap_or_else(|| GridContent::Image(Image::builtin("question-mark-circle"))),
        tooltip: text(node, "tooltip"),
        keywords: node
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
            .unwrap_or_default(),
        actions: node
            .children()
            .iter()
            .find(|c| c.tag == "action-panel")
            .map(panel),
    }
}

/// `{color}` is a flat colour; anything else is `serializeProtoImage`'s shape.
fn grid_content(value: &Value) -> Option<GridContent> {
    match value.get("color") {
        Some(color) => color_like(color).map(GridContent::Color),
        None => image(value).map(GridContent::Image),
    }
}

/// `serializeColorLike`: `{raw}` or `{dynamic: {light, dark}}`, plain
/// strings accepted too. A dynamic colour is read as its light side.
fn color_like(color: &Value) -> Option<Color> {
    let raw = color
        .as_str()
        .or_else(|| color.get("raw").and_then(Value::as_str))
        .or_else(|| {
            let dynamic = color.get("dynamic").unwrap_or(color);
            dynamic
                .get("light")
                .or_else(|| dynamic.get("dark"))
                .and_then(Value::as_str)
        })?;
    Some(if raw.starts_with('#') {
        Color::Literal(raw.to_owned())
    } else {
        Color::Named(raw.to_owned())
    })
}

/// An image an extension sent as JSON, in any of the shapes the API
/// accepts: `serializeProtoImage`'s object, or a bare source string (a URL,
/// a path, an asset or a builtin icon name).
#[must_use]
pub fn image_from_json(value: &Value) -> Option<Image> {
    match value.as_str() {
        Some(raw) => {
            let mut image = Image::builtin(String::new());
            image.source = raw_source(raw);
            Some(image)
        }
        None => image(value),
    }
}

/// An image as `serializeProtoImage` writes it: `{source: {raw} | {themed}}`,
/// or `{fileIcon}`.
fn image(value: &Value) -> Option<Image> {
    if let Some(path) = value.get("fileIcon").and_then(Value::as_str) {
        let mut image = Image::builtin(String::new());
        image.source = ImageSource::FileIcon(path.to_owned());
        return Some(image);
    }
    let source = image_source(value.get("source")?)?;
    let mut image = Image::builtin(String::new());
    image.source = source;
    image.fallback = value.get("fallback").and_then(image_source);
    image.tint = value.get("tintColor").and_then(color_like);
    Some(image)
}

fn image_source(value: &Value) -> Option<ImageSource> {
    if let Some(raw) = value.as_str() {
        return Some(raw_source(raw));
    }
    if let Some(themed) = value.get("themed") {
        let side = |key: &str| themed.get(key).and_then(Value::as_str).map(raw_source);
        return match (side("light"), side("dark")) {
            (Some(light), Some(dark)) => Some(ImageSource::Themed {
                light: Box::new(light),
                dark: Box::new(dark),
            }),
            (Some(one), None) | (None, Some(one)) => Some(one),
            (None, None) => None,
        };
    }
    value.get("raw").and_then(Value::as_str).map(raw_source)
}

/// A URL, an absolute path, a file in the extension's assets, or a builtin
/// icon name, told apart the way the C++ `ImageURL` does.
fn raw_source(raw: &str) -> ImageSource {
    if raw.contains("://") || raw.starts_with("data:") {
        ImageSource::Url(raw.to_owned())
    } else if raw.starts_with('/') {
        ImageSource::Url(format!("file://{raw}"))
    } else if std::path::Path::new(raw).extension().is_some() {
        ImageSource::Asset(raw.to_owned())
    } else {
        ImageSource::Builtin(raw.to_owned())
    }
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
    item.icon = node.props.get("icon").and_then(image);
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
            // `Action.SubmitForm` renders a no-op `onAction` beside the real
            // `onSubmit`, which the host calls with the form's values.
            let handler = text(node, "onSubmit").or_else(|| text(node, "onAction"))?;
            let mut action = Action::new(text(node, "title").unwrap_or_default(), handler);
            action.key = text(node, "stableId");
            action.shortcut = node.props.get("shortcut").and_then(shortcut);
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
                     "keywords": ["launcher"], "icon": {"source": {"raw": "star"}},
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
        assert_eq!(compass.icon, Some(Image::builtin("star")));
        assert_eq!(list.sections[0].items[0].icon, None);
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

        let menu = node(serde_json::json!({"$t": "menu-bar"}));
        assert_eq!(
            to_view(&menu),
            Err(Unsupported {
                tag: "menu-bar".to_owned()
            })
        );
    }

    #[test]
    fn a_form_keeps_its_fields_values_and_submit_handler() {
        let root = node(serde_json::json!({
            "$t": "form", "navigationTitle": "New Issue",
            "children": [
                {"$t": "text-field", "id": "title", "title": "Title", "placeholder": "Short",
                 "value": {"value": "Crash", "eventCount": 3}, "onChange": "cb-1"},
                {"$t": "text-area-field", "id": "body", "defaultValue": "Steps"},
                {"$t": "separator"},
                {"$t": "checkbox-field", "id": "urgent", "label": "Urgent", "value": true},
                {"$t": "dropdown-field", "id": "repo", "children": [
                    {"$t": "dropdown-item", "title": "Compass", "value": "compass"},
                    {"$t": "dropdown-section", "title": "Forks", "children": [
                        {"$t": "dropdown-item", "title": "Vicinae", "value": "vicinae"}
                    ]}
                ]},
                {"$t": "form-description", "title": "Note", "text": "Be kind"},
                {"$t": "action-panel", "children": [
                    {"$t": "action", "title": "Create", "onAction": "cb-noop",
                     "onSubmit": "cb-submit"}
                ]}
            ]
        }));
        let View::Form(form) = to_view(&root).expect("a form") else {
            panic!("not a form");
        };
        assert_eq!(form.navigation_title.as_deref(), Some("New Issue"));
        assert_eq!(form.items.len(), 6);
        let FormItem::Field(title) = &form.items[0] else {
            panic!("not a field");
        };
        assert_eq!(title.name, "title");
        assert_eq!(title.value, Some(FieldValue::Text("Crash".into())));
        assert_eq!(title.echo, Some(Seq::from_raw(3)), "an echo of edit 3");
        assert_eq!(title.on_change.as_ref().map(|h| h.0.as_str()), Some("cb-1"));
        let FormItem::Field(body) = &form.items[1] else {
            panic!("not a field");
        };
        assert_eq!(
            (&body.value, body.echo),
            (&Some(FieldValue::Text("Steps".into())), None),
            "a default is not an echo"
        );
        assert!(matches!(form.items[2], FormItem::Separator { .. }));
        let FormItem::Field(urgent) = &form.items[3] else {
            panic!("not a field");
        };
        assert_eq!(urgent.value, Some(FieldValue::Bool(true)));
        let FormItem::Field(repo) = &form.items[4] else {
            panic!("not a field");
        };
        let FieldKind::Dropdown(dropdown) = &repo.kind else {
            panic!("not a dropdown");
        };
        let options: Vec<&str> = dropdown
            .sections
            .iter()
            .flat_map(|s| s.options.iter().map(|o| o.value.as_str()))
            .collect();
        assert_eq!(options, ["compass", "vicinae"]);
        assert!(matches!(
            &form.items[5],
            FormItem::Description { text, .. } if text == "Be kind"
        ));
        assert_eq!(
            form.actions.expect("actions").actions()[0].handler.0,
            "cb-submit",
            "SubmitForm runs onSubmit, not its no-op onAction"
        );
    }

    #[test]
    fn a_grid_keeps_its_cells_content_and_actions() {
        let root = node(serde_json::json!({
            "$t": "grid", "columns": 5, "inset": "small", "navigationTitle": "Emoji",
            "children": [
                {"$t": "grid-item", "title": "sun", "id": "s",
                 "content": {"source": {"raw": "sun-16"}}},
                {"$t": "grid-section", "title": "Photos", "columns": 3, "children": [
                    {"$t": "grid-item", "title": "cat", "keywords": ["pet"],
                     "content": {"source": {"raw": "https://example.com/cat.png"}},
                     "children": [{"$t": "action-panel", "children": [
                         {"$t": "action", "title": "Copy", "onAction": "cb-9"}
                     ]}]},
                    {"$t": "grid-item", "title": "red", "content": {"color": {"raw": "#ff0000"}}},
                    {"$t": "grid-item", "title": "logo", "content": {"source": {"raw": "logo.png"}}}
                ]}
            ]
        }));
        let View::Grid(grid) = to_view(&root).expect("a grid") else {
            panic!("not a grid");
        };
        assert_eq!(grid.navigation_title.as_deref(), Some("Emoji"));
        assert_eq!((grid.columns, grid.inset), (Some(5), GridInset::Small));
        assert!(grid.search.host_filtering);
        assert_eq!(grid.sections.len(), 2);
        assert_eq!(grid.sections[0].items[0].key.as_deref(), Some("s"));
        assert_eq!(
            grid.sections[0].items[0].content,
            GridContent::Image(Image::builtin("sun-16"))
        );

        let photos = &grid.sections[1];
        assert_eq!(
            (photos.title.as_deref(), photos.columns),
            (Some("Photos"), Some(3))
        );
        let cat = &photos.items[0];
        assert_eq!(cat.keywords, ["pet"]);
        assert!(matches!(
            &cat.content,
            GridContent::Image(image)
                if image.source == ImageSource::Url("https://example.com/cat.png".into())
        ));
        assert_eq!(
            cat.actions.as_ref().expect("actions").actions()[0]
                .handler
                .0,
            "cb-9"
        );
        assert_eq!(
            photos.items[1].content,
            GridContent::Color(Color::Literal("#ff0000".into()))
        );
        assert!(matches!(
            &photos.items[2].content,
            GridContent::Image(image) if image.source == ImageSource::Asset("logo.png".into())
        ));
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

/// The C++ `NAMED_SHORTCUTS` (`model-deser.cpp`), resolved to the defaults
/// `keybind-manager.cpp` gives them.
fn named_shortcut(name: &str) -> Option<Shortcut> {
    use KeyModifier::{Ctrl, Shift};
    let (modifiers, key): (&[KeyModifier], &str) = match name {
        "copy" | "copy-deeplink" => (&[Ctrl, Shift], "c"),
        "copy-name" => (&[Ctrl, Shift], "."),
        "copy-path" => (&[Ctrl, Shift], ","),
        "save" => (&[Ctrl], "s"),
        "duplicate" => (&[Ctrl], "d"),
        "edit" => (&[Ctrl], "e"),
        "move-down" => (&[Ctrl, Shift], "arrowDown"),
        "move-up" => (&[Ctrl, Shift], "arrowUp"),
        "new" => (&[Ctrl], "n"),
        "open" | "open-with" => (&[Ctrl], "o"),
        "pin" => (&[Ctrl, Shift], "p"),
        "refresh" => (&[Ctrl], "r"),
        "remove" => (&[Ctrl], "x"),
        "remove-all" => (&[Ctrl, Shift], "x"),
        _ => return None,
    };
    Some(Shortcut::new(modifiers.iter().copied(), key))
}

/// A `shortcut` prop: a named common shortcut, or `{key, modifiers}`.
///
/// `cmd` is Control here, as Qt makes it off macOS; `opt` is `alt`, and
/// `windows` is `meta`. A modifier this does not know drops the shortcut
/// rather than binding a different chord.
fn shortcut(value: &Value) -> Option<Shortcut> {
    if let Some(name) = value.as_str() {
        return named_shortcut(name);
    }
    let key = value.get("key")?.as_str().filter(|key| !key.is_empty())?;
    let mut modifiers = Vec::new();
    for modifier in value
        .get("modifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let modifier = match modifier.as_str()? {
            "cmd" | "ctrl" => KeyModifier::Ctrl,
            "opt" | "alt" => KeyModifier::Alt,
            "shift" => KeyModifier::Shift,
            "meta" | "windows" => KeyModifier::Meta,
            _ => return None,
        };
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
        }
    }
    Some(Shortcut::new(modifiers, key))
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn named_and_explicit_shortcuts_resolve_as_the_cpp_does() {
        assert_eq!(
            shortcut(&serde_json::json!("copy")),
            Some(Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "c"))
        );
        assert_eq!(
            shortcut(&serde_json::json!({"key": "r", "modifiers": ["cmd", "cmd", "shift"]})),
            Some(Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "r")),
            "cmd is Control off macOS, and a repeat is one modifier"
        );
        assert_eq!(shortcut(&serde_json::json!("nonsense")), None);
        assert_eq!(
            shortcut(&serde_json::json!({"key": "r", "modifiers": ["hyper"]})),
            None,
            "an unknown modifier drops the shortcut rather than binding plain r"
        );
    }
}
