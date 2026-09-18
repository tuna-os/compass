//! The notification call, against a real bus and a real server.
//!
//! `Notify` takes eight positional arguments, so the whole of this port is
//! getting their order and types right — and nothing about that can be checked
//! by reading the code that sends them. A mock server on a private
//! `dbus-daemon` records exactly what arrives.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use compass_notify::{Notification, Urgency};
use support::bus;
use zbus::zvariant::{OwnedValue, Value};

/// One recorded call to `Notify`.
#[derive(Debug, Clone, Default)]
struct Received {
    app_name: String,
    replaces_id: u32,
    icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedValue>,
    timeout: i32,
}

/// A notification server that remembers what it was told.
struct MockServer {
    received: Arc<Mutex<Vec<Received>>>,
    next_id: Arc<Mutex<u32>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl MockServer {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let mut id = self.next_id.lock().expect("the id counter");
        *id += 1;

        self.received.lock().expect("the log").push(Received {
            app_name,
            replaces_id,
            icon: app_icon,
            summary,
            body,
            actions,
            hints,
            timeout: expire_timeout,
        });

        *id
    }
}

/// Starts a bus with the mock server on it, and connects a client to it.
async fn serve(address: &str) -> (zbus::Connection, Arc<Mutex<Vec<Received>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let server = MockServer {
        received: Arc::clone(&received),
        next_id: Arc::new(Mutex::new(0)),
    };

    let _server_conn = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .name(compass_notify::SERVICE)
        .expect("the well-known name")
        .serve_at(compass_notify::PATH, server)
        .expect("serve the interface")
        .build()
        .await
        .expect("the mock server connects");

    // Leaked on purpose: the connection has to outlive this function, and a
    // test process that ends is the cleanup.
    std::mem::forget(_server_conn);

    let client = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .build()
        .await
        .expect("the client connects");

    (client, received)
}

fn hint_u8(received: &Received, name: &str) -> Option<u8> {
    let value = received.hints.get(name)?;
    match Value::from(value.clone()) {
        Value::U8(byte) => Some(byte),
        _ => None,
    }
}

#[tokio::test]
async fn the_eight_arguments_arrive_in_the_order_notify_declares_them() {
    let Some(bus) =
        bus::start_or_skip("the_eight_arguments_arrive_in_the_order_notify_declares_them")
    else {
        return;
    };
    let (client, received) = serve(bus.address()).await;

    let id = compass_notify::send(
        &client,
        &Notification::new("A title", "A body")
            .icon("dialog-information")
            .urgency(Urgency::High),
    )
    .await
    .expect("the mock server answers");

    assert_eq!(id, 1, "the server's id is returned, not invented");

    let log = received.lock().expect("the log");
    assert_eq!(log.len(), 1);
    let call = &log[0];

    assert_eq!(call.app_name, "Vicinae", "the app name the C++ sends");
    assert_eq!(call.replaces_id, 0, "a new notification, not a replacement");
    assert_eq!(call.icon, "dialog-information");
    assert_eq!(call.summary, "A title", "the title is the summary");
    assert_eq!(call.body, "A body");
    assert!(call.actions.is_empty(), "the C++ sends no actions");
    assert_eq!(call.timeout, -1, "the server decides how long to show it");
    assert_eq!(
        hint_u8(call, "urgency"),
        Some(2),
        "High is 2 in the hint, as a byte"
    );
}

#[tokio::test]
async fn an_absent_icon_is_an_empty_string_rather_than_a_missing_argument() {
    // `n.iconPath.value_or(QString())`. `Notify` is positional: leaving the
    // argument out would shift every later one along, so a notification with
    // no icon would arrive with its title in the icon slot.
    let Some(bus) =
        bus::start_or_skip("an_absent_icon_is_an_empty_string_rather_than_a_missing_argument")
    else {
        return;
    };
    let (client, received) = serve(bus.address()).await;

    compass_notify::send(&client, &Notification::new("Title", "Body"))
        .await
        .expect("the mock server answers");

    let log = received.lock().expect("the log");
    let call = &log[0];
    assert_eq!(call.icon, "");
    assert_eq!(
        call.summary, "Title",
        "the title is still in the summary slot"
    );
    assert_eq!(hint_u8(call, "urgency"), Some(1), "the default is Normal");
}

#[tokio::test]
async fn each_urgency_reaches_the_server_as_its_own_byte() {
    let Some(bus) = bus::start_or_skip("each_urgency_reaches_the_server_as_its_own_byte") else {
        return;
    };
    let (client, received) = serve(bus.address()).await;

    for (urgency, expected) in [
        (Urgency::Low, 0u8),
        (Urgency::Normal, 1),
        (Urgency::High, 2),
    ] {
        compass_notify::send(&client, &Notification::new("T", "B").urgency(urgency))
            .await
            .expect("the mock server answers");

        let log = received.lock().expect("the log");
        assert_eq!(
            hint_u8(log.last().expect("a call"), "urgency"),
            Some(expected),
            "{urgency:?} should be {expected}"
        );
    }
}

#[tokio::test]
async fn the_ids_the_server_mints_come_back_in_order() {
    // The divergence from the C++, which throws the id away: a caller that
    // wants to replace or close a notification needs it.
    let Some(bus) = bus::start_or_skip("the_ids_the_server_mints_come_back_in_order") else {
        return;
    };
    let (client, _received) = serve(bus.address()).await;

    let first = compass_notify::send(&client, &Notification::new("One", ""))
        .await
        .expect("sent");
    let second = compass_notify::send(&client, &Notification::new("Two", ""))
        .await
        .expect("sent");

    assert_eq!((first, second), (1, 2));
}

#[tokio::test]
async fn a_bus_with_no_notification_server_is_an_error_not_a_silent_success() {
    // A machine with no server is the ordinary case, and a caller that wanted
    // to tell someone something should learn that it did not.
    let Some(bus) =
        bus::start_or_skip("a_bus_with_no_notification_server_is_an_error_not_a_silent_success")
    else {
        return;
    };

    let client = zbus::connection::Builder::address(bus.address())
        .expect("the private bus address")
        .build()
        .await
        .expect("the client connects");

    let error = compass_notify::send(&client, &Notification::new("T", "B"))
        .await
        .expect_err("there is no server on this bus");
    assert!(
        format!("{error}").contains("could not be sent"),
        "the error should say what failed: {error}"
    );
}
