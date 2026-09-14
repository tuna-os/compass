//! The checked-in contract XML is compared against a live object server.
//!
//! `PLAN.md` §6 Phase 3 asks us to "publish the versioned interface XML in-tree
//! so the extension and the engine can be reviewed against one another". A
//! document nobody checks is a document that drifts, and the previous check
//! was a substring match against a member list typed into the same file —
//! see `support/introspect.rs` for why that could not fail for its stated
//! reason.
//!
//! WHAT THIS PROVES, AND BY WHAT CHAIN
//!
//! `zbus` offers no way to introspect a `#[zbus::proxy]` trait, so the XML
//! cannot be compared to `proxy.rs` directly. The chain is two links:
//!
//!   1. **XML ≡ mock.** This file. Both interfaces are served on a private bus
//!      and their `org.freedesktop.DBus.Introspectable.Introspect` output is
//!      compared, member by member and argument by argument, against the
//!      checked-in document.
//!
//!   2. **mock ≡ proxy.** `every_contract_member_is_reached_through_the_proxy`
//!      below drives every method, property and signal of both interfaces
//!      through the real `ShellClient`. A signature disagreement between the
//!      proxy and the mock is a failed call or a decode error, so that test
//!      fails if the two drift on any member.
//!
//! Together those give XML ≡ proxy, which is the claim `contract.rs` makes.
//! Link 2 is only as strong as its coverage, which is why it is one test over
//! the whole surface rather than an argument about the suite as a whole.
//!
//! **What no test in this repository can prove** is that the real GNOME Shell
//! extension implements this contract; that needs the extension, and it lives
//! outside this tree. The XML is the artefact the two sides are reviewed
//! against, and this makes our side of it true.

mod support;

use support::introspect::{self, Arg, Property};
use support::mock::{MockOptions, MockShell};
use support::{bus, mock};

use compass_shell::{
    CLIPBOARD_INTERFACE, CLIPBOARD_PATH, WINDOWS_INTERFACE, WINDOWS_PATH, contract,
};

/// Fetches the introspection document a served object actually emits.
async fn live_xml(address: &str, path: &str) -> String {
    let conn = zbus::connection::Builder::address(address)
        .expect("private bus address")
        .build()
        .await
        .expect("connect to the private bus");

    zbus::fdo::IntrospectableProxy::builder(&conn)
        .destination(contract::SHELL_SERVICE)
        .expect("well-known name")
        .path(path)
        .expect("object path")
        .build()
        .await
        .expect("build the introspectable proxy")
        .introspect()
        .await
        .expect("the object server should answer Introspect")
}

// ---------------------------------------------------------------------------
// Link 1: the XML matches what is actually served
// ---------------------------------------------------------------------------

#[tokio::test]
async fn served_interfaces_match_the_checked_in_xml() {
    let Some(bus) = bus::start_or_skip("served_interfaces_match_the_checked_in_xml") else {
        return;
    };
    let _shell = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock shell");

    for (path, interface, declared_xml) in [
        (WINDOWS_PATH, WINDOWS_INTERFACE, contract::WINDOWS_XML),
        (CLIPBOARD_PATH, CLIPBOARD_INTERFACE, contract::CLIPBOARD_XML),
    ] {
        let declared = introspect::parse(declared_xml, interface).unwrap_or_else(|| {
            panic!("the checked-in XML for {interface} does not declare that interface")
        });
        let served = live_xml(bus.address(), path).await;
        let live = introspect::parse(&served, interface).unwrap_or_else(|| {
            panic!("the object at {path} does not serve {interface}:\n{served}")
        });

        let differences = introspect::differences(&declared, &live);
        assert!(
            differences.is_empty(),
            "{interface} has drifted from its checked-in XML \
             (crates/compass-shell/dbus/…). Update the document and bump \
             CONTRACT_VERSION if the change is breaking:\n  - {}",
            differences.join("\n  - ")
        );

        // The comparison above is symmetric over members that exist; assert
        // separately that the contract is not empty, so a parse that silently
        // found nothing cannot pass as agreement.
        assert!(
            !declared.methods.is_empty() && !declared.properties.is_empty(),
            "parsed {interface} as {} method(s) and {} property/properties, which means the \
             parser, not the contract, is what agreed with itself",
            declared.methods.len(),
            declared.properties.len()
        );
    }
}

