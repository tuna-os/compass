//! Frame-level tests: exhaustive variant round-trips, partial frames, and the
//! oversized-frame guard.

use compass_ipc::codec::{FrameCodec, LENGTH_PREFIX_LEN, MAX_FRAME_LEN};
use compass_ipc::{
    ClipboardEntry, ClipboardKind, DoctorCheck, DoctorStatus, Error, ErrorKind, PROTOCOL_VERSION,
    ProtocolError, QueryHit, Request, RequestEnvelope, Response, ResponseEnvelope, WindowCommand,
    WindowOutcome,
};
use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Every `Request` variant. Kept exhaustive by the `match` in
/// [`request_variants_are_exhaustive`].
fn all_requests() -> Vec<Request> {
    vec![
        Request::Ping,
        Request::Toggle,
        Request::Show,
        Request::Hide,
        Request::Query {
            text: String::new(),
        },
        Request::Query {
            text: "firefox --private".into(),
        },
        Request::Query {
            text: "unicode: é 漢字 🚀".into(),
        },
        Request::Doctor,
        Request::Shutdown,
        Request::AttachWindow,
        Request::WindowOutcome(WindowOutcome::Shown),
        Request::WindowOutcome(WindowOutcome::Hidden),
        Request::WindowOutcome(WindowOutcome::Failed(String::new())),
        Request::WindowOutcome(WindowOutcome::Failed("no compositor: é 🚀".into())),
        Request::RecordLaunch {
            key: "app.desktop".into(),
        },
        Request::RecordLaunch { key: String::new() },
        Request::ClipboardHistory {
            query: String::new(),
            limit: 50,
        },
        Request::ClipboardHistory {
            query: "https://é.example 🚀".into(),
            limit: u32::MAX,
        },
        Request::ClipboardContent { id: "abc".into() },
        Request::ClipboardContent { id: String::new() },
        Request::ListWindows,
        Request::ActivateWindow { id: 0 },
        Request::ActivateWindow { id: u32::MAX },
        Request::CloseWindow { id: 7 },
        Request::ClipboardPaste { id: "abc".into() },
        Request::ClipboardSetPinned {
            id: "abc".into(),
            pinned: true,
        },
        Request::ClipboardSetPinned {
            id: String::new(),
            pinned: false,
        },
        Request::ClipboardRemove { id: "abc".into() },
        Request::RunPowerCommand {
            id: "reboot".into(),
        },
        Request::RunMediaCommand {
            id: "play-pause".into(),
        },
        Request::RunExtensionCommand {
            id: "@raycast/github:search-repositories".into(),
            arguments_json: Some(r#"{"query":"compass"}"#.into()),
        },
        Request::ExtensionView {
            session: 1,
            after: 0,
        },
        Request::ExtensionView {
            session: u64::MAX,
            after: u64::MAX,
        },
        Request::ExtensionEvent {
            session: 1,
            handler: "cb-7".into(),
            args_json: "[\"é 🚀\", 3]".into(),
        },
        Request::ExtensionPop { session: 1 },
        Request::SetExtensionPreferences {
            id: "@raycast/github:search".into(),
            values_json: "{\"token\":\"é 🚀\"}".into(),
        },
        Request::ExtensionAlertAnswer {
            session: 1,
            confirmed: true,
        },
        Request::CloseExtension { session: 1 },
        Request::SearchFiles {
            query: "rapport é 🚀".into(),
            category: Some("Documents".into()),
        },
        Request::SearchFiles {
            query: String::new(),
            category: None,
        },
        Request::OpenFile {
            path: "/home/me/Documents/rapport é.pdf".into(),
            reveal: true,
        },
        Request::OAuthRedirect {
            url: "raycast://oauth?package_name=Extension&code=é&state=s".into(),
        },
        Request::ListShortcuts,
        Request::SaveShortcut {
            id: None,
            name: "Recherche 🚀".into(),
            icon: "default".into(),
            url: "https://x.test/?q={query}".into(),
            app: "default".into(),
        },
        Request::SaveShortcut {
            id: Some("sct-0123456789ab".into()),
            name: String::new(),
            icon: "icon://builtin/link".into(),
            url: "{clipboard}".into(),
            app: "firefox.desktop".into(),
        },
        Request::RemoveShortcut {
            id: "sct-0123456789ab".into(),
        },
        Request::OpenShortcut {
            id: "sct-0123456789ab".into(),
            arguments: vec!["é".into(), String::new()],
        },
        Request::ExpandShortcut {
            id: "sct-0123456789ab".into(),
            arguments: vec![],
        },
        Request::ListSnippets,
        Request::SaveSnippet {
            id: None,
            name: "Signature ✍".into(),
            text: "Best,\n{cursor}".into(),
            keyword: Some(";sig".into()),
            word: true,
            apps: vec!["org.gnome.TextEditor.desktop".into()],
        },
        Request::SaveSnippet {
            id: Some("snp-0123456789ab".into()),
            name: "Address".into(),
            text: "1 Rue de l'Église".into(),
            keyword: None,
            word: false,
            apps: vec![],
        },
        Request::RemoveSnippet {
            id: "snp-0123456789ab".into(),
        },
        Request::ExpandSnippet {
            id: "snp-0123456789ab".into(),
            arguments: vec![("name".into(), "Zoë".into())],
        },
        Request::PasteSnippet {
            id: "snp-0123456789ab".into(),
            arguments: vec![],
        },
    ]
}

/// Every `Response` variant.
fn all_responses() -> Vec<Response> {
    vec![
        Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: 1,
        },
        Response::Pong {
            protocol_version: u16::MAX,
            pid: u32::MAX,
        },
        Response::Ack,
        Response::QueryResults { hits: vec![] },
        Response::QueryResults {
            hits: vec![
                QueryHit {
                    id: "app:firefox.desktop".into(),
                    title: "Firefox".into(),
                    subtitle: Some("Web Browser".into()),
                    score: 100,
                },
                QueryHit {
                    id: "x".into(),
                    title: "y".into(),
                    subtitle: None,
                    score: 0,
                },
            ],
        },
        Response::DoctorReport { checks: vec![] },
        Response::DoctorReport {
            checks: vec![
                DoctorCheck {
                    name: "portal.global-shortcuts".into(),
                    status: DoctorStatus::Ok,
                    detail: None,
                },
                DoctorCheck {
                    name: "shell-extension".into(),
                    status: DoctorStatus::Warn,
                    detail: Some("not installed; window management degraded".into()),
                },
                DoctorCheck {
                    name: "ipc.socket".into(),
                    status: DoctorStatus::Fail,
                    detail: Some("permission denied".into()),
                },
            ],
        },
        Response::ShuttingDown,
        Response::Error(ProtocolError::new(ErrorKind::VersionMismatch, "v2 vs v1")),
        Response::Error(ProtocolError::new(ErrorKind::Unsupported, "")),
        Response::Error(ProtocolError::new(ErrorKind::BadRequest, "empty query")),
        Response::Error(ProtocolError::new(ErrorKind::Internal, "handler panicked")),
        Response::WindowAttached,
        Response::Window(WindowCommand::Show),
        Response::Window(WindowCommand::Hide),
        Response::Window(WindowCommand::Toggle),
        Response::ClipboardHistory { entries: vec![] },
        Response::ClipboardHistory {
            entries: vec![
                ClipboardEntry {
                    id: "a".into(),
                    preview: "hello é 🚀".into(),
                    mime_type: "text/plain;charset=utf-8".into(),
                    kind: ClipboardKind::Text,
                    pinned: true,
                    updated_at: i64::MAX,
                    url_host: None,
                },
                ClipboardEntry {
                    id: String::new(),
                    preview: "Image".into(),
                    mime_type: "image/png".into(),
                    kind: ClipboardKind::Image,
                    pinned: false,
                    updated_at: 0,
                    url_host: Some("example.org".into()),
                },
            ],
        },
        Response::ClipboardContent {
            mime_type: "text/plain".into(),
            data: "é 🚀".into(),
        },
        Response::ClipboardContent {
            mime_type: "image/png".into(),
            data: vec![0, 255, 0x89, b'P', b'N', b'G'],
        },
        Response::Windows { windows: vec![] },
        Response::Windows {
            windows: vec![compass_ipc::WindowInfo {
                id: u32::MAX,
                title: "Title é 🚀".into(),
                wm_class: "org.gnome.Nautilus".into(),
                app_name: Some("Files".into()),
                app_icon: None,
                pid: Some(1),
                workspace: Some(-1),
                focused: true,
                can_close: false,
            }],
        },
        Response::ExtensionStarted { session: 7 },
        Response::ExtensionNeedsArguments {
            title: "Search Repositories".into(),
            fields: vec![compass_ipc::PreferenceField {
                name: "query".into(),
                title: "Query".into(),
                description: String::new(),
                placeholder: "Query".into(),
                required: true,
                kind: compass_ipc::PreferenceFieldKind::Text,
                value_json: None,
            }],
        },
        Response::ExtensionNeedsPreferences {
            title: "Search Repositories".into(),
            fields: vec![
                compass_ipc::PreferenceField {
                    name: "token".into(),
                    title: "Token".into(),
                    description: String::new(),
                    placeholder: "ghp_…".into(),
                    required: true,
                    kind: compass_ipc::PreferenceFieldKind::Password,
                    value_json: None,
                },
                compass_ipc::PreferenceField {
                    name: "sort".into(),
                    title: "Sort".into(),
                    description: "Order".into(),
                    placeholder: String::new(),
                    required: false,
                    kind: compass_ipc::PreferenceFieldKind::Dropdown {
                        options: vec![("Stars".into(), "stars".into())],
                    },
                    value_json: Some("\"stars\"".into()),
                },
            ],
        },
        Response::ExtensionView {
            version: 3,
            view_json: Some("{\"kind\":\"list\"}".into()),
            problem: None,
            ended: false,
            depth: 2,
            alert: Some(compass_ipc::ExtensionAlert {
                title: "Delete é 🚀?".into(),
                message: String::new(),
                confirm_text: "Delete".into(),
                cancel_text: "Cancel".into(),
            }),
            toast: Some(compass_ipc::ExtensionToast {
                title: "Copied".into(),
                message: "to the clipboard".into(),
                style: compass_ipc::ExtensionToastStyle::Animated,
            }),
        },
        Response::ExtensionView {
            version: u64::MAX,
            view_json: None,
            problem: Some("Compass cannot draw the extension component <grid> yet".into()),
            ended: true,
            depth: 0,
            alert: None,
            toast: None,
        },
        Response::Files {
            heading: "Results".into(),
            files: vec![compass_ipc::FileHit {
                path: "/home/me/Documents/rapport é.pdf".into(),
                name: "rapport é.pdf".into(),
                category: "Documents".into(),
            }],
        },
        Response::Files {
            heading: "Recently Accessed".into(),
            files: vec![],
        },
        Response::Shortcuts {
            shortcuts: vec![compass_ipc::ShortcutEntry {
                id: "sct-0123456789ab".into(),
                name: "Recherche 🚀".into(),
                icon: "icon://favicon/x.test".into(),
                url: "https://x.test/?q={query}".into(),
                app: "default".into(),
                open_count: 3,
                created_at: 1_700_000_000,
                updated_at: 1_700_000_100,
                last_used_at: Some(1_700_000_200),
            }],
        },
        Response::Shortcuts { shortcuts: vec![] },
        Response::Text {
            text: "https://x.test/?q=é".into(),
        },
        Response::Snippets {
            snippets: vec![
                compass_ipc::SnippetEntry {
                    id: "snp-0123456789ab".into(),
                    name: "Signature ✍".into(),
                    text: Some("Best,\n{cursor}".into()),
                    file: None,
                    created_at: 1_700_000_000,
                    updated_at: Some(1_700_000_001),
                    keyword: Some(";sig".into()),
                    word: true,
                    apps: vec![],
                },
                compass_ipc::SnippetEntry {
                    id: "snp-ba9876543210".into(),
                    name: "Logo".into(),
                    text: None,
                    file: Some("/home/me/logo.png".into()),
                    created_at: 1_700_000_000,
                    updated_at: None,
                    keyword: None,
                    word: false,
                    apps: vec!["gimp.desktop".into()],
                },
            ],
        },
        Response::Snippets { snippets: vec![] },
    ]
}

