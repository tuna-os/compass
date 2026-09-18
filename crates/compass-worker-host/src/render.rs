//! What `UI/render` carries.
//!
//! The extension's React tree is reconciled in the worker, not here:
//! `src/typescript/extension-manager/src/reconciler.ts` keeps a tree of
//! instances and, on every frame, sends the host
//!
//! ```ts
//! globalState.client.UI.render(JSON.stringify({ views }));
//! ```
//!
//! where `views` is the navigation stack — one entry per `view` slot, each the
//! last child of that slot.
//!
//! # A view with no root is unchanged, not empty
//!
//! The reconciler sends `{ dirty, root: dirty ? viewRoot : undefined }`. So a
//! view whose subtree did not change this frame arrives as `{"dirty": false}`
//! with **no root at all**, and it means *keep what you had*. A host that
//! treated a missing root as an empty view would blank a screen every time
//! something unrelated re-rendered. [`ViewFrame::is_unchanged`] is that
//! distinction, and a test pins it against the reconciler's own line.
//!
//! # Nodes are `$t` plus whatever the component passed
//!
//! `createInstance` builds `{ $t: type }` and copies every prop onto it, with
//! two substitutions: a React element in a prop is dropped with a console
//! error, and **a function becomes a callback id string** —
//! `callbackManager.subscribe(v)` — which is the id that later comes back
//! through `EventCore/handlerActivated`. So a handler, on the wire, is a
//! string prop like any other, and the only way to know which props are
//! handlers is to know the component. [`RenderNode::children`] is the one prop
//! this type reads; everything else stays as JSON, because inventing a typed
//! model here would be a second component model to keep in step with the
//! first.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One frame of the navigation stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewFrame {
    /// Whether this view changed in the frame that produced it.
    #[serde(default)]
    pub dirty: bool,
    /// The view's tree, present only when `dirty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<RenderNode>,
}

impl ViewFrame {
    /// Whether this frame says "keep what you had".
    ///
    /// True when there is no root. `dirty` is not consulted: the reconciler
    /// sends `{ dirty: true }` with no root for a view slot that has no child
    /// yet, and that is also nothing to draw.
    #[must_use]
    pub fn is_unchanged(&self) -> bool {
        self.root.is_none()
    }
}

/// A node of a rendered view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderNode {
    /// The intrinsic element's name, from `jsx.d.ts` — `list`, `list-item`,
    /// `action-panel` and so on.
    #[serde(rename = "$t")]
    pub tag: String,
    /// Children, in order. Absent rather than empty when there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<RenderNode>>,
    /// Every other prop, as it arrived.
    #[serde(flatten)]
    pub props: BTreeMap<String, serde_json::Value>,
}

impl RenderNode {
    /// The node's children, or nothing.
    #[must_use]
    pub fn children(&self) -> &[RenderNode] {
        self.children.as_deref().unwrap_or(&[])
    }

    /// This node and every node beneath it.
    pub fn walk(&self) -> impl Iterator<Item = &RenderNode> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children().iter().rev());
            Some(node)
        })
    }

    /// A prop, whatever its type.
    #[must_use]
    pub fn prop(&self, name: &str) -> Option<&serde_json::Value> {
        self.props.get(name)
    }
}

/// The whole payload of one `UI/render`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPayload {
    /// The navigation stack, outermost first.
    #[serde(default)]
    pub views: Vec<ViewFrame>,
}

impl RenderPayload {
    /// Reads a payload out of the `json` parameter of `UI/render`.
    ///
    /// # Errors
    ///
    /// [`serde_json::Error`] if it is not the shape the reconciler sends.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The topmost view that carries a tree, which is what a front end draws.
    #[must_use]
    pub fn top(&self) -> Option<&RenderNode> {
        self.views
            .iter()
            .rev()
            .find_map(|frame| frame.root.as_ref())
    }
}