/// The exact shape of the contract, spelled out rather than inferred.
///
/// The test above proves the XML and the wire agree. It cannot prove they
/// agree on the *right* thing: deleting `CloseWindow` from both would leave
/// them in perfect agreement. This pins the surface, so shrinking it is a
/// deliberate edit here and not a silent one.
#[test]
fn the_contract_is_exactly_these_members() {
    let windows = introspect::parse(contract::WINDOWS_XML, WINDOWS_INTERFACE).expect("windows XML");
    assert_eq!(
        windows.methods.keys().collect::<Vec<_>>(),
        ["ActivateWindow", "CloseWindow", "ListWindows"],
        "the windows contract gained or lost a method"
    );
    assert_eq!(
        windows.signals.keys().collect::<Vec<_>>(),
        ["WindowsChanged"],
        "the windows contract gained or lost a signal"
    );
    assert_eq!(
        windows.methods["ActivateWindow"],
        [Arg {
            name: Some("id".into()),
            ty: "u".into(),
            direction: "in".into()
        }]
    );
    assert_eq!(
        windows.methods["ListWindows"],
        [Arg {
            name: Some("windows".into()),
            ty: "aa{sv}".into(),
            direction: "out".into()
        }]
    );

    let clipboard =
        introspect::parse(contract::CLIPBOARD_XML, CLIPBOARD_INTERFACE).expect("clipboard XML");
    assert_eq!(
        clipboard.methods.keys().collect::<Vec<_>>(),
        ["GetClipboard", "SetClipboard"],
        "the clipboard contract gained or lost a method"
    );
    assert_eq!(
        clipboard.signals["ClipboardChanged"]
            .iter()
            .map(|a| a.ty.as_str())
            .collect::<Vec<_>>(),
        ["ay", "s", "s"],
        "ClipboardChanged's payload changed shape"
    );

    for (name, iface) in [("windows", &windows), ("clipboard", &clipboard)] {
        assert_eq!(
            iface.properties.get("Version"),
            Some(&Property {
                ty: "u".into(),
                access: "read".into()
            }),
            "the {name} contract must expose a read-only u32 Version; version negotiation \
             (compass_shell::CONTRACT_VERSION) is the only thing standing between a GNOME \
             release and a silently wrong client"
        );
    }
}

// ---------------------------------------------------------------------------
// Link 2: the proxies reach every member of the contract
// ---------------------------------------------------------------------------

