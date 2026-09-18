//! `UI/render`, and the event that answers it.
//!
//! This is the first half of the largest piece of the extension API. It does
//! two things and refuses the rest by name:
//!
//! * **`UI/render`** parses the payload ([`crate::render`]) and keeps the
//!   navigation stack, applying the reconciler's "a view with no root is
//!   unchanged" rule. Whoever draws it asks [`UiService::stack`].
//! * **`EventCore/handlerActivated`** is the way back: an action in the UI
//!   carries a callback id that arrived as an ordinary string prop, and
//!   [`handler_activated`] builds the event that fires it.
//!
//! # What it does not do
//!
//! The rest of `UI` — toasts, HUD, navigation, search and selected text — is
//! [`crate::ui_shell_service`], which delegates to a `Shell` trait rather than
//! accepting the call and doing nothing: an extension that believed the user
//! had been told something would be worse off than one whose call failed.
//! `UI/confirmAlert` is still refused by name, because it answers whenever the
//! *user* does and the host cannot hold a reply open yet.

use std::sync::Mutex;

use crate::render::{RenderNode, RenderPayload, apply};
use crate::tsapi::{self, Call};

/// The methods this serves.
pub const METHODS: &[&str] = &["UI/render"];

/// Holds the last rendered navigation stack.
#[derive(Debug, Default)]
pub struct UiService {
    stack: Mutex<Vec<Option<RenderNode>>>,
}

impl UiService {
    /// A service with an empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current stack, outermost first.
    ///
    /// An entry is `None` when the worker has never sent a tree for that slot,
    /// which is not the same as an empty view.
    #[must_use]
    pub fn stack(&self) -> Vec<Option<RenderNode>> {
        self.lock().clone()
    }

    /// The topmost view that has a tree.
    #[must_use]
    pub fn top(&self) -> Option<RenderNode> {
        self.lock().iter().rev().find_map(Clone::clone)
    }

    /// The stack, recovering from a poisoned lock.
    ///
    /// A panic in a handler must not take the whole host's UI with it: the
    /// data is a cache of what the worker last said, and the worker will say
    /// it again on the next frame.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Option<RenderNode>>> {
        self.stack
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Answers `call`, or `None` if it is not ours.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let Some(json) = call.params.get("json").and_then(serde_json::Value::as_str) else {
            return Some(tsapi::reply_error(
                id,
                "UI/render was called without its `json` parameter",
            ));
        };

        match RenderPayload::parse(json) {
            Ok(payload) => {
                let mut stack = self.lock();
                *stack = apply(&stack, &payload);
                // `render` returns void, and the generator replies null.
                Some(tsapi::reply(id, serde_json::Value::Null))
            }
            // Not a panic and not a silent drop: an extension whose render
            // this host cannot read should see it fail, with the reason.
            Err(error) => Some(tsapi::reply_error(
                id,
                &format!("UI/render payload could not be read: {error}"),
            )),
        }
    }
}

impl tsapi::Service for UiService {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The payload that fires a handler an extension registered.
///
/// `figura/tsapi.fig` declares `event handlerActivated(id: string, args:
/// any[])` on `EventCore`, and the generator emits events as
/// `{ jsonrpc, method, params: { id, args } }` with no message id — an event
/// is answered by not answering.
///
/// `id` is the callback id the reconciler substituted for the function prop,
/// so it comes straight off a node: an action's `onAction`, a list's
/// `onSelectionChange`.
#[must_use]
pub fn handler_activated(handler_id: &str, args: &[serde_json::Value]) -> String {
    tsapi::event(
        "EventCore/handlerActivated",
        serde_json::json!({ "id": handler_id, "args": args }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn render_call(id: u64, json: &str) -> Call {
        Call {
            jsonrpc: crate::rpc::VERSION.to_owned(),
            id: Some(id),
            method: "UI/render".to_owned(),
            params: serde_json::json!({ "json": json }),
        }
    }

    fn result_of(service: &UiService, call: &Call) -> serde_json::Value {
        let payload = UiService::handle(service, call).expect("a UI call is answered");
        serde_json::from_str(&payload).expect("a JSON reply")
    }

    #[test]
    fn render_is_on_the_ledger_and_confirm_alert_is_not() {
        assert!(tsapi::is_implemented("UI/render"));
        // The shell half of `UI` now has a home (`crate::ui_shell_service`), so
        // its methods are on the ledger. `confirmAlert` is not: it answers
        // whenever the user does, and a host that replied for them would hand
        // an extension a decision nobody made.
        assert!(
            !tsapi::is_implemented("UI/confirmAlert"),
            "confirmAlert suspends on a person; the host cannot hold a reply open yet"
        );
    }

    #[test]
    fn the_shell_methods_are_not_answered_by_the_render_service() {
        // Two services, one prefix: a `UI/showToast` that fell through to the
        // render path would be answered with a render error rather than served.
        let service = UiService::new();
        for method in crate::ui_shell_service::METHODS {
            let call = crate::tsapi::parse(&format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{{}}}}"#
            ))
            .expect("a well-formed call");
            assert!(UiService::handle(&service, &call).is_none(), "{method}");
        }
    }

