//! The logind calls, against a real bus and a mock manager.
//!
//! The part worth testing is not that a method name is spelled right — a unit
//! test covers that — but that the arguments and the *replies* are the shapes
//! logind uses. `ListSessions` answers `a(susso)`, and `CanHibernate` answers
//! a string that the C++ never reads.

mod support;

use std::sync::{Arc, Mutex};

use compass_power::{Action, Capability, PowerManager};
use support::bus;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

/// One row of `ListSessions`, as logind's `a(susso)` decodes.
type SessionRow = (String, u32, String, String, OwnedObjectPath);

/// What the mock was asked to do.
#[derive(Debug, Clone, Default)]
struct Calls {
    performed: Vec<(String, bool)>,
    flags: Vec<u64>,
    sleep: Vec<u64>,
    locked: Vec<String>,
    terminated: Vec<String>,
}

struct MockLogind {
    calls: Arc<Mutex<Calls>>,
    /// What each `CanX` answers.
    answers: Arc<Mutex<std::collections::HashMap<String, String>>>,
    sessions: Arc<Mutex<Vec<SessionRow>>>,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl MockLogind {
    #[zbus(name = "PowerOff")]
    fn power_off(&self, interactive: bool) {
        self.calls
            .lock()
            .expect("log")
            .performed
            .push(("PowerOff".to_owned(), interactive));
    }

    #[zbus(name = "Reboot")]
    fn reboot(&self, interactive: bool) {
        self.calls
            .lock()
            .expect("log")
            .performed
            .push(("Reboot".to_owned(), interactive));
    }

    #[zbus(name = "Suspend")]
    fn suspend(&self, interactive: bool) {
        self.calls
            .lock()
            .expect("log")
            .performed
            .push(("Suspend".to_owned(), interactive));
    }

    #[zbus(name = "Hibernate")]
    fn hibernate(&self, interactive: bool) {
        self.calls
            .lock()
            .expect("log")
            .performed
            .push(("Hibernate".to_owned(), interactive));
    }

    #[zbus(name = "RebootWithFlags")]
    fn reboot_with_flags(&self, flags: u64) {
        self.calls.lock().expect("log").flags.push(flags);
    }

    #[zbus(name = "Sleep")]
    fn sleep(&self, flags: u64) {
        self.calls.lock().expect("log").sleep.push(flags);
    }

    #[zbus(name = "LockSession")]
    fn lock_session(&self, id: String) {
        self.calls.lock().expect("log").locked.push(id);
    }

    #[zbus(name = "TerminateSession")]
    fn terminate_session(&self, id: String) {
        self.calls.lock().expect("log").terminated.push(id);
    }

    #[zbus(name = "ListSessions")]
    fn list_sessions(&self) -> Vec<SessionRow> {
        self.sessions.lock().expect("sessions").clone()
    }

    #[zbus(name = "CanPowerOff")]
    fn can_power_off(&self) -> String {
        self.answer("CanPowerOff")
    }

    #[zbus(name = "CanReboot")]
    fn can_reboot(&self) -> String {
        self.answer("CanReboot")
    }

    #[zbus(name = "CanSuspend")]
    fn can_suspend(&self) -> String {
        self.answer("CanSuspend")
    }

    #[zbus(name = "CanHibernate")]
    fn can_hibernate(&self) -> String {
        self.answer("CanHibernate")
    }
}

impl MockLogind {
    fn answer(&self, method: &str) -> String {
        self.answers
            .lock()
            .expect("answers")
            .get(method)
            .cloned()
            .unwrap_or_else(|| "yes".to_owned())
    }
}

struct Harness {
    manager: PowerManager,
    calls: Arc<Mutex<Calls>>,
    answers: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

fn path(id: &str) -> OwnedObjectPath {
    ObjectPath::try_from(format!("/org/freedesktop/login1/session/{id}"))
        .expect("a valid object path")
        .into()
}

async fn serve(address: &str) -> Harness {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let answers = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let sessions = Arc::new(Mutex::new(vec![
        (
            "ssh".to_owned(),
            1000,
            "someone".to_owned(),
            String::new(),
            path("ssh"),
        ),
        (
            "desktop".to_owned(),
            1000,
            "someone".to_owned(),
            "seat0".to_owned(),
            path("desktop"),
        ),
    ]));

    let mock = MockLogind {
        calls: Arc::clone(&calls),
        answers: Arc::clone(&answers),
        sessions,
    };

    let server = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .name(compass_power::SERVICE)
        .expect("the well-known name")
        .serve_at(compass_power::PATH, mock)
        .expect("serve the interface")
        .build()
        .await
        .expect("the mock logind connects");
    std::mem::forget(server);

    let client = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .build()
        .await
        .expect("the client connects");

    Harness {
        manager: PowerManager::new(client),
        calls,
        answers,
    }
}

#[tokio::test]
async fn every_action_reaches_logind_by_its_own_name() {
    let Some(bus) = bus::start_or_skip("every_action_reaches_logind_by_its_own_name") else {
        return;
    };
    let harness = serve(bus.address()).await;

    for action in [
        Action::PowerOff,
        Action::Reboot,
        Action::Suspend,
        Action::Hibernate,
    ] {
        harness
            .manager
            .perform(action, false)
            .await
            .unwrap_or_else(|e| panic!("{action:?}: {e}"));
    }

    let calls = harness.calls.lock().expect("log");
    assert_eq!(
        calls.performed,
        vec![
            ("PowerOff".to_owned(), false),
            ("Reboot".to_owned(), false),
            ("Suspend".to_owned(), false),
            ("Hibernate".to_owned(), false),
        ]
    );
}

#[tokio::test]
async fn the_capability_reply_is_read_and_not_merely_counted() {
    // The declared divergence, end to end: logind says "na", and this reports
    // that the action cannot be offered. The C++ reports that it can.
    let Some(bus) = bus::start_or_skip("the_capability_reply_is_read_and_not_merely_counted")
    else {
        return;
    };
    let harness = serve(bus.address()).await;

    for (reply, expected, offerable) in [
        ("yes", Capability::Yes, true),
        ("no", Capability::No, false),
        ("challenge", Capability::Challenge, true),
        ("na", Capability::NotAvailable, false),
        ("something-new", Capability::Unknown, false),
    ] {
        harness
            .answers
            .lock()
            .expect("answers")
            .insert("CanHibernate".to_owned(), reply.to_owned());

        let capability = harness
            .manager
            .can(Action::Hibernate)
            .await
            .expect("logind answers");
        assert_eq!(capability, expected, "logind said {reply:?}");
        assert_eq!(capability.is_offerable(), offerable);
    }
}

#[tokio::test]
async fn a_soft_reboot_sends_the_flag_and_sleep_sends_zero() {
    let Some(bus) = bus::start_or_skip("a_soft_reboot_sends_the_flag_and_sleep_sends_zero") else {
        return;
    };
    let harness = serve(bus.address()).await;

    harness.manager.soft_reboot().await.expect("soft reboot");
    harness.manager.sleep().await.expect("sleep");

    let calls = harness.calls.lock().expect("log");
    assert_eq!(calls.flags, vec![compass_power::SOFT_REBOOT_FLAG]);
    assert_eq!(calls.flags, vec![4], "1 << 2, as the C++ writes it");
    assert_eq!(calls.sleep, vec![0]);
}

#[tokio::test]
async fn the_session_list_decodes_and_the_seated_one_is_locked() {
    // `a(susso)`: a struct of string, uint32, string, string, object path.
    // Getting the shape wrong is a decode error, not a wrong answer, which is
    // why this drives it through a real reply.
    let Some(bus) = bus::start_or_skip("the_session_list_decodes_and_the_seated_one_is_locked")
    else {
        return;
    };
    let harness = serve(bus.address()).await;

    let sessions = harness.manager.sessions().await.expect("list sessions");
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "ssh");
    assert_eq!(sessions[0].seat, "");
    assert_eq!(sessions[1].seat, "seat0");
    assert_eq!(sessions[1].path, "/org/freedesktop/login1/session/desktop");

