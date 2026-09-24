//! Power actions, over systemd-logind.
//!
//! Ports `SystemdPowerManager`
//! (`src/server/src/services/power-manager/systemd/`). Every action is one
//! method on `org.freedesktop.login1.Manager`, on the **system** bus.
//!
//! # One declared divergence, and it is a bug fix
//!
//! The C++ asks logind whether an action is possible and then does not read
//! the answer:
//!
//! ```cpp
//! bool SystemdPowerManager::can(const QString &method) const {
//!   auto reply = m_iface->call(method);
//!   auto args = reply.arguments();
//!   if (args.isEmpty()) return false;
//!   return true;
//! }
//! ```
//!
//! `CanHibernate` replies with a *string* — `"yes"`, `"no"`, `"challenge"` or
//! `"na"` — and every one of those is a non-empty argument list. So the C++
//! reports that a machine can hibernate when logind has just said it cannot,
//! and the user gets a menu entry that does nothing.
//!
//! [`Capability`] reads the string. This is a divergence on purpose: the
//! behaviour is wrong rather than merely different, nothing on disk depends on
//! it, and a port that reproduced it would be copying a defect into a second
//! engine. It is recorded here and in `PARITY.md` rather than done quietly.
//!
//! # What is not here
//!
//! `logout()` in the C++ also quits the application, and on KDE and GNOME it
//! calls the desktop's own session manager on the *session* bus first.
//! [`LogoutTarget`] is that decision, as data; performing it belongs to
//! whoever owns the process's lifetime.

use zbus::zvariant::OwnedObjectPath;

/// The logind service.
pub const SERVICE: &str = "org.freedesktop.login1";
/// The manager object.
pub const PATH: &str = "/org/freedesktop/login1";
/// The manager interface.
pub const INTERFACE: &str = "org.freedesktop.login1.Manager";

/// The `RebootWithFlags` bit the C++ sends for a soft reboot.
///
/// `SD_LOGIND_SOFT_REBOOT = 1 << 2` there, written down here because it goes
/// on the wire as a number and nothing else in this crate would catch a change
/// to it.
pub const SOFT_REBOOT_FLAG: u64 = 1 << 2;

/// What logind says about an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// It can be done now.
    Yes,
    /// It cannot.
    No,
    /// It can, after the user authenticates.
    Challenge,
    /// The hardware or configuration does not support it.
    NotAvailable,
    /// logind said something this build does not know.
    Unknown,
}

impl Capability {
    /// Reads logind's reply string.
    #[must_use]
    pub fn parse(reply: &str) -> Self {
        match reply {
            "yes" => Self::Yes,
            "no" => Self::No,
            "challenge" => Self::Challenge,
            "na" => Self::NotAvailable,
            _ => Self::Unknown,
        }
    }

    /// Whether to offer the action to the user.
    ///
    /// `Challenge` counts: the action is possible, and the user will be asked
    /// for a password by polkit. `Unknown` does not — a reply this build
    /// cannot read is not a promise.
    #[must_use]
    pub fn is_offerable(self) -> bool {
        matches!(self, Self::Yes | Self::Challenge)
    }
}

/// One entry of `ListSessions`.
///
/// The signature is `a(susso)`: id, uid, user name, seat, object path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// logind's session id, e.g. `"2"`.
    pub id: String,
    /// The owning user.
    pub uid: u32,
    /// That user's name.
    pub user: String,
    /// The seat, empty for a session with no seat.
    pub seat: String,
    /// The session object.
    pub path: String,
}

/// Which session is this user's graphical one.
///
/// `getUserSession` in the C++: the first with our uid **and a non-empty
/// seat**. The seat check is what skips an SSH session or a user service
/// manager, which would otherwise be locked or terminated instead of the
/// desktop.
#[must_use]
pub fn current_session(sessions: &[Session], uid: u32) -> Option<&Session> {
    sessions
        .iter()
        .find(|session| session.uid == uid && !session.seat.is_empty())
}