#[test]
fn request_variants_are_exhaustive() {
    // If a variant is added to `Request`, this stops compiling and whoever adds
    // it has to extend `all_requests`.
    for request in all_requests() {
        match request {
            Request::Ping
            | Request::Toggle
            | Request::Show
            | Request::Hide
            | Request::Query { .. }
            | Request::Doctor
            | Request::Shutdown
            | Request::AttachWindow
            | Request::RecordLaunch { .. }
            | Request::ClipboardHistory { .. }
            | Request::ClipboardContent { .. }
            | Request::ListWindows
            | Request::ActivateWindow { .. }
            | Request::CloseWindow { .. }
            | Request::ClipboardPaste { .. }
            | Request::ClipboardSetPinned { .. }
            | Request::ClipboardRemove { .. }
            | Request::RunExtensionCommand { .. }
            | Request::RunPowerCommand { .. }
            | Request::RunMediaCommand { .. }
            | Request::ExtensionView { .. }
            | Request::ExtensionEvent { .. }
            | Request::ExtensionPop { .. }
            | Request::SetExtensionPreferences { .. }
            | Request::ExtensionAlertAnswer { .. }
            | Request::CloseExtension { .. }
            | Request::SearchFiles { .. }
            | Request::OpenFile { .. }
            | Request::OAuthRedirect { .. }
            | Request::ListShortcuts
            | Request::SaveShortcut { .. }
            | Request::RemoveShortcut { .. }
            | Request::OpenShortcut { .. }
            | Request::ExpandShortcut { .. }
            | Request::ListSnippets
            | Request::SaveSnippet { .. }
            | Request::RemoveSnippet { .. }
            | Request::ExpandSnippet { .. }
            | Request::PasteSnippet { .. }
            | Request::WindowOutcome(_) => {}
        }
    }
}

