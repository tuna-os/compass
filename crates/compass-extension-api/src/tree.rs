//! The rendered tree, its id assignment pass and its flattening.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionItem, ActionPanel, HandlerId, Shortcut};
use crate::id::{Fnv, NodeId};
use crate::view::{Detail, EmptyState, FormItem, FormView, GridView, ListView, View};

/// Hashes a small leaf value into `h`. Leaf values only — never a node with children,
/// which would make the walk quadratic.
fn leaf<T: Serialize>(value: &T, h: &mut Fnv) {
    match serde_json::to_string(value) {
        Ok(s) => h.write_framed(s.as_bytes()),
        // Unreachable for the types in this crate, but a fingerprint must never panic.
        Err(_) => h.write(b"\0serialize-failed"),
    }
}

macro_rules! fingerprint {
    ($($field:expr),* $(,)?) => {{
        let mut h = Fnv::new();
        $( leaf(&$field, &mut h); )*
        h.finish()
    }};
}

/// One node, flattened out of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSummary {
    /// The node's derived identity.
    pub id: NodeId,
    /// Its parent, or `None` for the root.
    pub parent: Option<NodeId>,
    /// A short tag naming the node's shape. Owned on the wire so that a tree can be
    /// read back from storage; borrowed in practice.
    pub kind: Cow<'static, str>,
    /// Hash of the node's *own* fields — children excluded, id excluded. Two nodes with
    /// the same id and the same fingerprint need no repaint.
    pub fingerprint: u64,
}

/// An action found in the tree, with everything dispatch needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRef {
    /// The action node's identity.
    pub node: NodeId,
    /// The token to hand back to the extension.
    pub handler: HandlerId,
    /// Title, for denial and log messages the user reads.
    pub title: String,
    /// Shortcut, if the action has one.
    pub shortcut: Option<Shortcut>,
}

#[derive(Debug, Default)]
pub(crate) struct Visit {
    pub(crate) nodes: Vec<NodeSummary>,
    pub(crate) actions: Vec<ActionRef>,
}

impl Visit {
    fn push(&mut self, id: NodeId, parent: Option<NodeId>, kind: &'static str, fingerprint: u64) {
        self.nodes.push(NodeSummary {
            id,
            parent,
            kind: Cow::Borrowed(kind),
            fingerprint,
        });
    }
}

/// A rendered view tree with identity assigned.
///
/// Construct with [`ViewTree::new`]; that is the only way ids get stamped, and it is a
/// pure function of the tree's shape and keys.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ViewTree {
    root: View,
}

impl ViewTree {
    /// Assigns identity to every node and returns the tree.
    #[must_use]
    pub fn new(mut root: View) -> Self {
        let id = NodeId::ROOT.child("root", crate::id::NodeKey::Stable(root.tag()));
        match &mut root {
            View::List(v) => assign_list(v, id),
            View::Grid(v) => assign_grid(v, id),
            View::Detail(v) => assign_detail(v, id),
            View::Form(v) => assign_form(v, id),
        }
        Self { root }
    }

    /// The root view.
    #[must_use]
    pub fn root(&self) -> &View {
        &self.root
    }

    /// Consumes the tree, yielding the root view.
    #[must_use]
    pub fn into_root(self) -> View {
        self.root
    }

    /// Every node, in render order.
    #[must_use]
    pub fn nodes(&self) -> Vec<NodeSummary> {
        self.visit().nodes
    }

    /// Every action reachable in the tree, in render order.
    #[must_use]
    pub fn actions(&self) -> Vec<ActionRef> {
        self.visit().actions
    }

    pub(crate) fn visit(&self) -> Visit {
        let mut v = Visit::default();
        match &self.root {
            View::List(x) => visit_list(x, None, &mut v),
            View::Grid(x) => visit_grid(x, None, &mut v),
            View::Detail(x) => visit_detail(x, None, "detail", &mut v),
            View::Form(x) => visit_form(x, None, &mut v),
        }
        v
    }
}

// ---------------------------------------------------------------------------
// identity assignment
// ---------------------------------------------------------------------------

