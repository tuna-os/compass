//! Clipboard history and the keyring it keeps its key in.
//!
//! Part of [`super`]; see that module for the purity rule. Clipboard history
//! needs two things a bare compositor session may lack: a way to see what
//! other applications copy (the data-control protocol, or on GNOME the Shell
//! extension), and a keyring answering the Secret Service on the session bus,
//! where the key that encrypts the history is kept. Without the second, the
//! engine only logs a keyring error; this says what to install.

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use super::wlroots::WaylandFindings;
use crate::doctor::bus::BusProbe;
use crate::doctor::env::Env;

/// The Secret Service's well-known name.
pub const SECRET_SERVICE_BUS_NAME: &str = "org.freedesktop.secrets";
/// The default collection, through its alias.
pub const DEFAULT_COLLECTION_PATH: &str = "/org/freedesktop/secrets/aliases/default";
/// The collection interface, whose `Locked` says whether it is open.
pub const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";

/// `clipboard.history`: whether copies made in other applications reach the
/// history.
#[must_use]
pub fn clipboard_history(env: &Env, wayland: Option<&WaylandFindings>) -> DoctorCheck {
    const NAME: &str = "clipboard.history";
    if super::desktop::is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Ok,
            "on GNOME, clipboard history comes from the Compass GNOME Shell extension \
             (see gnome.shell-extension) and keeps its key in the keyring (see keyring)",
        );
    }
    match wayland {
        Some(found) if found.capabilities.data_control => check(
            NAME,
            DoctorStatus::Ok,
            "the compositor lets Compass see what is copied in other applications, so \
             clipboard history records it (it also needs the keyring: see keyring)",
        ),
        Some(_) => check(
            NAME,
            DoctorStatus::Warn,
            "the compositor does not offer the data-control protocol, so clipboard history \
             only records what is copied while the launcher is open",
        ),
        None => check(
            NAME,
            DoctorStatus::Warn,
            "no Wayland display was found, so clipboard history cannot follow the clipboard",
        ),
    }
}

/// `keyring`: whether a keyring answers, and is unlocked.
pub async fn keyring<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "keyring";
    let advice = match super::desktop::wlroots_compositor(env) {
        Some(compositor) => format!(
            "Install GNOME Keyring (or KeePassXC with its Secret Service integration on) and \
             start it with your session, for example with \
             `exec gnome-keyring-daemon --start --components=secrets` in {}",
            compositor.config_file()
        ),
        None => "Install GNOME Keyring, KWallet or KeePassXC with its Secret Service \
                 integration on, and make sure it starts with your session"
            .to_owned(),
    };
    let lost = "clipboard history, extension preferences and extension storage are off";
    let locked = bus
        .property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
        )
        .await;
    match locked {
        Ok(Some(value)) if value == "false" => check(
            NAME,
            DoctorStatus::Ok,
            "a keyring answers on the session bus and is unlocked; Compass keeps the key \
             that encrypts clipboard history and extension data there",
        ),
        Ok(Some(_)) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "the keyring is locked, so {lost} until it is unlocked. Logging in with a \
                 password normally unlocks it; otherwise unlock it in your keyring manager"
            ),
        ),
        Ok(None) => match bus.name_has_owner(SECRET_SERVICE_BUS_NAME).await {
            Ok(true) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "a keyring answers on the session bus but has no default collection, so \
                     {lost} until one is created. Open your keyring manager and create a \
                     keyring named Login, or set one as the default"
                ),
            ),
            _ => check(
                NAME,
                DoctorStatus::Warn,
                format!("no keyring answers on the session bus, so {lost}. {advice}"),
            ),
        },
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!("could not ask the session bus for a keyring ({err}), so {lost}. {advice}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::checks::tests::detail;
    use compass_wayland::compositor::{Capabilities, Family};

    fn sway() -> Env {
        Env::from_pairs([("SWAYSOCK", "/run/user/1000/sway-ipc.sock")])
    }

    #[tokio::test]
    async fn an_unlocked_keyring_passes_and_a_locked_one_says_so() {
        let unlocked = FakeBus::new().with_property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
            "false",
        );
        assert_eq!(keyring(&sway(), &unlocked).await.status, DoctorStatus::Ok);
        let locked = FakeBus::new().with_property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
            "true",
        );
        let c = keyring(&sway(), &locked).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("is locked"));
    }

    #[tokio::test]
    async fn no_keyring_on_sway_says_what_to_install_and_where_to_start_it() {
        let c = keyring(&sway(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("no keyring answers"), "{d}");
        assert!(d.contains("gnome-keyring-daemon"), "{d}");
        assert!(d.contains("~/.config/sway/config"), "{d}");
    }

    #[test]
    fn clipboard_history_follows_data_control() {
        let found = |data_control| WaylandFindings {
            family: Family::Wlroots,
            capabilities: Capabilities {
                data_control,
                ..Capabilities::default()
            },
            compositor_ipc: None,
        };
        let c = clipboard_history(&sway(), Some(&found(true)));
        assert_eq!(c.status, DoctorStatus::Ok);
        let c = clipboard_history(&sway(), Some(&found(false)));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("while the launcher is open"));
    }
}