/// Who should be asked to end the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoutTarget {
    /// `org.kde.Shutdown.logout`, on the session bus.
    Plasma,
    /// `org.gnome.SessionManager.Logout(1)` — 1 means "no confirmation".
    Gnome,
    /// `org.freedesktop.login1.Manager.TerminateSession`, on the system bus.
    Logind,
}

/// The logout path for a desktop, from `$XDG_CURRENT_DESKTOP`.
///
/// The C++ asks `Environment::isPlasmaDesktop()` then `isGnomeDesktop()` and
/// otherwise falls back to logind, in that order. Matching is
/// case-insensitive and per entry, because the variable is a colon-separated
/// list — `ubuntu:GNOME` is GNOME.
#[must_use]
pub fn logout_target(current_desktop: &str) -> LogoutTarget {
    let entries: Vec<String> = current_desktop
        .split(':')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .collect();

    if entries
        .iter()
        .any(|entry| entry == "kde" || entry == "plasma")
    {
        return LogoutTarget::Plasma;
    }
    if entries.iter().any(|entry| entry == "gnome") {
        return LogoutTarget::Gnome;
    }
    LogoutTarget::Logind
}

/// Why an action failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bus refused, or logind is not there.
    #[error("logind refused or is unavailable: {0}")]
    Bus(#[from] zbus::Error),

    /// This user has no seated session, so there is nothing to lock or end.
    #[error("this user has no seated session")]
    NoSession,
}

/// The actions logind offers, as the C++ names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Power off.
    PowerOff,
    /// Reboot.
    Reboot,
    /// Suspend.
    Suspend,
    /// Hibernate.
    Hibernate,
}

impl Action {
    /// The method that performs it.
    #[must_use]
    pub fn method(self) -> &'static str {
        match self {
            Self::PowerOff => "PowerOff",
            Self::Reboot => "Reboot",
            Self::Suspend => "Suspend",
            Self::Hibernate => "Hibernate",
        }
    }

    /// The method that asks whether it is possible.
    ///
    /// logind's convention, and the C++'s: `Can` plus the action's name.
    #[must_use]
    pub fn query(self) -> &'static str {
        match self {
            Self::PowerOff => "CanPowerOff",
            Self::Reboot => "CanReboot",
            Self::Suspend => "CanSuspend",
            Self::Hibernate => "CanHibernate",
        }
    }
}

/// `org.freedesktop.login1.Manager`: the methods this crate calls.
#[allow(missing_docs)]
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn hibernate(&self, interactive: bool) -> zbus::Result<()>;
    fn can_power_off(&self) -> zbus::Result<String>;
    fn can_reboot(&self) -> zbus::Result<String>;
    fn can_suspend(&self) -> zbus::Result<String>;
    fn can_hibernate(&self) -> zbus::Result<String>;
    fn reboot_with_flags(&self, flags: u64) -> zbus::Result<()>;
    fn sleep(&self, flags: u64) -> zbus::Result<()>;
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
    fn lock_session(&self, session_id: &str) -> zbus::Result<()>;
    fn terminate_session(&self, session_id: &str) -> zbus::Result<()>;
}

/// `org.gnome.SessionManager`, for GNOME's logout.
#[allow(missing_docs)]
#[zbus::proxy(
    interface = "org.gnome.SessionManager",
    default_service = "org.gnome.SessionManager",
    default_path = "/org/gnome/SessionManager"
)]
trait GnomeSession {
    fn logout(&self, mode: u32) -> zbus::Result<()>;
}

/// `org.kde.Shutdown`, for Plasma's logout.
#[allow(missing_docs)]
#[zbus::proxy(
    interface = "org.kde.Shutdown",
    default_service = "org.kde.Shutdown",
    default_path = "/Shutdown"
)]
trait PlasmaShutdown {
    fn logout(&self) -> zbus::Result<()>;
}

/// A logind client.
#[derive(Debug)]
pub struct PowerManager {
    connection: zbus::Connection,
}