/// Drives the whole contract surface through the real client in one test.
///
/// Individually these paths are covered by `mock_bus.rs`, but the chain in this
/// file's header depends on *complete* coverage rather than on the suite
/// happening to touch everything, so it is asserted in one place. If a member
/// is added to the contract and not to this test, the size assertions at the
/// end fail.
#[tokio::test]
async fn every_contract_member_is_reached_through_the_proxy() {
    let Some(bus) = bus::start_or_skip("every_contract_member_is_reached_through_the_proxy") else {
        return;
    };
    let shell = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock shell");
    shell.set_windows(vec![
        mock::MockWindow::new(1, "firefox", "A tab").focused(),
        mock::MockWindow::new(2, "gedit", "notes.txt"),
    ]);

    let client = compass_shell::ShellClient::connect(
        compass_shell::ShellConfig::default().with_address(bus.address()),
    )
    .await
    .expect("client");

    // Windows.Version and Clipboard.Version, via the capability probe.
    let caps = client.refresh_capabilities().await;
    let expected = compass_shell::Availability::Available {
        version: compass_shell::CONTRACT_VERSION,
    };
    assert_eq!(caps.windows, expected);
    assert_eq!(caps.clipboard, expected);

    // Windows.ListWindows
    let windows = client.list_windows().await.expect("ListWindows");
    assert_eq!(windows.len(), 2);

    // Both signals are subscribed BEFORE the calls that change state, because
    // a signal emitted before its match rule is installed is simply lost.
    let mut changes = client
        .windows_changed()
        .await
        .expect("WindowsChanged stream");
    let mut clips = client
        .clipboard_changes()
        .await
        .expect("ClipboardChanged stream");

    // Windows.ActivateWindow and Windows.CloseWindow
    client
        .activate_window(compass_shell::WindowId(2))
        .await
        .expect("ActivateWindow");
    client
        .close_window(compass_shell::WindowId(1))
        .await
        .expect("CloseWindow");
    assert_eq!(
        shell.calls(),
        [("ActivateWindow", 2), ("CloseWindow", 1)],
        "the mock did not see both window methods"
    );

    // Clipboard.SetClipboard and Clipboard.GetClipboard
    let written = compass_shell::ClipboardContent::text("hello");
    client.set_clipboard(&written).await.expect("SetClipboard");
    let read_back = client.clipboard().await.expect("GetClipboard");
    assert_eq!(read_back, written, "the clipboard did not round-trip");

    // The two signals. `mock_bus.rs` asserts their payloads in detail; here the
    // point is only that the proxy's signal declarations decode what the mock
    // emits, which is the half of the contract a method call cannot reach.
    shell.emit_windows_changed().await.expect("emit");
    tokio::time::timeout(std::time::Duration::from_secs(5), changes.next())
        .await
        .expect("WindowsChanged should arrive")
        .expect("a change");

    shell
        .emit_clipboard_changed(b"copied", "text/plain", "org.gnome.TextEditor")
        .await
        .expect("emit");
    let entry = tokio::time::timeout(std::time::Duration::from_secs(5), clips.next())
        .await
        .expect("ClipboardChanged should arrive")
        .expect("an entry")
        .expect("a well-formed entry");
    assert_eq!(entry.content.data, b"copied");
    assert_eq!(entry.source_app.as_deref(), Some("org.gnome.TextEditor"));

    // Nothing above may be dropped without this failing: the counts come from
    // the parsed XML, so adding a member to the contract and not to this test
    // is caught here rather than silently reducing the coverage the chain in
    // this file's header depends on.
    let windows_iface =
        introspect::parse(contract::WINDOWS_XML, WINDOWS_INTERFACE).expect("windows XML");
    let clipboard_iface =
        introspect::parse(contract::CLIPBOARD_XML, CLIPBOARD_INTERFACE).expect("clipboard XML");
    const EXERCISED_METHODS: usize = 5; // List/Activate/Close + Get/SetClipboard
    const EXERCISED_SIGNALS: usize = 2;
    assert_eq!(
        windows_iface.methods.len() + clipboard_iface.methods.len(),
        EXERCISED_METHODS,
        "the contract has methods this test does not call, so the mock-and-proxy link in this \
         file's header no longer covers the whole surface"
    );
    assert_eq!(
        windows_iface.signals.len() + clipboard_iface.signals.len(),
        EXERCISED_SIGNALS,
        "the contract has signals this test does not receive"
    );
}

// ---------------------------------------------------------------------------
// Controls: the comparison above is shown to fail when it should
// ---------------------------------------------------------------------------
//
// A comparison that has only ever been run on two documents that agree is not
// evidence of anything. Each control mutates the checked-in XML in one way a
// real change would, and asserts the difference is reported and names the
// member. The positive control asserts an unmutated document is silent, so the
// controls cannot all be passing because `differences` always returns
// something.

/// Applies one textual substitution to the windows contract and reports the
/// differences against the unmutated document.
fn diff_after(from: &str, to: &str) -> Vec<String> {
    let original = contract::WINDOWS_XML;
    assert!(
        original.contains(from),
        "the control's search string {from:?} is not in the contract XML, so this control is \
         testing nothing"
    );
    let mutated = original.replacen(from, to, 1);
    let declared = introspect::parse(original, WINDOWS_INTERFACE).expect("original");
    let live = introspect::parse(&mutated, WINDOWS_INTERFACE).expect("mutated");
    introspect::differences(&declared, &live)
}