    #[test]
    fn a_render_is_kept_and_answered_with_null() {
        let service = UiService::new();
        let answer = result_of(
            &service,
            &render_call(1, r#"{"views":[{"dirty":true,"root":{"$t":"list"}}]}"#),
        );
        assert_eq!(answer["result"], serde_json::Value::Null);
        assert!(answer.get("error").is_none());

        assert_eq!(service.top().map(|n| n.tag), Some("list".to_owned()));
    }

    #[test]
    fn an_unchanged_frame_does_not_clear_the_view() {
        // The rule the reconciler's shape implies, at the layer that would
        // actually blank someone's screen.
        let service = UiService::new();
        result_of(
            &service,
            &render_call(1, r#"{"views":[{"dirty":true,"root":{"$t":"detail"}}]}"#),
        );
        result_of(&service, &render_call(2, r#"{"views":[{"dirty":false}]}"#));

        assert_eq!(
            service.top().map(|n| n.tag),
            Some("detail".to_owned()),
            "an unchanged frame replaced the view with nothing"
        );
    }

    #[test]
    fn a_payload_that_cannot_be_read_is_an_error_reply_naming_the_reason() {
        let service = UiService::new();
        let answer = result_of(&service, &render_call(1, "{this is not json"));
        assert!(
            answer["error"]
                .as_str()
                .is_some_and(|e| e.contains("could not be read")),
            "{answer}"
        );
        assert!(
            service.stack().is_empty(),
            "a payload that could not be read must not have changed the stack"
        );
    }

    #[test]
    fn a_render_with_no_json_parameter_is_refused_rather_than_treated_as_empty() {
        let service = UiService::new();
        let mut call = render_call(1, "");
        call.params = serde_json::json!({});
        let answer = result_of(&service, &call);
        assert!(answer["error"].is_string(), "{answer}");
    }

    #[test]
    fn a_call_for_another_service_and_an_event_are_declined() {
        let service = UiService::new();
        assert_eq!(
            UiService::handle(
                &service,
                &Call {
                    jsonrpc: crate::rpc::VERSION.to_owned(),
                    id: Some(1),
                    method: "Storage/get".to_owned(),
                    params: serde_json::json!({}),
                }
            ),
            None
        );

        let mut event = render_call(1, r#"{"views":[]}"#);
        event.id = None;
        assert_eq!(UiService::handle(&service, &event), None);
    }

    #[test]
    fn the_handler_event_matches_the_idl_and_carries_no_id() {
        let fig = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("the repository root")
                .join("figura/tsapi.fig"),
        )
        .expect("read the IDL");
        assert!(
            fig.contains("event handlerActivated(id: string, args: any[]);"),
            "figura/tsapi.fig no longer declares handlerActivated with (id, args)"
        );

        let value: serde_json::Value =
            serde_json::from_str(&handler_activated("cb-7", &[serde_json::json!("row-1")]))
                .expect("JSON");
        assert_eq!(value["method"], "EventCore/handlerActivated");
        assert_eq!(value["params"]["id"], "cb-7");
        assert_eq!(value["params"]["args"], serde_json::json!(["row-1"]));
        assert!(
            value.get("id").is_none(),
            "an event carries no message id; one here would look like a reply: {value}"
        );
    }

    #[test]
    fn a_handler_id_from_a_rendered_tree_is_what_gets_fired() {
        // The round trip the two halves of this module exist for: a function
        // prop became a string in the worker, the string arrives in the
        // render, and firing it sends that same string back.
        let service = UiService::new();
        result_of(
            &service,
            &render_call(
                1,
                r#"{"views":[{"dirty":true,"root":{"$t":"list","children":[
                    {"$t":"list-item","title":"One","children":[
                        {"$t":"action-panel","children":[
                            {"$t":"action","title":"Open","onAction":"cb-42"}
                        ]}
                    ]}
                ]}}]}"#,
            ),
        );

        let action = service
            .top()
            .expect("a tree")
            .walk()
            .find(|node| node.tag == "action")
            .cloned()
            .expect("the action survived the render");
        let handler = action
            .prop("onAction")
            .and_then(serde_json::Value::as_str)
            .expect("the action carries a callback id");

        let event: serde_json::Value =
            serde_json::from_str(&handler_activated(handler, &[])).expect("JSON");
        assert_eq!(event["params"]["id"], "cb-42");
    }
}