    harness.manager.lock(1000).await.expect("lock");
    harness
        .manager
        .terminate_session(1000)
        .await
        .expect("terminate");

    let calls = harness.calls.lock().expect("log");
    assert_eq!(
        calls.locked,
        vec!["desktop".to_owned()],
        "the seated session is the desktop; the SSH one must not be locked"
    );
    assert_eq!(calls.terminated, vec!["desktop".to_owned()]);
}

#[tokio::test]
async fn a_user_with_no_seated_session_is_told_so() {
    let Some(bus) = bus::start_or_skip("a_user_with_no_seated_session_is_told_so") else {
        return;
    };
    let harness = serve(bus.address()).await;

    let error = harness
        .manager
        .lock(4242)
        .await
        .expect_err("nobody with that uid is logged in");
    assert!(
        matches!(error, compass_power::Error::NoSession),
        "got {error:?}"
    );

    assert!(
        harness.calls.lock().expect("log").locked.is_empty(),
        "nothing should have been locked"
    );
}

#[tokio::test]
async fn a_bus_with_no_logind_is_an_error() {
    let Some(bus) = bus::start_or_skip("a_bus_with_no_logind_is_an_error") else {
        return;
    };
    let client = zbus::connection::Builder::address(bus.address())
        .expect("the private bus address")
        .build()
        .await
        .expect("the client connects");

    let manager = PowerManager::new(client);
    assert!(
        manager.perform(Action::PowerOff, false).await.is_err(),
        "a machine without logind must not report that it powered off"
    );
}
