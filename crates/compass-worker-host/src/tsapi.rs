//! The extension protocol that rides inside the manager's payloads.
//!
//! `figura/manager.fig` says it plainly: *"At this layer, the manager is
//! unaware what the payload is made of. The payload is in reality another rpc
//! message belonging to the vicinae<->extension spec."* That spec is
//! `figura/tsapi.fig`, and the C++ engine confirms the nesting —
//! `ExtensionCommandRuntime` routes the string out of an `extensionMessage`
//! straight into a `tsapi::Server`, and sends replies back out through
//! `messageExtension`.
//!
//! So a payload is a JSON-RPC message *serialised as a string*, carried as one
//! field of another JSON-RPC message. This module is the inner layer.
//!
//! # The error field is a string, not a JSON-RPC error object
//!
//! JSON-RPC 2.0 says `error` is an object with `code` and `message`. Both
//! existing implementations disagree, in the same direction: the C++ sends
//! `JsonRpcErrorResponse{.jsonrpc = "2.0", .id = id, .error = error}` with
//! `std::optional<std::string> error`, and the TypeScript client does
//! `handler.reject(msg.error)` on whatever arrives. A host that sent a
//! conforming error object would reject extensions with `[object Object]`.
//! [`reply_error`] therefore sends a string, and a test pins that against the
//! generator rather than against this paragraph.
//!
//! # What is not here yet
//!
//! The host APIs themselves. [`IMPLEMENTED`] is the ledger, and
//! `the_ledger_names_only_methods_the_idl_declares` keeps it honest; a method
//! not on it gets [`unimplemented()`], which is an error naming the method rather
//! than a silent hang. An extension calling one gets a rejected promise it can
//! report, which is the difference between a bug and a mystery.

use serde::{Deserialize, Serialize};

/// A message from an extension: a call, or an event it raised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Call {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Present on a call that wants an answer, absent on an event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// `"<Service>/<method>"`, the same scheme the manager protocol uses.
    pub method: String,
    /// Named arguments, keyed by parameter name.
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Call {
    /// Whether this is an event, which is answered by not answering.
    #[must_use]
    pub fn is_event(&self) -> bool {
        self.id.is_none()
    }

    /// The service half of the method name.
    #[must_use]
    pub fn service(&self) -> &str {
        self.method.split('/').next().unwrap_or(&self.method)
    }
}

/// Reads a payload carried by `Manager/extensionMessage`.
///
/// # Errors
///
/// [`serde_json::Error`] if the payload is not a JSON-RPC message. It is a
/// string that came out of a JSON field, so it has survived one round of
/// quoting already and being unparseable here means the worker built it wrong.
pub fn parse(payload: &str) -> Result<Call, serde_json::Error> {
    serde_json::from_str(payload)
}

/// A successful answer, ready to hand to `Manager/messageExtension`.
#[must_use]
pub fn reply(id: u64, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": crate::rpc::VERSION, "id": id, "result": result }).to_string()
}

/// A successful answer with no `result` member, which the generated client
/// resolves as `undefined` rather than `null`.
///
/// For the few calls whose Raycast counterpart resolves `undefined` and whose
/// callers test for it (`LocalStorage.getItem` of a missing key).
#[must_use]
pub fn reply_undefined(id: u64) -> String {
    serde_json::json!({ "jsonrpc": crate::rpc::VERSION, "id": id }).to_string()
}

/// A failed answer. `message` reaches the extension as the rejection value.
#[must_use]
pub fn reply_error(id: u64, message: &str) -> String {
    serde_json::json!({ "jsonrpc": crate::rpc::VERSION, "id": id, "error": message }).to_string()
}

/// An event, which the extension's transport delivers to its subscribers.
#[must_use]
pub fn event(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": crate::rpc::VERSION, "method": method, "params": params })
        .to_string()
}

/// The answer to a call this host does not serve yet.
///
/// Naming the method matters: the extension's own stack trace stops at the
/// generated client, so this string is the only thing that says which API was
/// missing.
#[must_use]
pub fn unimplemented(id: u64, method: &str) -> String {
    reply_error(
        id,
        &format!("{method} is not implemented by this host (compass-worker-host)"),
    )
}