#[test]
fn control_an_unmutated_document_reports_nothing() {
    let parsed = introspect::parse(contract::WINDOWS_XML, WINDOWS_INTERFACE).expect("windows XML");
    assert!(
        introspect::differences(&parsed, &parsed).is_empty(),
        "the comparison reports differences between a document and itself, so every other \
         control here proves nothing"
    );
}

#[test]
fn control_a_renamed_method_is_reported() {
    let differences = diff_after(
        "<method name=\"ActivateWindow\">",
        "<method name=\"RaiseWindow\">",
    );
    assert!(
        differences.iter().any(|d| d.contains("ActivateWindow")),
        "renaming a method went unreported: {differences:?}"
    );
    assert!(
        differences.iter().any(|d| d.contains("RaiseWindow")),
        "the new name was not reported as unexpected: {differences:?}"
    );
}

#[test]
fn control_a_retyped_argument_is_reported() {
    let differences = diff_after(
        "<arg type=\"u\" name=\"id\" direction=\"in\"/>",
        "<arg type=\"s\" name=\"id\" direction=\"in\"/>",
    );
    assert!(
        differences
            .iter()
            .any(|d| d.contains("\"u\"") && d.contains("\"s\"")),
        "changing an argument's type went unreported: {differences:?}"
    );
}

#[test]
fn control_a_flipped_direction_is_reported() {
    let differences = diff_after(
        "<arg type=\"aa{sv}\" name=\"windows\" direction=\"out\"/>",
        "<arg type=\"aa{sv}\" name=\"windows\" direction=\"in\"/>",
    );
    assert!(
        differences.iter().any(|d| d.contains("direction")),
        "flipping an argument's direction went unreported: {differences:?}"
    );
}

#[test]
fn control_a_renamed_in_argument_is_reported() {
    let differences = diff_after(
        "<arg type=\"u\" name=\"id\" direction=\"in\"/>",
        "<arg type=\"u\" name=\"window_id\" direction=\"in\"/>",
    );
    assert!(
        differences.iter().any(|d| d.contains("window_id")),
        "renaming an in argument went unreported: {differences:?}"
    );
}

#[test]
fn control_a_dropped_signal_is_reported() {
    let differences = diff_after("<signal name=\"WindowsChanged\"/>", "");
    assert!(
        differences
            .iter()
            .any(|d| d.contains("WindowsChanged") && d.contains("absent")),
        "removing a signal went unreported: {differences:?}"
    );
}

#[test]
fn control_a_changed_property_access_is_reported() {
    let differences = diff_after(
        "<property name=\"Version\" type=\"u\" access=\"read\"/>",
        "<property name=\"Version\" type=\"u\" access=\"readwrite\"/>",
    );
    assert!(
        differences
            .iter()
            .any(|d| d.contains("Version") && d.contains("readwrite")),
        "widening a property's access went unreported: {differences:?}"
    );
}

#[test]
fn control_an_added_method_is_reported() {
    let differences = diff_after(
        "<signal name=\"WindowsChanged\"/>",
        "<method name=\"MoveWindow\"><arg type=\"u\" name=\"id\" direction=\"in\"/></method>\
         <signal name=\"WindowsChanged\"/>",
    );
    assert!(
        differences
            .iter()
            .any(|d| d.contains("MoveWindow") && d.contains("missing from the XML")),
        "a method served but not declared went unreported: {differences:?}"
    );
}

/// An out-argument name difference is deliberately NOT reported, because
/// `zbus` does not emit out-argument names at all and reporting it would fail
/// every run. This pins that as an intended limitation rather than an
/// oversight; see `support/introspect.rs`.
#[test]
fn control_an_unnamed_out_argument_is_tolerated() {
    let differences = diff_after(
        "<arg type=\"aa{sv}\" name=\"windows\" direction=\"out\"/>",
        "<arg type=\"aa{sv}\" direction=\"out\"/>",
    );
    assert!(
        differences.is_empty(),
        "dropping an out argument's NAME must be tolerated — zbus omits them — but the \
         comparison reported: {differences:?}"
    );
}
