//! The shell half of `UI`, read against
//! `src/server/src/extension/api/ui-service.hpp` and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_core::alert::{Alert, SemanticColor};
use compass_worker_host::tsapi::{Call, Deferral};
use compass_worker_host::ui_shell_service::{
    CLOSE_MAIN_WINDOW_DELAY_MS, CloseWindow, CommandInfo, Notification, PopToRoot, Shell,
    ToastStyle, UiShellService, Urgency, view_popped, view_pushed,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    SetToast(String, ToastStyle, String),
    ClearToast,
    CloseWindow(CloseWindow),
    ShowHud(String),
    PopToRoot(bool),
    PushView(CommandInfo),
    PopView,
    SetSearchText(String),
    SelectedText,
    SendNotification(Notification),
    ShowAlert(Alert, Deferral),
}

#[derive(Default)]
struct Stub {
    selection: Option<Result<String, String>>,
    did: RefCell<Vec<Did>>,
}

impl Shell for Stub {
    fn set_toast(&self, title: &str, style: ToastStyle, message: &str) {
        self.did
            .borrow_mut()
            .push(Did::SetToast(title.to_owned(), style, message.to_owned()));
    }
    fn show_alert(&self, alert: &Alert, deferral: &Deferral) {
        self.did
            .borrow_mut()
            .push(Did::ShowAlert(alert.clone(), deferral.clone()));
    }
    fn clear_toast(&self) {
        self.did.borrow_mut().push(Did::ClearToast);
    }
    fn close_window(&self, options: CloseWindow) {
        self.did.borrow_mut().push(Did::CloseWindow(options));
    }
    fn show_hud(&self, text: &str) {
        self.did.borrow_mut().push(Did::ShowHud(text.to_owned()));
    }
    fn pop_to_root(&self, clear_search: bool) {
        self.did.borrow_mut().push(Did::PopToRoot(clear_search));
    }
    fn push_view(&self, command: &CommandInfo) {
        self.did.borrow_mut().push(Did::PushView(command.clone()));
    }
    fn pop_view(&self) {
        self.did.borrow_mut().push(Did::PopView);
    }
    fn set_search_text(&self, text: &str) {
        self.did
            .borrow_mut()
            .push(Did::SetSearchText(text.to_owned()));
    }
    fn selected_text(&self) -> Result<String, String> {
        self.did.borrow_mut().push(Did::SelectedText);
        self.selection.clone().unwrap_or_else(|| Ok(String::new()))
    }
    fn send_notification(&self, notification: &Notification) {
        self.did
            .borrow_mut()
            .push(Did::SendNotification(notification.clone()));
    }
}

fn command() -> CommandInfo {
    CommandInfo {
        name: "Search Stories".to_owned(),
        icon: "hn.png".to_owned(),
    }
}

