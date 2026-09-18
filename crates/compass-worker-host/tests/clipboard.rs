//! `Clipboard/*`, read against
//! `src/server/src/extension/api/clipboard-service.hpp`, its `parseContent`,
//! and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_worker_host::clipboard_service::{
    Clipboard, ClipboardService, Content, CopyOptions, ReadContent,
};
use compass_worker_host::tsapi::Call;

/// What the backend was asked to do, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    Copy(Content, CopyOptions),
    Paste(Content),
    Clear,
    Read,
}

#[derive(Default)]
struct Stub {
    on_read: ReadContent,
    did: RefCell<Vec<Did>>,
}

impl Clipboard for Stub {
    fn copy(&self, content: Content, options: CopyOptions) {
        self.did.borrow_mut().push(Did::Copy(content, options));
    }
    fn paste(&self, content: Content) {
        self.did.borrow_mut().push(Did::Paste(content));
    }
    fn clear(&self) {
        self.did.borrow_mut().push(Did::Clear);
    }
    fn read(&self) -> ReadContent {
        self.did.borrow_mut().push(Did::Read);
        self.on_read.clone()
    }
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn answer(
    service: &ClipboardService<Stub>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let reply = service.handle(&call(method, params)).expect("answered");
    let reply: serde_json::Value = serde_json::from_str(&reply).expect("a JSON reply");
    assert_eq!(reply["id"], 3, "{reply}");
    reply["result"].clone()
}

/// The single thing the backend was asked to do.
fn only(service: &ClipboardService<Stub>) -> Did {
    let did = service.clipboard().did.borrow();
    assert_eq!(did.len(), 1, "{did:?}");
    did[0].clone()
}

#[test]
fn markup_wins_over_urls_and_text() {
    // `if (content.html) return Clipboard::Html{...}` comes first in
    // `parseContent`, and carries the plain-text alternative with it.
    let service = ClipboardService::new(Stub::default());
    assert_eq!(
        answer(
            &service,
            "Clipboard/copy",
            serde_json::json!({
                "content": {"html": "<b>x</b>", "text": "x", "urls": ["https://e.org"]},
                "options": {"concealed": false},
            }),
        ),
        serde_json::Value::Null,
        "a void method replies null"
    );

    assert_eq!(
        only(&service),
        Did::Copy(
            Content::Html {
                html: "<b>x</b>".to_owned(),
                text: Some("x".to_owned()),
            },
            CopyOptions { concealed: false },
        )
    );
}

#[test]
fn urls_win_over_text() {
    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {"urls": ["https://a.org", "https://b.org"], "text": "x"}}),
    );

    assert_eq!(
        only(&service),
        Did::Copy(
            Content::Urls(vec!["https://a.org".to_owned(), "https://b.org".to_owned()]),
            CopyOptions { concealed: false },
        )
    );
}

#[test]
fn an_empty_url_list_falls_through_to_the_text() {
    // `if (content.urls && !content.urls->empty())` -- the emptiness check is
    // the difference between copying nothing and copying the text.
    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {"urls": [], "text": "x"}}),
    );

    assert_eq!(
        only(&service),
        Did::Copy(Content::Text("x".to_owned()), CopyOptions::default())
    );
}

#[test]
fn empty_text_is_still_text_but_no_text_is_no_data() {
    // `if (content.text) return Clipboard::Text(...)` tests presence, not
    // emptiness, and falls off the end to a default-constructed `Content`.
    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {"text": ""}}),
    );
    assert_eq!(
        only(&service),
        Did::Copy(Content::Text(String::new()), CopyOptions::default())
    );

    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {}}),
    );
    assert_eq!(
        only(&service),
        Did::Copy(Content::NoData, CopyOptions::default())
    );
}

#[test]
fn concealed_is_carried_through_and_defaults_to_false() {
    // `{.concealed = options.concealed}`; `ClipboardOptions::concealed` is a
    // plain `bool` in the IDL, so an omitted one is false.
    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {"text": "secret"}, "options": {"concealed": true}}),
    );
    assert_eq!(
        only(&service),
        Did::Copy(
            Content::Text("secret".to_owned()),
            CopyOptions { concealed: true }
        )
    );

    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/copy",
        serde_json::json!({"content": {"text": "plain"}}),
    );
    assert_eq!(
        only(&service),
        Did::Copy(
            Content::Text("plain".to_owned()),
            CopyOptions { concealed: false }
        )
    );
}

#[test]
fn paste_goes_to_the_paste_path_not_the_copy_one() {
    // `m_paste.pasteContent(...)` -- a different service in the C++, because it
    // types into the focused window rather than filling a selection.
    let service = ClipboardService::new(Stub::default());
    answer(
        &service,
        "Clipboard/paste",
        serde_json::json!({"content": {"text": "typed"}}),
    );

    assert_eq!(
        only(&service),
        Did::Paste(Content::Text("typed".to_owned()))
    );
}

#[test]
fn clear_clears_and_copies_nothing() {
    // `QGuiApplication::clipboard()->clear()` -- not a copy of empty content.
    let service = ClipboardService::new(Stub::default());
    assert_eq!(
        answer(&service, "Clipboard/clear", serde_json::json!({})),
        serde_json::Value::Null
    );
    assert_eq!(only(&service), Did::Clear);
}

#[test]
fn read_content_always_has_text_and_omits_what_is_absent() {
    // `result.text = rc.text.toStdString()` is unguarded; `html` is set only
    // `if (rc.html)` and `urls` only `if (!rc.urls.empty())`.
    let service = ClipboardService::new(Stub::default());
    assert_eq!(
        answer(&service, "Clipboard/readContent", serde_json::json!({})),
        serde_json::json!({"text": ""}),
        "an empty clipboard reads as empty text, not as an empty object"
    );

    let service = ClipboardService::new(Stub {
        on_read: ReadContent {
            text: "x".to_owned(),
            html: Some("<b>x</b>".to_owned()),
            urls: vec!["https://e.org".to_owned()],
        },
        ..Stub::default()
    });
    assert_eq!(
        answer(&service, "Clipboard/readContent", serde_json::json!({})),
        serde_json::json!({"text": "x", "html": "<b>x</b>", "urls": ["https://e.org"]})
    );
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = ClipboardService::new(Stub::default());
    assert!(
        service
            .handle(&call("Storage/get", serde_json::json!({})))
            .is_none()
    );
    assert!(service.clipboard().did.borrow().is_empty());
}

#[test]
fn an_event_is_not_answered_and_does_not_touch_the_clipboard() {
    let service = ClipboardService::new(Stub::default());
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "Clipboard/clear",
        "params": {},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
    assert!(service.clipboard().did.borrow().is_empty());
}