fn assign_panel(panel: &mut ActionPanel, parent: NodeId) {
    panel.id = parent.child("actions", crate::id::NodeKey::Index(0));
    for (si, section) in panel.sections.iter_mut().enumerate() {
        section.id = panel
            .id
            .child_keyed("sections", section.title.as_deref(), si);
        assign_items(&mut section.items, section.id);
    }
}

fn assign_items(items: &mut [ActionItem], parent: NodeId) {
    for (i, item) in items.iter_mut().enumerate() {
        match item {
            ActionItem::Action(a) => {
                a.id = parent.child_keyed("items", a.key.as_deref(), i);
            }
            ActionItem::Submenu(s) => {
                s.id = parent.child_keyed("items", s.key.as_deref(), i);
                let sid = s.id;
                assign_items(&mut s.items, sid);
            }
        }
    }
}

fn assign_empty(empty: &mut EmptyState, parent: NodeId) {
    empty.id = parent.child("empty", crate::id::NodeKey::Index(0));
    if let Some(p) = &mut empty.actions {
        assign_panel(p, empty.id);
    }
}

fn assign_detail(detail: &mut Detail, id: NodeId) {
    detail.id = id;
    if let Some(p) = &mut detail.actions {
        assign_panel(p, id);
    }
}

fn assign_list(view: &mut ListView, id: NodeId) {
    view.id = id;
    for (si, section) in view.sections.iter_mut().enumerate() {
        section.id = id.child_keyed(
            "sections",
            section.key.as_deref().or(section.title.as_deref()),
            si,
        );
        for (ii, item) in section.items.iter_mut().enumerate() {
            item.id = section.id.child_keyed("items", item.key.as_deref(), ii);
            let iid = item.id;
            if let Some(d) = &mut item.detail {
                assign_detail(d, iid.child("detail", crate::id::NodeKey::Index(0)));
            }
            if let Some(p) = &mut item.actions {
                assign_panel(p, iid);
            }
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
    if let Some(e) = &mut view.empty_state {
        assign_empty(e, id);
    }
}

fn assign_grid(view: &mut GridView, id: NodeId) {
    view.id = id;
    for (si, section) in view.sections.iter_mut().enumerate() {
        section.id = id.child_keyed(
            "sections",
            section.key.as_deref().or(section.title.as_deref()),
            si,
        );
        for (ii, item) in section.items.iter_mut().enumerate() {
            item.id = section.id.child_keyed("items", item.key.as_deref(), ii);
            let iid = item.id;
            if let Some(p) = &mut item.actions {
                assign_panel(p, iid);
            }
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
    if let Some(e) = &mut view.empty_state {
        assign_empty(e, id);
    }
}

fn assign_form(view: &mut FormView, id: NodeId) {
    view.id = id;
    for (i, item) in view.items.iter_mut().enumerate() {
        match item {
            FormItem::Field(f) => f.id = id.child("items", crate::id::NodeKey::Stable(&f.name)),
            FormItem::Description { id: did, .. } => {
                *did = id.child("items", crate::id::NodeKey::Index(i));
            }
            FormItem::Separator { id: sid } => {
                *sid = id.child("items", crate::id::NodeKey::Index(i));
            }
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
}

// ---------------------------------------------------------------------------
// flattening
// ---------------------------------------------------------------------------

fn visit_panel(panel: &ActionPanel, parent: NodeId, v: &mut Visit) {
    v.push(
        panel.id,
        Some(parent),
        "action_panel",
        fingerprint!(panel.title),
    );
    for section in &panel.sections {
        v.push(
            section.id,
            Some(panel.id),
            "action_section",
            fingerprint!(section.title),
        );
        visit_action_items(&section.items, section.id, v);
    }
}

fn visit_action_items(items: &[ActionItem], parent: NodeId, v: &mut Visit) {
    for item in items {
        match item {
            ActionItem::Action(a) => visit_action(a, parent, v),
            ActionItem::Submenu(s) => {
                v.push(
                    s.id,
                    Some(parent),
                    "action_submenu",
                    fingerprint!(s.key, s.title, s.icon, s.shortcut, s.on_open),
                );
                visit_action_items(&s.items, s.id, v);
            }
        }
    }
}

fn visit_action(a: &Action, parent: NodeId, v: &mut Visit) {
    v.push(
        a.id,
        Some(parent),
        "action",
        fingerprint!(a.key, a.title, a.icon, a.shortcut, a.style, a.handler),
    );
    v.actions.push(ActionRef {
        node: a.id,
        handler: a.handler.clone(),
        title: a.title.clone(),
        shortcut: a.shortcut.clone(),
    });
}

fn visit_empty(e: &EmptyState, parent: NodeId, v: &mut Visit) {
    v.push(
        e.id,
        Some(parent),
        "empty_state",
        fingerprint!(e.title, e.description, e.icon),
    );
    if let Some(p) = &e.actions {
        visit_panel(p, e.id, v);
    }
}

fn visit_detail(d: &Detail, parent: Option<NodeId>, kind: &'static str, v: &mut Visit) {
    v.push(
        d.id,
        parent,
        kind,
        fingerprint!(d.markdown, d.metadata, d.is_loading, d.navigation_title),
    );
    if let Some(p) = &d.actions {
        visit_panel(p, d.id, v);
    }
}

fn visit_list(view: &ListView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "list",
        fingerprint!(
            view.navigation_title,
            view.is_loading,
            view.show_detail,
            view.selected,
            view.on_selection_change,
            view.search,
            view.pagination,
        ),
    );
    for section in &view.sections {
        v.push(
            section.id,
            Some(view.id),
            "list_section",
            fingerprint!(section.key, section.title, section.subtitle),
        );
        for item in &section.items {
            v.push(
                item.id,
                Some(section.id),
                "list_item",
                fingerprint!(
                    item.key,
                    item.title,
                    item.subtitle,
                    item.icon,
                    item.accessories,
                    item.keywords
                ),
            );
            if let Some(d) = &item.detail {
                visit_detail(d, Some(item.id), "item_detail", v);
            }
            if let Some(p) = &item.actions {
                visit_panel(p, item.id, v);
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
    if let Some(e) = &view.empty_state {
        visit_empty(e, view.id, v);
    }
}

fn visit_grid(view: &GridView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "grid",
        fingerprint!(
            view.navigation_title,
            view.is_loading,
            view.columns,
            view.aspect_ratio,
            view.inset,
            view.fit,
            view.selected,
            view.on_selection_change,
            view.search,
            view.pagination,
        ),
    );
    for section in &view.sections {
        v.push(
            section.id,
            Some(view.id),
            "grid_section",
            fingerprint!(
                section.key,
                section.title,
                section.subtitle,
                section.columns,
                section.aspect_ratio,
                section.inset,
                section.fit
            ),
        );
        for item in &section.items {
            v.push(
                item.id,
                Some(section.id),
                "grid_item",
                fingerprint!(
                    item.key,
                    item.title,
                    item.subtitle,
                    item.content,
                    item.tooltip,
                    item.keywords
                ),
            );
            if let Some(p) = &item.actions {
                visit_panel(p, item.id, v);
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
    if let Some(e) = &view.empty_state {
        visit_empty(e, view.id, v);
    }
}

fn visit_form(view: &FormView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "form",
        fingerprint!(view.navigation_title, view.is_loading, view.enable_drafts),
    );
    for item in &view.items {
        match item {
            FormItem::Field(f) => v.push(
                f.id,
                Some(view.id),
                "form_field",
                fingerprint!(
                    f.name,
                    f.title,
                    f.error,
                    f.info,
                    f.autofocus,
                    f.value,
                    f.on_change,
                    f.kind
                ),
            ),
            FormItem::Description { id, title, text } => {
                v.push(
                    *id,
                    Some(view.id),
                    "form_description",
                    fingerprint!(title, text),
                );
            }
            FormItem::Separator { id } => {
                v.push(*id, Some(view.id), "form_separator", fingerprint!(()));
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
}