fn service() -> UiShellService<Stub> {
    UiShellService::new(Stub::default(), command())
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 8, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn reply(
    service: &UiShellService<Stub>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let answer = service.handle(&call(method, params)).expect("answered");
    serde_json::from_str(&answer).expect("a JSON reply")
}

fn did(service: &UiShellService<Stub>) -> Vec<Did> {
    service.shell().did.borrow().clone()
}

#[test]
fn a_toast_reorders_its_arguments_and_drops_the_id() {
    // `setToast(title, style, message)` against the IDL's
    // `showToast(id, title, message, style)`.
    let service = service();
    let answer = reply(
        &service,
        "UI/showToast",
        serde_json::json!({
            "id": "t1", "title": "Copied", "message": "to the clipboard", "style": "Success",
        }),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert_eq!(
        did(&service),
        [Did::SetToast(
            "Copied".to_owned(),
            ToastStyle::Success,
            "to the clipboard".to_owned()
        )]
    );
}

#[test]
fn the_idls_error_style_is_the_shells_danger_and_the_rest_map_by_name() {
    for (wire, style) in [
        ("Success", ToastStyle::Success),
        ("Info", ToastStyle::Info),
        ("Warning", ToastStyle::Warning),
        ("Error", ToastStyle::Danger),
        ("Dynamic", ToastStyle::Dynamic),
        // `default: return ToastStyle::Success;`
        ("Catastrophe", ToastStyle::Success),
    ] {
        let service = service();
        reply(
            &service,
            "UI/showToast",
            serde_json::json!({"id": "t", "title": "T", "message": "M", "style": wire}),
        );
        assert_eq!(
            did(&service),
            [Did::SetToast("T".to_owned(), style, "M".to_owned())],
            "{wire}"
        );
    }
}

#[test]
fn updating_a_toast_does_nothing_at_all_because_the_cpp_does_nothing() {
    // `Void::Future updateToast(...) override { return Void::ok(); }` -- the
    // body is empty. Reproduced deliberately; see PARITY.md.
    let service = service();
    let answer = reply(
        &service,
        "UI/updateToast",
        serde_json::json!({"id": "t1", "title": "Still working"}),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert!(did(&service).is_empty(), "{:?}", did(&service));
}

#[test]
fn hiding_a_toast_ignores_the_id_and_clears_whatever_is_showing() {
    let service = service();
    reply(
        &service,
        "UI/hideToast",
        serde_json::json!({"id": "some-other-toast"}),
    );

    assert_eq!(did(&service), [Did::ClearToast]);
}

#[test]
fn the_hud_closes_the_window_first_and_then_shows() {
    // The order is the point: `closeWindow(...)` then `showHud(...)`, so the
    // HUD outlives the window it replaces.
    let service = service();
    reply(
        &service,
        "UI/showHud",
        serde_json::json!({"text": "Copied", "clear_root": true, "popToRoot": "Immediate"}),
    );

    assert_eq!(
        did(&service),
        [
            Did::CloseWindow(CloseWindow {
                pop_to_root: PopToRoot::Immediate,
                clear_root_search: true,
                delay_ms: 0,
            }),
            Did::ShowHud("Copied".to_owned()),
        ]
    );
}

#[test]
fn closing_the_main_window_asks_for_the_fifty_millisecond_delay() {
    // `closeWindow({...}, 50ms)` -- `showHud`'s call has no delay, this one
    // does, and it is the difference between the window vanishing under a
    // still-rendering view and after it.
    let service = service();
    reply(
        &service,
        "UI/closeMainWindow",
        serde_json::json!({"clearRoot": false, "popToRoot": "Suspended"}),
    );

    assert_eq!(
        did(&service),
        [Did::CloseWindow(CloseWindow {
            pop_to_root: PopToRoot::Suspended,
            clear_root_search: false,
            delay_ms: CLOSE_MAIN_WINDOW_DELAY_MS,
        })]
    );
    assert_eq!(CLOSE_MAIN_WINDOW_DELAY_MS, 50);
}

#[test]
fn an_unknown_pop_to_root_is_the_default_one() {
    // `default: return PopToRootType::Default;`
    let service = service();
    reply(
        &service,
        "UI/closeMainWindow",
        serde_json::json!({"clearRoot": false, "popToRoot": "Eventually"}),
    );

    let Did::CloseWindow(options) = did(&service)[0] else {
        panic!("expected a close");
    };
    assert_eq!(options.pop_to_root, PopToRoot::Default);
}

#[test]
fn pop_to_root_carries_the_clear_search_flag() {
    for clear in [true, false] {
        let service = service();
        reply(
            &service,
            "UI/popToRoot",
            serde_json::json!({"clearSearchBar": clear}),
        );
        assert_eq!(did(&service), [Did::PopToRoot(clear)]);
    }
}

#[test]
fn pushing_a_view_names_it_after_the_running_command() {
    // `setNavigationTitle(m_command->name())` and
    // `setNavigationIcon(m_command->iconUrl())`.
    let service = service();
    reply(&service, "UI/pushView", serde_json::json!({}));

    assert_eq!(did(&service), [Did::PushView(command())]);
}

#[test]
fn the_view_pushed_event_is_separate_from_the_reply() {
    // `QTimer::singleShot(0, ...)` -- the event goes out after the reply, so it
    // is a message the host sends, not part of the answer.
    let service = service();
    let answer = reply(&service, "UI/pushView", serde_json::json!({}));
    assert_eq!(answer["result"], serde_json::Value::Null);
    assert!(answer.get("method").is_none(), "{answer}");

    let event: serde_json::Value = serde_json::from_str(&view_pushed()).expect("JSON");
    assert_eq!(event["method"], "UI/viewPushed");
    assert!(event.get("id").is_none(), "an event has no id: {event}");

    let event: serde_json::Value = serde_json::from_str(&view_popped()).expect("JSON");
    assert_eq!(event["method"], "UI/viewPoped", "the IDL's spelling");
}

#[test]
fn popping_a_view_and_setting_the_search_text_reach_the_shell() {
    let service = service();
    reply(&service, "UI/popView", serde_json::json!({}));
    reply(
        &service,
        "UI/setSearchText",
        serde_json::json!({"text": "rust"}),
    );

    assert_eq!(
        did(&service),
        [Did::PopView, Did::SetSearchText("rust".to_owned())]
    );
}

#[test]
fn selected_text_answers_the_string_or_the_services_own_error() {
    let service = UiShellService::new(
        Stub {
            selection: Some(Ok("highlighted".to_owned())),
            ..Stub::default()
        },
        command(),
    );
    let answer = reply(&service, "UI/getSelectedText", serde_json::json!({}));
    assert_eq!(answer["result"], "highlighted");

    let service = UiShellService::new(
        Stub {
            selection: Some(Err("no selection owner on this display".to_owned())),
            ..Stub::default()
        },
        command(),
    );
    let answer = reply(&service, "UI/getSelectedText", serde_json::json!({}));
    assert_eq!(answer["error"], "no selection owner on this display");
}

#[test]
fn a_notification_maps_its_urgency_and_carries_the_icon_source() {
    for (wire, urgency) in [
        ("Low", Urgency::Low),
        ("Normal", Urgency::Normal),
        ("High", Urgency::High),
        // `default: return Urgency::Normal;`
        ("Screaming", Urgency::Normal),
    ] {
        let service = service();
        reply(
            &service,
            "UI/sendDesktopNotification",
            serde_json::json!({"data": {"title": "T", "body": "B", "urgency": wire}}),
        );
        assert_eq!(
            did(&service),
            [Did::SendNotification(Notification {
                title: "T".to_owned(),
                body: "B".to_owned(),
                icon: None,
                urgency,
            })],
            "{wire}"
        );
    }

    // `if (!data.icon)` is the fast path; with an icon the C++ renders it to a
    // temporary PNG first. The rendering is the shell's, so the source travels.
    let service = service();
    reply(
        &service,
        "UI/sendDesktopNotification",
        serde_json::json!({"data": {
            "title": "T", "body": "B", "urgency": "Normal",
            "icon": {"source": {"raw": "https://e.org/i.png"}},
        }}),
    );
    let Did::SendNotification(sent) = &did(&service)[0] else {
        panic!("expected a notification");
    };
    assert_eq!(
        sent.icon,
        Some(serde_json::json!({"source": {"raw": "https://e.org/i.png"}}))
    );
}

#[test]
fn confirm_alert_is_on_the_ledger_and_handle_still_declines_it() {
    // It suspends on a person, so it is served through `defer` and never by
    // `handle`. Both halves matter: on the ledger, because an extension cannot
    // tell from the wire that its reply came back on a later turn; declined by
    // `handle`, because an answer there would be the host deciding for them.
    assert!(compass_worker_host::tsapi::is_implemented(
        "UI/confirmAlert"
    ));
    let service = service();
    assert!(
        service
            .handle(&call("UI/confirmAlert", serde_json::json!({})))
            .is_none()
    );
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = service();
    assert!(
        service
            .handle(&call("Storage/get", serde_json::json!({})))
            .is_none()
    );
    assert!(did(&service).is_empty());
}

#[test]
fn an_event_is_not_answered_and_touches_nothing() {
    let service = service();
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "UI/hideToast",
        "params": {"id": "t"},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
    assert!(did(&service).is_empty());
}

/// The alert the stub was shown, and what it owes.
fn shown_alert(stub: &Stub) -> (Alert, Deferral) {
    stub.did
        .borrow()
        .iter()
        .find_map(|did| match did {
            Did::ShowAlert(alert, deferral) => Some((alert.clone(), deferral.clone())),
            _ => None,
        })
        .expect("an alert was shown")
}

#[test]
fn confirm_alert_is_taken_but_not_answered() {
    // The whole point of the deferral: the dialog is up and the extension is
    // still waiting. A reply now would resolve its promise before the person
    // has looked at the dialog.
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = confirm_alert_call();

    assert_eq!(
        service.handle(&call),
        None,
        "handle must not answer a call that answers later"
    );
    let deferral = UiShellService::defer(&service, &call).expect("taken");
    assert_eq!(deferral.id, 42);
    assert_eq!(deferral.method, "UI/confirmAlert");
}

#[test]
fn confirm_alert_passes_the_payloads_text_to_the_shell() {
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = confirm_alert_call();
    UiShellService::defer(&service, &call).expect("taken");

    let (alert, deferral) = shown_alert(service.shell());
    assert_eq!(alert.title, "Delete everything?");
    assert_eq!(alert.message, "This cannot be undone");
    assert_eq!(alert.confirm_text, "Delete");
    assert_eq!(alert.cancel_text, "Keep");
    assert_eq!(deferral.id, 42, "the shell needs the id to answer with");
}

#[test]
fn confirm_alert_reads_its_fields_under_payload_as_the_runtime_sends_them() {
    // `confirmAlert(payload: ConfirmAlertPayload)` names its one argument, so
    // the generated client sends {"payload": {...}}. The flat fixture above
    // hid that this read nothing from a real extension: every alert came up
    // with an empty title.
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = call_with_id(
        "UI/confirmAlert",
        7,
        serde_json::json!({ "payload": {
            "title": "Delete it?",
            "description": "Are you sure?",
            "primaryAction": { "title": "Confirm", "style": "Default" },
            "dismissAction": { "title": "Cancel", "style": "Cancel" },
            "rememberUserChoice": false,
        }}),
    );
    UiShellService::defer(&service, &call).expect("taken");
    let (alert, _) = shown_alert(service.shell());
    assert_eq!(alert.title, "Delete it?");
    assert_eq!(alert.message, "Are you sure?");
    assert_eq!(alert.confirm_text, "Confirm");
}

#[test]
fn the_primary_action_is_red_whatever_style_the_extension_asked_for() {
    // confirmAlert hardcodes SemanticColor::Red for the primary action and
    // Foreground for the dismiss one; the payload's `style` never reaches the
    // colour. An extension asking for Default does not get a safe-looking
    // button on a destructive action.
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = call_with_id(
        "UI/confirmAlert",
        42,
        serde_json::json!({
            "title": "Go on?",
            "description": "Really?",
            "primaryAction": { "title": "Yes", "style": "Default" },
            "dismissAction": { "title": "No", "style": "Cancel" },
        }),
    );
    UiShellService::defer(&service, &call).expect("taken");

    let (alert, _) = shown_alert(service.shell());
    assert_eq!(alert.confirm_color, SemanticColor::RED);
    assert_eq!(alert.cancel_color, SemanticColor::FOREGROUND);
}

#[test]
fn an_action_with_an_empty_title_falls_back_too() {
    // A button with no text on it is not a button. The TypeScript client fills
    // these in, but the host is what the wire reaches, and an extension that
    // hand-rolls the call can send "".
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = call_with_id(
        "UI/confirmAlert",
        42,
        serde_json::json!({
            "title": "Go on?",
            "description": "Really?",
            "primaryAction": { "title": "" },
            "dismissAction": { "title": "" },
        }),
    );
    UiShellService::defer(&service, &call).expect("taken");

    let (alert, _) = shown_alert(service.shell());
    assert_eq!(alert.confirm_text, "Confirm");
    assert_eq!(alert.cancel_text, "Cancel");
}

#[test]
fn missing_action_titles_fall_back_to_confirm_and_cancel() {
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = call_with_id(
        "UI/confirmAlert",
        42,
        serde_json::json!({ "title": "Go on?", "description": "Really?" }),
    );
    UiShellService::defer(&service, &call).expect("taken");

    let (alert, _) = shown_alert(service.shell());
    assert_eq!(alert.confirm_text, "Confirm");
    assert_eq!(alert.cancel_text, "Cancel");
}

#[test]
fn an_alert_with_no_icon_in_the_payload_keeps_the_default_warning() {
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = confirm_alert_call();
    UiShellService::defer(&service, &call).expect("taken");

    let (alert, _) = shown_alert(service.shell());
    let icon = alert.icon.expect("the default survives");
    assert_eq!(icon.source, "warning");
    assert_eq!(icon.fill, Some(SemanticColor::RED));
}

#[test]
fn a_builtin_icon_in_the_payload_is_tinted_and_anything_else_is_not() {
    for (source, tinted) in [("trash", true), ("file:///tmp/photo.png", false)] {
        let stub = Stub::default();
        let service = UiShellService::new(stub, CommandInfo::default());
        let call = call_with_id(
            "UI/confirmAlert",
            42,
            serde_json::json!({
                "title": "Go on?",
                "description": "Really?",
                "icon": { "source": source },
            }),
        );
        UiShellService::defer(&service, &call).expect("taken");

        let (alert, _) = shown_alert(service.shell());
        let icon = alert.icon.expect("the payload's icon");
        assert_eq!(icon.source, source);
        assert_eq!(
            icon.fill.is_some(),
            tinted,
            "{source} should{} be tinted",
            if tinted { "" } else { " not" }
        );
    }
}

#[test]
fn nothing_else_defers() {
    // Every other method answers now. A method that started deferring by
    // accident would leave its extension waiting for a person who is never
    // asked.
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    for method in ["UI/showToast", "UI/popToRoot", "UI/getSelectedText"] {
        let call = call_with_id(method, 7, serde_json::json!({}));
        assert_eq!(
            UiShellService::defer(&service, &call),
            None,
            "{method} must not defer"
        );
    }
}

#[test]
fn the_deferral_answers_with_the_calls_own_id() {
    // The id is the only thing connecting the answer to the promise. Answering
    // with any other would resolve some other call.
    let stub = Stub::default();
    let service = UiShellService::new(stub, CommandInfo::default());
    let call = confirm_alert_call();
    let deferral = UiShellService::defer(&service, &call).expect("taken");

    let value: serde_json::Value =
        serde_json::from_str(&deferral.answer(serde_json::json!(true))).expect("JSON");
    assert_eq!(value["id"], 42);
    assert_eq!(value["result"], true);
}

/// A call with an explicit id, for tests that check the id is quoted back.
fn call_with_id(method: &str, id: u64, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

/// A `UI/confirmAlert` call with every field set.
fn confirm_alert_call() -> Call {
    call_with_id(
        "UI/confirmAlert",
        42,
        serde_json::json!({
            "title": "Delete everything?",
            "description": "This cannot be undone",
            "primaryAction": { "title": "Delete", "style": "Destructive" },
            "dismissAction": { "title": "Keep", "style": "Cancel" },
        }),
    )
}
