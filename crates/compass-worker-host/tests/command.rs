//! `Command/*`, read against
//! `src/server/src/extension/api/command-service.hpp`, its
//! `transformApiLaunchProps`, and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_worker_host::command_service::{
    CommandService, Commands, LaunchProps, NO_SUCH_COMMAND,
};
use compass_worker_host::tsapi::Call;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    Find(String, String, String),
    Activate(String, LaunchProps),
    SetSubtitle(Option<String>),
    ItemsChanged,
    OpenPreferences,
}

#[derive(Default)]
struct Stub {
    found: Option<String>,
    did: RefCell<Vec<Did>>,
}

impl Commands for Stub {
    fn find_command(&self, extension: &str, author: &str, command: &str) -> Option<String> {
        self.did.borrow_mut().push(Did::Find(
            extension.to_owned(),
            author.to_owned(),
            command.to_owned(),
        ));
        self.found.clone()
    }
    fn activate(&self, entrypoint_id: &str, props: LaunchProps) {
        self.did
            .borrow_mut()
            .push(Did::Activate(entrypoint_id.to_owned(), props));
    }
    fn set_subtitle_override(&self, subtitle: Option<&str>) {
        self.did
            .borrow_mut()
            .push(Did::SetSubtitle(subtitle.map(str::to_owned)));
    }
    fn items_changed(&self) {
        self.did.borrow_mut().push(Did::ItemsChanged);
    }
    fn open_preferences(&self) {
        self.did.borrow_mut().push(Did::OpenPreferences);
    }
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn reply(
    service: &CommandService<Stub>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let answer = service.handle(&call(method, params)).expect("answered");
    serde_json::from_str(&answer).expect("a JSON reply")
}

fn did(service: &CommandService<Stub>) -> Vec<Did> {
    service.commands().did.borrow().clone()
}

fn launch(options: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"options": options})
}