#[test]
fn response_variants_are_exhaustive() {
    for response in all_responses() {
        match response {
            Response::Pong { .. }
            | Response::Ack
            | Response::QueryResults { .. }
            | Response::DoctorReport { .. }
            | Response::ShuttingDown
            | Response::Error(_)
            | Response::WindowAttached
            | Response::ClipboardHistory { .. }
            | Response::ClipboardContent { .. }
            | Response::Windows { .. }
            | Response::ExtensionStarted { .. }
            | Response::ExtensionNeedsPreferences { .. }
            | Response::ExtensionNeedsArguments { .. }
            | Response::ExtensionView { .. }
            | Response::Files { .. }
            | Response::Shortcuts { .. }
            | Response::Text { .. }
            | Response::Snippets { .. }
            | Response::Window(_) => {}
        }
    }
}

#[test]
fn every_request_variant_round_trips() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    for (id, request) in all_requests().into_iter().enumerate() {
        let envelope = RequestEnvelope::new(id as u64, request);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap();
        assert_eq!(decoded, Some(envelope));
        assert!(buf.is_empty(), "codec left trailing bytes");
    }
}

#[test]
fn every_response_variant_round_trips() {
    let mut codec = FrameCodec::<ResponseEnvelope>::new();

    for (id, response) in all_responses().into_iter().enumerate() {
        let envelope = ResponseEnvelope::new(id as u64, response);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        assert_eq!(codec.decode(&mut buf).unwrap(), Some(envelope));
        assert!(buf.is_empty());
    }
}

