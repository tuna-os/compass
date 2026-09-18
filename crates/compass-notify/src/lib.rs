//! Desktop notifications, over `org.freedesktop.Notifications`.
//!
//! Ports `FreedesktopNotificationClient`
//! (`src/server/src/services/desktop-notification/freedesktop/`). One method
//! call, and the whole of the port is getting its eight arguments right —
//! `Notify` is positional, so an argument in the wrong place is a body in the
//! summary or a timeout read as an urgency.
//!
//! # Not the Notification portal
//!
//! `org.freedesktop.portal.Notification` exists and is a different interface
//! with a different shape. The C++ calls the plain session-bus service, so
//! this does too: inside a Flatpak the session bus is proxied and the call
//! still arrives, and switching to the portal would change what a notification
//! looks like without anyone deciding to.
//!
//! # What this does that the C++ does not
//!
//! `QDBusConnection::sessionBus().send(msg)` is fire-and-forget: it returns
//! whether the message was *queued*, not whether the notification was shown,
//! and it throws away the id the server returns. This awaits the reply and
//! returns that id, because an id is what a later `CloseNotification` or a
//! replacement needs — and because "queued" is not an outcome worth reporting.

use zbus::zvariant::Value;

/// The service, path and interface, which are all the same string.
pub const SERVICE: &str = "org.freedesktop.Notifications";
/// The object path.
pub const PATH: &str = "/org/freedesktop/Notifications";
/// The interface.
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// The `app_name` every notification is sent under.
///
/// Still `Vicinae`, not `Compass`: it is what the C++ sends, what the binary
/// is still called, and what a notification server may already have rules
/// about. Renaming it is a product decision, not a porting one.
pub const APP_NAME: &str = "Vicinae";

/// How much the notification wants to interrupt.
///
/// The numbers are the specification's and the C++'s `mapUrgency`: they go on
/// the wire as the `urgency` hint, a byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Urgency {
    /// Shown quietly.
    Low = 0,
    /// The default.
    #[default]
    Normal = 1,
    /// Stays until dismissed, on most servers.
    High = 2,
}

/// What to show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notification {
    /// The bold first line.
    pub title: String,
    /// The text under it.
    pub body: String,
    /// An icon name or a path, or nothing.
    pub icon: Option<String>,
    /// How much to interrupt.
    pub urgency: Urgency,
}

impl Notification {
    /// A notification with a title and a body.
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    /// Sets the icon.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the urgency.
    #[must_use]
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }
}

/// Why a notification could not be sent.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bus refused, or nothing is serving the interface.
    ///
    /// A machine with no notification server is the ordinary case of this, and
    /// it is an error rather than a silent success: a caller that wanted to
    /// tell someone something should know it did not.
    #[error("the notification could not be sent: {0}")]
    Bus(#[from] zbus::Error),
}

/// Sends `notification` over `connection`, answering with the server's id.
///
/// The arguments are `Notify`'s, in its order:
/// `app_name, replaces_id, app_icon, summary, body, actions, hints, timeout`.
/// `replaces_id` is 0 (a new notification), `actions` is empty, and `timeout`
/// is -1 (the server decides) — all three as the C++ sends them.
///
/// # Errors
///
/// [`Error::Bus`] if the call fails, which includes there being no
/// notification server.
pub async fn send(
    connection: &zbus::Connection,
    notification: &Notification,
) -> Result<u32, Error> {
    let hints: std::collections::HashMap<&str, Value<'_>> =
        std::collections::HashMap::from([("urgency", Value::U8(notification.urgency as u8))]);
    let actions: Vec<&str> = Vec::new();

    let reply = connection
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "Notify",
            &(
                APP_NAME,
                0u32,
                notification.icon.as_deref().unwrap_or(""),
                notification.title.as_str(),
                notification.body.as_str(),
                actions,
                hints,
                -1i32,
            ),
        )
        .await?;

    Ok(reply.body().deserialize::<u32>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_urgency_numbers_are_the_cpps() {
        // `mapUrgency` in the C++, and the specification's own values. A
        // notification sent with the wrong byte is shown with the wrong
        // prominence, silently.
        assert_eq!(Urgency::Low as u8, 0);
        assert_eq!(Urgency::Normal as u8, 1);
        assert_eq!(Urgency::High as u8, 2);
        assert_eq!(Urgency::default(), Urgency::Normal);

        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join("src/server/src/services/desktop-notification/freedesktop/freedesktop-notification-client.cpp");
        let cpp = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (variant, number) in [("Low", 0), ("Normal", 1), ("High", 2)] {
            assert!(
                cpp.contains(&format!("case Urgency::{variant}:\n    return {number};")),
                "the C++ no longer maps {variant} to {number}"
            );
        }
    }

    #[test]
    fn the_bus_names_are_the_cpps() {
        assert_eq!(SERVICE, "org.freedesktop.Notifications");
        assert_eq!(PATH, "/org/freedesktop/Notifications");
        assert_eq!(INTERFACE, SERVICE);
    }
}