#[test]
fn launching_needs_the_extension_the_author_and_the_command_to_match() {
    // The C++ walks `rootManager->extensions()` comparing `repo->name()`,
    // `repo->author()` and `cmd->commandId()`; all three reach the backend.
    let service = CommandService::new(Stub {
        found: Some("hn.stories".to_owned()),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "Command/launchCommand",
        launch(serde_json::json!({
            "extensionName": "hacker-news",
            "ownerOrAuthorName": "vicinae",
            "name": "stories",
            "type": "User",
        })),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert_eq!(
        did(&service),
        [
            Did::Find(
                "hacker-news".to_owned(),
                "vicinae".to_owned(),
                "stories".to_owned()
            ),
            Did::Activate("hn.stories".to_owned(), LaunchProps::default()),
        ]
    );
}

#[test]
fn a_command_that_is_not_there_fails_with_the_cpp_message() {
    // `return Void::fail("No such command");`
    let service = CommandService::new(Stub::default());
    let answer = reply(
        &service,
        "Command/launchCommand",
        launch(serde_json::json!({
            "extensionName": "nope",
            "ownerOrAuthorName": "nobody",
            "name": "missing",
        })),
    );

    assert_eq!(answer["error"], NO_SUCH_COMMAND);
    assert_eq!(NO_SUCH_COMMAND, "No such command");
    assert!(
        !did(&service).iter().any(|d| matches!(d, Did::Activate(..))),
        "nothing is activated: {:?}",
        did(&service)
    );
}

#[test]
fn launch_props_carry_context_fallback_text_and_string_arguments() {
    let service = CommandService::new(Stub {
        found: Some("e.c".to_owned()),
        ..Stub::default()
    });
    reply(
        &service,
        "Command/launchCommand",
        launch(serde_json::json!({
            "extensionName": "e", "ownerOrAuthorName": "a", "name": "c",
            "context": {"from": "root"},
            "fallbackText": "typed",
            "arguments": {"query": "rust", "zone": "eu"},
        })),
    );

    assert_eq!(
        did(&service)[1],
        Did::Activate(
            "e.c".to_owned(),
            LaunchProps {
                launch_context: Some(serde_json::json!({"from": "root"})),
                fallback_text: Some("typed".to_owned()),
                arguments: vec![
                    ("query".to_owned(), "rust".to_owned()),
                    ("zone".to_owned(), "eu".to_owned()),
                ],
            }
        )
    );
}

#[test]
fn a_non_string_argument_is_dropped_rather_than_stringified() {
    // `if (v.is_string())` -- an extension that passes a number loses that
    // argument silently. Reproduced, not improved: a launcher that stringified
    // it would hand the command an argument the C++ never gives it.
    let service = CommandService::new(Stub {
        found: Some("e.c".to_owned()),
        ..Stub::default()
    });
    reply(
        &service,
        "Command/launchCommand",
        launch(serde_json::json!({
            "extensionName": "e", "ownerOrAuthorName": "a", "name": "c",
            "arguments": {"count": 3, "nested": {"x": "y"}, "kept": "yes"},
        })),
    );

    let Did::Activate(_, props) = &did(&service)[1] else {
        panic!("expected an activation: {:?}", did(&service));
    };
    assert_eq!(props.arguments, [("kept".to_owned(), "yes".to_owned())]);
}

#[test]
fn arguments_that_are_not_an_object_are_ignored_entirely() {
    // `args && args->is_object()` -- `arguments` is `any` in the IDL, so an
    // array or a string is well-formed JSON and still not usable.
    for arguments in [
        serde_json::json!(["query", "rust"]),
        serde_json::json!("query=rust"),
        serde_json::Value::Null,
    ] {
        let service = CommandService::new(Stub {
            found: Some("e.c".to_owned()),
            ..Stub::default()
        });
        reply(
            &service,
            "Command/launchCommand",
            launch(serde_json::json!({
                "extensionName": "e", "ownerOrAuthorName": "a", "name": "c",
                "arguments": arguments,
            })),
        );

        let Did::Activate(_, props) = &did(&service)[1] else {
            panic!("expected an activation");
        };
        assert!(props.arguments.is_empty(), "{arguments} yielded {props:?}");
    }
}

#[test]
fn a_subtitle_is_set_and_an_empty_one_clears_the_override() {
    // `if (payload.subtitle && !payload.subtitle->empty())` -- empty clears.
    let service = CommandService::new(Stub::default());
    reply(
        &service,
        "Command/updateCommandMetadata",
        serde_json::json!({"payload": {"subtitle": "3 unread"}}),
    );
    assert_eq!(
        did(&service),
        [
            Did::SetSubtitle(Some("3 unread".to_owned())),
            Did::ItemsChanged
        ]
    );

    for payload in [
        serde_json::json!({"subtitle": ""}),
        serde_json::json!({}),
        serde_json::json!({"subtitle": serde_json::Value::Null}),
    ] {
        let service = CommandService::new(Stub::default());
        reply(
            &service,
            "Command/updateCommandMetadata",
            serde_json::json!({"payload": payload}),
        );
        assert_eq!(
            did(&service),
            [Did::SetSubtitle(None), Did::ItemsChanged],
            "{payload} should clear the override"
        );
    }
}

#[test]
fn the_root_list_is_told_even_when_nothing_changed() {
    // `if (m_rootManager) emit itemsChanged();` sits outside the branch.
    let service = CommandService::new(Stub::default());
    reply(
        &service,
        "Command/updateCommandMetadata",
        serde_json::json!({"payload": {}}),
    );

    assert!(did(&service).contains(&Did::ItemsChanged));
}

#[test]
fn both_preference_methods_do_the_same_thing() {
    // "for now both behave the same" -- `openExtensionPreferences` is a
    // delegation to `openCommandPreferences`.
    for method in [
        "Command/openExtensionPreferences",
        "Command/openCommandPreferences",
    ] {
        let service = CommandService::new(Stub::default());
        let answer = reply(&service, method, serde_json::json!({}));
        assert_eq!(answer["result"], serde_json::Value::Null);
        assert_eq!(did(&service), [Did::OpenPreferences], "{method}");
    }
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = CommandService::new(Stub::default());
    assert!(
        service
            .handle(&call("Storage/get", serde_json::json!({})))
            .is_none()
    );
    assert!(did(&service).is_empty());
}

#[test]
fn an_event_is_not_answered_and_launches_nothing() {
    let service = CommandService::new(Stub {
        found: Some("e.c".to_owned()),
        ..Stub::default()
    });
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "Command/launchCommand",
        "params": {"options": {"extensionName": "e", "ownerOrAuthorName": "a", "name": "c"}},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
    assert!(did(&service).is_empty());
}