/// The `tsapi` methods this host serves.
///
/// Every entry must name a method `figura/tsapi.fig` declares, which is what
/// stops this from becoming a list of aspirations, and every entry must be
/// claimed by a service, which is what stops it from becoming a list of
/// promises.
pub const IMPLEMENTED: &[&str] = &[
    "Storage/get",
    "Storage/set",
    "Storage/remove",
    "Storage/clear",
    "Storage/list",
    "OAuth/getTokens",
    "OAuth/setTokens",
    "OAuth/removeTokens",
    "OAuth/authorize",
    "UI/render",
    "UI/showToast",
    "UI/updateToast",
    "UI/hideToast",
    "UI/showHud",
    "UI/closeMainWindow",
    "UI/popToRoot",
    "UI/pushView",
    "UI/popView",
    "UI/setSearchText",
    "UI/getSelectedText",
    // Answered later, through `Deferral`, not by a service returning a value.
    "UI/confirmAlert",
    "UI/sendDesktopNotification",
    "FileSearch/search",
    "Application/list",
    "Application/open",
    "Application/getDefault",
    "Application/showInFileBrowser",
    "Application/runInTerminal",
    "Command/launchCommand",
    "Command/updateCommandMetadata",
    "Command/openExtensionPreferences",
    "Command/openCommandPreferences",
    "WindowManagement/focusWindow",
    "WindowManagement/getActiveWindow",
    "WindowManagement/getActiveWorkspace",
    "WindowManagement/getWindows",
    "WindowManagement/getScreens",
    "WindowManagement/getWorkspaces",
    "WindowManagement/setWindowBounds",
    "Wallpaper/set",
    "BrowserExtension/getTabs",
    "BrowserExtension/focusTab",
    "Clipboard/copy",
    "Clipboard/paste",
    "Clipboard/clear",
    "Clipboard/readContent",
    // Answered once a person has allowed it, through a `Broker`.
    "HostCommand/run",
];

/// Something that answers some of the extension API.
///
/// One method, deliberately: a service is asked about a call and either
/// answers it or declines. Declining rather than erroring is what lets the
/// host hold several services and ask each in turn — the first one asked would
/// otherwise refuse every call in the system.
pub trait Service {
    /// The answer, as a payload for `Manager/messageExtension`, or `None` if
    /// this call is not this service's.
    fn handle(&self, call: &Call) -> Option<String>;

    /// Whether this service takes `call` but cannot answer it yet.
    ///
    /// Most of the API is a question a backend can answer while the extension
    /// waits. A few are not: `UI/confirmAlert` answers when a *person* presses
    /// a button, and `OAuth/authorize` when they finish with a browser. A
    /// service returning a [`Deferral`] here is saying "this is mine, and the
    /// reply comes later" — the host sends nothing now and remembers what to
    /// quote back when the answer arrives.
    ///
    /// The default is `None`, so a service that answers everything it takes
    /// does not have to know this exists.
    fn defer(&self, _call: &Call) -> Option<Deferral> {
        None
    }
}

/// A call whose reply is owed but not yet known.
///
/// It carries the JSON-RPC id because that is the only thing that connects the
/// eventual answer to the promise the extension is waiting on. Losing it means
/// an extension that waits for ever; guessing it means resolving some *other*
/// promise with this answer, which is worse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferral {
    /// The id to quote back.
    pub id: u64,
    /// Which method is waiting, for whoever has to report on it.
    pub method: String,
}

impl Deferral {
    /// The deferral for `call`, or `None` if it carries no id.
    ///
    /// An event has no id and so cannot be deferred: there is nothing to
    /// answer.
    #[must_use]
    pub fn for_call(call: &Call) -> Option<Self> {
        Some(Self {
            id: call.id?,
            method: call.method.clone(),
        })
    }

    /// The payload that answers this deferral with `value`.
    #[must_use]
    pub fn answer(&self, value: serde_json::Value) -> String {
        reply(self.id, value)
    }

    /// The payload that fails this deferral with `message`.
    #[must_use]
    pub fn fail(&self, message: &str) -> String {
        reply_error(self.id, message)
    }
}

/// Whether [`IMPLEMENTED`] names `method`.
#[must_use]
pub fn is_implemented(method: &str) -> bool {
    IMPLEMENTED.contains(&method)
}

#[cfg(test)]
mod deferral_tests {
    use super::*;