#[test]
fn all_variants_round_trip_back_to_back_in_one_stream() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();

    let envelopes: Vec<_> = all_requests()
        .into_iter()
        .enumerate()
        .map(|(i, r)| RequestEnvelope::new(i as u64, r))
        .collect();

    for envelope in &envelopes {
        codec.encode(envelope, &mut buf).unwrap();
    }

    for expected in &envelopes {
        assert_eq!(codec.decode(&mut buf).unwrap().as_ref(), Some(expected));
    }
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn a_frame_fed_one_byte_at_a_time_yields_exactly_one_message_at_the_end() {
    let envelope = RequestEnvelope::new(
        42,
        Request::Query {
            text: "a moderately long query".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();
    let wire = wire.freeze();
    assert!(wire.len() > LENGTH_PREFIX_LEN);

    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();

    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        let decoded = codec.decode(&mut buf).unwrap();

        if index + 1 < wire.len() {
            assert!(
                decoded.is_none(),
                "decoder produced a message after only {} bytes",
                index + 1
            );
        }
        if let Some(item) = decoded {
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![envelope]);
    assert!(buf.is_empty());
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn two_frames_fed_one_byte_at_a_time_yield_two_messages_in_order() {
    let a = RequestEnvelope::new(1, Request::Toggle);
    let b = RequestEnvelope::new(
        2,
        Request::Query {
            text: "second".into(),
        },
    );

    let mut wire = BytesMut::new();
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    codec.encode(&a, &mut wire).unwrap();
    let boundary = wire.len();
    codec.encode(&b, &mut wire).unwrap();
    let wire = wire.freeze();

    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();
    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        if let Some(item) = codec.decode(&mut buf).unwrap() {
            // The only two byte offsets that may complete a message are the
            // last byte of each frame.
            assert!(index + 1 == boundary || index + 1 == wire.len());
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![a, b]);
}

#[test]
fn a_truncated_frame_never_yields_a_message() {
    let envelope = RequestEnvelope::new(
        7,
        Request::Query {
            text: "truncated".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();

    // Drop the final byte: the decoder must wait forever rather than guess.
    let truncated = &wire[..wire.len() - 1];
    let mut buf = BytesMut::from(truncated);
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    // Nothing was lost while waiting: the missing byte completes the frame.
    buf.put_u8(wire[wire.len() - 1]);
    assert_eq!(codec.decode(&mut buf).unwrap(), Some(envelope));
}

#[test]
fn an_oversized_length_prefix_is_rejected_without_allocating() {
    for announced in [MAX_FRAME_LEN as u32 + 1, u32::MAX, u32::MAX - 1, 1 << 30] {
        let mut buf = BytesMut::with_capacity(LENGTH_PREFIX_LEN);
        buf.put_u32_le(announced);
        let capacity_before = buf.capacity();

        let err = FrameCodec::<RequestEnvelope>::new()
            .decode(&mut buf)
            .unwrap_err();

        match err {
            Error::FrameTooLarge { len, max } => {
                assert_eq!(len, announced as usize);
                assert_eq!(max, MAX_FRAME_LEN);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }

        // The whole point: four bytes from a peer must not turn into a
        // multi-gigabyte reservation.
        assert_eq!(buf.capacity(), capacity_before);
        assert!(buf.capacity() <= LENGTH_PREFIX_LEN * 2);
    }
}

#[test]
fn the_size_limit_is_configurable_and_enforced_on_both_sides() {
    let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(32);
    assert_eq!(codec.max_frame_len(), 32);

    let big = RequestEnvelope::new(
        1,
        Request::Query {
            text: "z".repeat(1000),
        },
    );
    let mut buf = BytesMut::new();
    assert!(matches!(
        codec.encode(&big, &mut buf).unwrap_err(),
        Error::FrameTooLarge { max: 32, .. }
    ));

    let mut wire = BytesMut::new();
    wire.put_u32_le(33);
    assert!(matches!(
        codec.decode(&mut wire).unwrap_err(),
        Error::FrameTooLarge { len: 33, max: 32 }
    ));
}

#[test]
fn a_well_framed_but_nonsense_body_is_an_error_not_a_panic() {
    for body in [vec![0xff, 0xff, 0xff, 0xff], vec![0x00], vec![0x7f; 12]] {
        let mut buf = BytesMut::new();
        buf.put_u32_le(body.len() as u32);
        buf.put_slice(&body);

        // Either it decodes to something (postcard is not self-describing, so
        // some byte strings are valid) or it errors. It must never panic and it
        // must always consume the frame.
        let result = FrameCodec::<RequestEnvelope>::new().decode(&mut buf);
        assert!(buf.is_empty(), "the frame body should have been consumed");
        let _ = result;
    }
}

#[test]
fn error_messages_are_legible() {
    let err = Error::FrameTooLarge {
        len: 5_000_000,
        max: MAX_FRAME_LEN,
    };
    assert_eq!(
        err.to_string(),
        "frame of 5000000 bytes exceeds the 1048576 byte limit"
    );

    let err = Error::VersionMismatch {
        expected: 1,
        actual: 9,
    };
    assert!(err.to_string().contains("peer speaks v9"));
    assert!(err.to_string().contains("this build speaks v1"));
}