/// Applies a frame to the stack a host is holding.
///
/// The rule the reconciler's shape implies: a frame with a root replaces that
/// entry, a frame without one leaves it alone, and the stack becomes as long
/// as the frame list — a pop is a shorter list, not a message.
///
/// Returns the new stack.
#[must_use]
pub fn apply(previous: &[Option<RenderNode>], payload: &RenderPayload) -> Vec<Option<RenderNode>> {
    payload
        .views
        .iter()
        .enumerate()
        .map(|(index, frame)| match &frame.root {
            Some(root) => Some(root.clone()),
            None => previous.get(index).cloned().flatten(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const JSX: &str = "src/typescript/api/types/jsx.d.ts";
    const RECONCILER: &str = "src/typescript/extension-manager/src/reconciler.ts";

    fn read(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    #[test]
    fn a_frame_with_no_root_means_unchanged_and_the_reconciler_still_says_so() {
        assert!(
            read(RECONCILER).contains("return { dirty, root: dirty ? viewRoot : undefined };"),
            "{RECONCILER} no longer omits the root of an unchanged view; the rule this module \
             is built on has moved"
        );

        let payload = RenderPayload::parse(r#"{"views":[{"dirty":false}]}"#).expect("parses");
        assert!(payload.views[0].is_unchanged());
        assert_eq!(
            payload.top(),
            None,
            "an unchanged frame has nothing to draw; drawing it as empty would blank the screen"
        );
    }

    #[test]
    fn an_unchanged_frame_keeps_what_was_there() {
        let first = RenderPayload::parse(r#"{"views":[{"dirty":true,"root":{"$t":"list"}}]}"#)
            .expect("parses");
        let stack = apply(&[], &first);
        assert_eq!(
            stack[0].as_ref().map(|n| n.tag.as_str()),
            Some("list"),
            "the first frame should have been taken"
        );

        let second = RenderPayload::parse(r#"{"views":[{"dirty":false}]}"#).expect("parses");
        let stack = apply(&stack, &second);
        assert_eq!(
            stack[0].as_ref().map(|n| n.tag.as_str()),
            Some("list"),
            "an unchanged frame replaced the view with nothing"
        );
    }

    #[test]
    fn a_push_adds_a_frame_and_a_pop_shortens_the_stack() {
        let stack = apply(
            &[],
            &RenderPayload::parse(
                r#"{"views":[{"dirty":false,"root":{"$t":"list"}},{"dirty":true,"root":{"$t":"detail"}}]}"#,
            )
            .expect("parses"),
        );
        assert_eq!(stack.len(), 2);

        let popped = apply(
            &stack,
            &RenderPayload::parse(r#"{"views":[{"dirty":false}]}"#).expect("parses"),
        );
        assert_eq!(popped.len(), 1, "a pop is a shorter list, not a message");
        assert_eq!(popped[0].as_ref().map(|n| n.tag.as_str()), Some("list"));
    }

    #[test]
    fn a_node_keeps_its_tag_its_children_and_every_other_prop() {
        let payload = RenderPayload::parse(
            r#"{"views":[{"dirty":true,"root":{
                "$t":"list",
                "isLoading":true,
                "onSelectionChange":"cb-7",
                "children":[{"$t":"list-item","title":"One"}]
            }}]}"#,
        )
        .expect("parses");

        let root = payload.top().expect("a root");
        assert_eq!(root.tag, "list");
        assert_eq!(root.prop("isLoading"), Some(&serde_json::json!(true)));
        assert_eq!(
            root.prop("onSelectionChange"),
            Some(&serde_json::json!("cb-7")),
            "a handler arrives as a callback id string, not as a function"
        );
        assert!(
            root.prop("children").is_none(),
            "children are the one prop this type reads, and must not be duplicated into props"
        );
        assert_eq!(root.children().len(), 1);
        assert_eq!(
            root.children()[0].prop("title"),
            Some(&serde_json::json!("One"))
        );
    }

    #[test]
    fn walking_a_tree_visits_every_node_in_order() {
        let payload = RenderPayload::parse(
            r#"{"views":[{"dirty":true,"root":{"$t":"list","children":[
                {"$t":"list-section","children":[{"$t":"list-item"},{"$t":"list-item"}]},
                {"$t":"empty-view"}
            ]}}]}"#,
        )
        .expect("parses");

        let tags: Vec<&str> = payload
            .top()
            .expect("a root")
            .walk()
            .map(|n| n.tag.as_str())
            .collect();
        assert_eq!(
            tags,
            vec![
                "list",
                "list-section",
                "list-item",
                "list-item",
                "empty-view"
            ]
        );
    }

    #[test]
    fn a_payload_round_trips_without_losing_props() {
        // The host will hand this on, and anything it silently drops here is
        // a component's prop that never reaches whoever draws it.
        let json = r#"{"views":[{"dirty":true,"root":{"$t":"grid","columns":5,"fit":"contain","children":[{"$t":"grid-item","content":{"source":"x.png"}}]}}]}"#;
        let payload = RenderPayload::parse(json).expect("parses");
        let again = serde_json::to_string(&payload).expect("serialises");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&again).expect("valid"),
            serde_json::from_str::<serde_json::Value>(json).expect("valid"),
        );
    }

    #[test]
    fn every_tag_this_module_names_in_a_test_is_one_the_jsx_types_declare() {
        // Not a claim to handle them: a guard against tests written against
        // tags that do not exist, which would pass for ever while proving
        // nothing about the real component set.
        let jsx = read(JSX);
        for tag in [
            "list",
            "list-item",
            "list-section",
            "detail",
            "grid",
            "grid-item",
            "empty-view",
            "form",
            "action-panel",
        ] {
            assert!(
                jsx.contains(&format!("\t\t\t{tag}: {{"))
                    || jsx.contains(&format!("\t\t\t\"{tag}\": {{")),
                "{JSX} does not declare <{tag}>"
            );
        }
    }
}