    /// A call, or an event when `id` is `None`.
    fn call(method: &str, id: Option<u64>) -> Call {
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": {},
        }))
        .expect("a well-formed message")
    }

    #[test]
    fn an_event_cannot_be_deferred() {
        // Tested directly rather than through the router, which screens events
        // out before any service sees them: this is the contract of the
        // function itself, and a deferral over an event would later answer
        // with an id nobody asked on.
        assert_eq!(Deferral::for_call(&call("UI/viewPoped", None)), None);
    }

    #[test]
    fn a_call_defers_under_its_own_id() {
        let deferral = Deferral::for_call(&call("UI/confirmAlert", Some(9))).expect("deferred");
        assert_eq!(deferral.id, 9);
        assert_eq!(deferral.method, "UI/confirmAlert");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    const FIG: &str = "figura/tsapi.fig";
    const TS: &str = "crates/compass-figura/src/typescript/server-boilerplate.ts.in";
    const CLIENT: &str = "crates/compass-figura/src/typescript/client-bus.ts.in";

    fn read(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/compass-worker-host sits two levels below the repository root")
            .join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Every `"<Service>/<method>"` the IDL declares, functions and events.
    fn declared_methods() -> BTreeSet<String> {
        let fig = read(FIG);
        let mut out = BTreeSet::new();
        let mut service: Option<String> = None;

        for line in fig.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("service ") {
                service = rest.split_whitespace().next().map(ToOwned::to_owned);
                continue;
            }
            if line.starts_with('}') {
                service = None;
                continue;
            }
            let Some(name) = service.as_deref() else {
                continue;
            };
            let Some(rest) = line
                .strip_prefix("fn ")
                .or_else(|| line.strip_prefix("event "))
            else {
                continue;
            };
            if let Some(method) = rest.split('(').next() {
                out.insert(format!("{name}/{}", method.trim()));
            }
        }
        out
    }

    #[test]
    fn the_idl_parses_into_something_worth_checking_against() {
        // Every test below is vacuous if this parser silently returns nothing,
        // and a ledger checked against an empty set would pass for ever.
        let declared = declared_methods();
        // 49 today, counted independently with
        // `grep -c "^\s*fn \|^\s*event " figura/tsapi.fig`. The floor is
        // below that so adding an API does not fail here, and far above what a
        // broken parser returns.
        assert!(
            declared.len() >= 45,
            "parsed only {} methods from {FIG}: {declared:?}",
            declared.len()
        );

        // Every service, not just enough methods overall: a parser that lost
        // track of the service name would still clear the floor above while
        // attributing methods to the wrong one.
        let services: BTreeSet<&str> = declared
            .iter()
            .filter_map(|m| m.split('/').next())
            .collect();
        assert_eq!(
            services.len(),
            read(FIG).matches("service ").count(),
            "found methods for {services:?}, which is not every service {FIG} declares"
        );
        for expected in [
            "UI/render",
            "Storage/get",
            "Clipboard/copy",
            "OAuth/authorize",
            "EventCore/handlerActivated",
        ] {
            assert!(
                declared.contains(expected),
                "{FIG} declares {expected} and the parser did not find it"
            );
        }
    }

    #[test]
    fn the_ledger_names_only_methods_the_idl_declares() {
        let declared = declared_methods();
        for method in IMPLEMENTED {
            assert!(
                declared.contains(*method),
                "{method} is on the implemented ledger but {FIG} does not declare it"
            );
            assert!(is_implemented(method));
        }
        // The count is reported rather than asserted: it is a roadmap figure,
        // and a test that pinned it would fail on every step forward.
        println!(
            "tsapi coverage: {}/{} methods",
            IMPLEMENTED.len(),
            declared.len()
        );
    }

    #[test]
    fn the_ledger_is_exactly_what_the_services_claim() {
        // Checked from both ends. An entry no service serves is a promise the
        // host does not keep -- an extension would be told the method exists
        // and then get nothing back.
        let mut claimed: Vec<&str> = crate::storage_service::METHODS
            .iter()
            .chain(crate::oauth_service::METHODS)
            .chain(crate::ui_service::METHODS)
            .chain(crate::file_search_service::METHODS)
            .chain(crate::clipboard_service::METHODS)
            .chain(crate::application_service::METHODS)
            .chain(crate::command_service::METHODS)
            .chain(crate::window_service::METHODS)
            .chain(crate::wallpaper_service::METHODS)
            .chain(crate::browser_service::METHODS)
            .chain(crate::ui_shell_service::METHODS)
            // A method that answers later is still a method the host serves,
            // so the ledger has to count it -- an extension cannot tell from
            // the wire whether its reply came back on the same turn.
            .chain(crate::ui_shell_service::DEFERRED_METHODS)
            .chain(crate::oauth_service::DEFERRED_METHODS)
            .chain(crate::host_command_service::METHODS)
            .chain(crate::host_command_service::DEFERRED_METHODS)
            .copied()
            .collect();
        let mut ledger: Vec<&str> = IMPLEMENTED.to_vec();
        claimed.sort_unstable();
        ledger.sort_unstable();
        assert_eq!(ledger, claimed);
    }

    #[test]
    fn a_call_parses_the_way_the_generated_client_sends_it() {
        let call = parse(r#"{"jsonrpc":"2.0","id":7,"method":"UI/render","params":{"json":"{}"}}"#)
            .expect("a well-formed call");
        assert_eq!(call.id, Some(7));
        assert_eq!(call.method, "UI/render");
        assert_eq!(call.service(), "UI");
        assert!(!call.is_event());
        assert_eq!(call.params, serde_json::json!({ "json": "{}" }));
    }

    #[test]
    fn a_call_without_an_id_is_an_event() {
        // The generated transport emits events with no id and expects no
        // answer. Answering one would leave a reply with an id no client is
        // waiting on, which the TS transport drops silently.
        let call = parse(
            r#"{"jsonrpc":"2.0","method":"EventCore/handlerActivated","params":{"id":"h","args":[]}}"#,
        )
        .expect("a well-formed event");
        assert!(call.is_event());
        assert_eq!(call.id, None);
    }

    #[test]
    fn a_payload_that_is_not_json_rpc_is_an_error_not_a_default() {
        assert!(parse("").is_err(), "an empty payload is not a call");
        assert!(
            parse(r#"{"jsonrpc":"2.0","id":1}"#).is_err(),
            "a message with no method is not a call"
        );
    }

    #[test]
    fn a_reply_has_the_shape_the_generated_transport_sends() {
        // `reply(id, result) { this.sendMessage({ jsonrpc: '2.0', id, result }); }`
        let ts = read(TS);
        assert!(
            ts.contains("this.sendMessage({ jsonrpc: '2.0', id, result });"),
            "{TS} no longer replies with jsonrpc/id/result"
        );

        let value: serde_json::Value =
            serde_json::from_str(&reply(1, serde_json::json!({ "ok": true })))
                .expect("a reply is JSON");
        let keys: BTreeSet<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, BTreeSet::from(["jsonrpc", "id", "result"]));
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
    }

    #[test]
    fn an_error_reply_carries_a_bare_string_as_both_engines_do() {
        // The C++ engine sent `error` as a `std::optional<std::string>`, and
        // the generated client rejects the call's promise with that field
        // as-is, so what the extension catches is whatever `error` holds.
        let client = read(CLIENT);
        assert!(
            client.contains("if (msg.error) handler.reject(msg.error);"),
            "{CLIENT} no longer rejects with the bare `error` field"
        );

        let value: serde_json::Value =
            serde_json::from_str(&reply_error(2, "no")).expect("an error reply is JSON");
        assert!(
            value["error"].is_string(),
            "a conforming JSON-RPC error object would reach the extension as `[object Object]`: \
             {value}"
        );
        assert_eq!(value["error"], "no");
        assert_eq!(value["id"], 2);
        assert!(
            value.get("result").is_none(),
            "an error reply carries no result"
        );
    }

    #[test]
    fn an_unimplemented_call_is_refused_by_name() {
        // Picked from the IDL rather than written down: every method named here
        // eventually gets implemented, and a test that then silently checks an
        // implemented one proves nothing.
        let declared = declared_methods();
        let method = declared
            .iter()
            .find(|method| !is_implemented(method))
            .expect("some method is still unimplemented");

        let refusal: serde_json::Value =
            serde_json::from_str(&unimplemented(3, method)).expect("JSON");
        let message = refusal["error"].as_str().expect("a string error");
        assert!(
            message.contains(method),
            "the refusal must name the method; the extension's stack trace stops at the \
             generated client: {message}"
        );
    }

    #[test]
    fn an_event_carries_no_id_so_no_client_waits_on_it() {
        let value: serde_json::Value =
            serde_json::from_str(&event("UI/viewPoped", serde_json::json!({})))
                .expect("an event is JSON");
        assert!(
            value.get("id").is_none(),
            "the transport routes by `msg.id === undefined && msg.method`; an id here would \
             make the event look like a reply to a request nobody sent: {value}"
        );
        assert_eq!(value["method"], "UI/viewPoped");
    }

    #[test]
    fn a_payload_survives_being_carried_inside_a_manager_message() {
        // The nesting is the part that is easy to get wrong: the payload is a
        // JSON *string* inside a JSON object, so it is quoted twice on the
        // wire. Building the outer message by concatenation rather than by
        // serialising is how that breaks.
        let inner = reply(9, serde_json::json!({ "text": "a \"quoted\" value\n" }));
        let outer = serde_json::json!({
            "session_id": "s",
            "payload": inner,
        });
        let encoded = serde_json::to_string(&outer).expect("the outer message serialises");

        let decoded: serde_json::Value = serde_json::from_str(&encoded).expect("it round trips");
        let carried = decoded["payload"].as_str().expect("payload is a string");
        let call: serde_json::Value = serde_json::from_str(carried).expect("the inner is JSON");
        assert_eq!(call["result"]["text"], "a \"quoted\" value\n");
    }
}