impl PowerManager {
    /// Wraps a connection to the bus logind is on.
    ///
    /// The connection is passed in rather than opened here: logind lives on
    /// the system bus, and a test needs to point this at a private one.
    #[must_use]
    pub fn new(connection: zbus::Connection) -> Self {
        Self { connection }
    }

    async fn manager(&self) -> Result<ManagerProxy<'_>, Error> {
        Ok(ManagerProxy::new(&self.connection).await?)
    }

    /// Performs `action`. `interactive` is logind's own flag.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if logind refuses.
    pub async fn perform(&self, action: Action, interactive: bool) -> Result<(), Error> {
        let manager = self.manager().await?;
        match action {
            Action::PowerOff => manager.power_off(interactive).await?,
            Action::Reboot => manager.reboot(interactive).await?,
            Action::Suspend => manager.suspend(interactive).await?,
            Action::Hibernate => manager.hibernate(interactive).await?,
        }
        Ok(())
    }

    /// Asks whether `action` is possible.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if logind refuses.
    pub async fn can(&self, action: Action) -> Result<Capability, Error> {
        let manager = self.manager().await?;
        let answer = match action {
            Action::PowerOff => manager.can_power_off().await?,
            Action::Reboot => manager.can_reboot().await?,
            Action::Suspend => manager.can_suspend().await?,
            Action::Hibernate => manager.can_hibernate().await?,
        };
        Ok(Capability::parse(&answer))
    }

    /// Reboots into a soft reboot, as `RebootWithFlags` does.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if logind refuses.
    pub async fn soft_reboot(&self) -> Result<(), Error> {
        self.manager()
            .await?
            .reboot_with_flags(SOFT_REBOOT_FLAG)
            .await?;
        Ok(())
    }

    /// Suspends without asking, as `Sleep(0)` does.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if logind refuses.
    pub async fn sleep(&self) -> Result<(), Error> {
        self.manager().await?.sleep(0).await?;
        Ok(())
    }

    /// Every session logind knows about.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if logind refuses or answers something else.
    pub async fn sessions(&self) -> Result<Vec<Session>, Error> {
        let raw = self.manager().await?.list_sessions().await?;
        Ok(raw
            .into_iter()
            .map(|(id, uid, user, seat, path)| Session {
                id,
                uid,
                user,
                seat,
                path: path.as_str().to_owned(),
            })
            .collect())
    }

    /// Locks this user's seated session.
    ///
    /// # Errors
    ///
    /// [`Error::NoSession`] if there is no seated session for `uid`, and
    /// [`Error::Bus`] if logind refuses.
    pub async fn lock(&self, uid: u32) -> Result<(), Error> {
        let sessions = self.sessions().await?;
        let session = current_session(&sessions, uid).ok_or(Error::NoSession)?;
        self.manager().await?.lock_session(&session.id).await?;
        Ok(())
    }

    /// Ends this user's seated session.
    ///
    /// # Errors
    ///
    /// As [`lock`](Self::lock).
    pub async fn terminate_session(&self, uid: u32) -> Result<(), Error> {
        let sessions = self.sessions().await?;
        let session = current_session(&sessions, uid).ok_or(Error::NoSession)?;
        self.manager().await?.terminate_session(&session.id).await?;
        Ok(())
    }

    /// Logs this user out the way their desktop does: GNOME's and Plasma's
    /// session managers on `session_bus`, else logind ending the session.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if the session manager or logind refuses, and
    /// [`Error::NoSession`] for logind with no seated session.
    pub async fn logout(
        &self,
        target: LogoutTarget,
        session_bus: &zbus::Connection,
        uid: u32,
    ) -> Result<(), Error> {
        match target {
            // 1: log out without asking again, as the C++ does.
            LogoutTarget::Gnome => GnomeSessionProxy::new(session_bus).await?.logout(1).await?,
            LogoutTarget::Plasma => {
                PlasmaShutdownProxy::new(session_bus)
                    .await?
                    .logout()
                    .await?
            }
            LogoutTarget::Logind => self.terminate_session(uid).await?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn session(id: &str, uid: u32, seat: &str) -> Session {
        Session {
            id: id.to_owned(),
            uid,
            user: "someone".to_owned(),
            seat: seat.to_owned(),
            path: format!("/org/freedesktop/login1/session/{id}"),
        }
    }

    #[test]
    fn logind_replies_are_read_rather_than_counted() {
        assert_eq!(Capability::parse("yes"), Capability::Yes);
        assert_eq!(Capability::parse("no"), Capability::No);
        assert_eq!(Capability::parse("challenge"), Capability::Challenge);
        assert_eq!(Capability::parse("na"), Capability::NotAvailable);
        assert_eq!(Capability::parse("maybe"), Capability::Unknown);

        assert!(Capability::Yes.is_offerable());
        assert!(
            Capability::Challenge.is_offerable(),
            "polkit will ask for a password; the action is still possible"
        );
        assert!(!Capability::No.is_offerable());
        assert!(!Capability::NotAvailable.is_offerable());
        assert!(
            !Capability::Unknown.is_offerable(),
            "a reply this build cannot read is not a promise"
        );
    }

    #[test]
    fn the_cpp_still_has_the_bug_this_port_declines_to_copy() {
        // If the C++ is ever fixed, this fails and the divergence note in the
        // module docs and in PARITY.md should go.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join("src/server/src/services/power-manager/systemd/systemd-power-manager.cpp");
        let cpp = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let body = cpp
            .split("bool SystemdPowerManager::can(")
            .nth(1)
            .expect("`can` is still there")
            .split("\n}")
            .next()
            .expect("its body");
        assert!(
            body.contains("if (args.isEmpty()) return false;") && body.contains("return true;"),
            "the C++ `can` no longer ignores logind's answer: {body}"
        );
        assert!(
            !body.contains("\"yes\""),
            "the C++ now reads the reply string; this port's divergence is over"
        );
    }

    #[test]
    fn the_seated_session_is_the_one_that_gets_locked() {
        // Without the seat check this locks or terminates an SSH session and
        // leaves the desktop alone.
        let sessions = vec![
            session("ssh", 1000, ""),
            session("desktop", 1000, "seat0"),
            session("other-user", 1001, "seat0"),
        ];
        assert_eq!(
            current_session(&sessions, 1000).map(|s| s.id.as_str()),
            Some("desktop")
        );
        assert_eq!(
            current_session(&sessions, 1002),
            None,
            "a user with no session at all"
        );
        assert_eq!(
            current_session(&[session("ssh", 1000, "")], 1000),
            None,
            "a seatless session is not the desktop"
        );
    }

    #[test]
    fn the_logout_target_follows_the_desktop() {
        assert_eq!(logout_target("KDE"), LogoutTarget::Plasma);
        assert_eq!(logout_target("GNOME"), LogoutTarget::Gnome);
        assert_eq!(
            logout_target("ubuntu:GNOME"),
            LogoutTarget::Gnome,
            "the variable is a list, and this one is GNOME"
        );
        assert_eq!(
            logout_target("gnome"),
            LogoutTarget::Gnome,
            "case does not decide which desktop this is"
        );
        assert_eq!(logout_target("sway"), LogoutTarget::Logind);
        assert_eq!(logout_target(""), LogoutTarget::Logind);
        assert_eq!(
            logout_target("KDE:GNOME"),
            LogoutTarget::Plasma,
            "the C++ asks about Plasma first"
        );
    }

    #[test]
    fn the_method_names_are_loginds() {
        assert_eq!(Action::PowerOff.method(), "PowerOff");
        assert_eq!(Action::PowerOff.query(), "CanPowerOff");
        assert_eq!(Action::Hibernate.query(), "CanHibernate");
        assert_eq!(SOFT_REBOOT_FLAG, 4, "1 << 2, as the C++ writes it");
    }
}
