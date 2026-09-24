//! Binding the launcher hotkey, and what happens when it fires.
//!
//! This is the trigger the resident window was built for
//! ([ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)):
//! the engine holds a GlobalShortcuts session, and an activation becomes a
//! [`WindowCommand::Toggle`] pushed to the attached window.
//!
//! # Why the portal and not a compositor binding
//!
//! On GNOME 50/51 — the first target — `org.freedesktop.portal.GlobalShortcuts`
//! is the only path an unprivileged application has to a global hotkey.
//! `compass-portals` documents the rest of that picture, including the
//! wlroots case where the interface does not exist at all.
//!
//! # A failure here never stops the engine
//!
//! No portal, a refused permission, or a compositor with no backend all end
//! with a warning and an engine that still answers `query`, `doctor` and an
//! explicit `vicinae toggle`. Refusing to start over a hotkey would make the
//! daemon unusable on exactly the machines that need the CLI most.

use std::sync::Arc;

use compass_ipc::{Response, WindowCommand};
use compass_portals::{
    PortalConfig, Portals, ShortcutDescriptor, ShortcutEvent, ShortcutsOutcome, Trigger,
};
use tokio::sync::RwLock;

use crate::serve::{EngineState, forward};

/// The shortcut id the engine registers, echoed back on every activation.
///
/// Stable across releases: the desktop stores the user's chosen trigger against
/// it, so changing this string would silently lose their binding.
pub const TOGGLE_ID: &str = "toggle";

/// What the user sees in their desktop's shortcut settings.
const TOGGLE_DESCRIPTION: &str = "Open the Compass launcher";

/// The trigger asked for, which the desktop may or may not grant.
///
/// The portal documents `preferred_trigger` as a *preference*: GNOME can assign
/// something else, or nothing. What we are actually bound to comes back in the
/// [`ShortcutsOutcome`] and is logged, because a launcher on a key the user did
/// not expect is worth one line in the journal.
/// Written in the portal's own spelling. `compass-portals` accepts `SUPER` and
/// `WIN` because users type them, but renders every trigger as `LOGO`, which is
/// what goes on the wire — so writing it the other way round would mean a
/// constant that does not match what the desktop is told.
const TOGGLE_TRIGGER: &str = "LOGO+space";

/// Acts on one shortcut event.
///
/// Returns `Some` with the engine's answer when the event was our activation,
/// and `None` when it was anything else. Split out from the loop so every
/// branch is testable without a portal — which matters more here than usual,
/// because the branches that must do *nothing* are the ones that break the
/// launcher when they do something.
pub async fn on_event(state: &Arc<RwLock<EngineState>>, event: ShortcutEvent) -> Option<Response> {
    match event {
        ShortcutEvent::Activated { id, .. } if id == TOGGLE_ID => {
            let slot = state.read().await.window_slot();
            Some(forward(&slot, WindowCommand::Toggle, "toggle a window").await)
        }

        // A key *release*. Toggling on this too would open the launcher on the
        // press and close it again on the release, which looks exactly like a
        // hotkey that does nothing.
        ShortcutEvent::Deactivated { .. } => None,

        // Some other application's shortcut on a shared session, or one we
        // registered later. Not ours to act on.
        ShortcutEvent::Activated { .. } => None,

        // The user re-bound something in system settings. Worth saying, because
        // "the hotkey stopped working" is otherwise a mystery.
        ShortcutEvent::Changed { shortcuts } => {
            for bound in shortcuts {
                tracing::info!(
                    id = %bound.id,
                    trigger = bound.trigger_description,
                    "the desktop changed a shortcut binding"
                );
            }
            None
        }

        // `ShortcutEvent` is `#[non_exhaustive]`, so this arm is required
        // rather than chosen. Ignoring an event this build does not know about
        // is the only safe reading: acting on it would mean guessing that some
        // future variant means "the user pressed the launcher key".
        other => {
            tracing::debug!(
                ?other,
                "ignoring a shortcut event this build does not handle"
            );
            None
        }
    }
}

/// Binds `Super+Space` over `xx-hotkey-v1` and forwards each trigger as a
/// toggle. `false` when the compositor does not offer the protocol or refuses
/// the trigger, so the caller can try the portal; `true` once a binding was
/// made, even after the compositor revokes it — the protocol leaves retrying
/// to the user.
async fn run_xx_hotkey(state: &Arc<RwLock<EngineState>>) -> bool {
    use compass_wayland::hotkey::{self, Hotkey, HotkeyError, HotkeyEvent, HotkeyRequest};

    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let request = HotkeyRequest {
        app_id: compass_ui::APP_ID.to_owned(),
        description: TOGGLE_DESCRIPTION.to_owned(),
        keysym: hotkey::KEYSYM_SPACE,
        modifiers: hotkey::modifiers::SUPER,
    };
    let bound = tokio::task::spawn_blocking(move || Hotkey::bind(&request, tx)).await;
    let _binding = match bound {
        Ok(Ok(binding)) => {
            tracing::info!(
                trigger = "Super+Space",
                "launcher hotkey bound over xx-hotkey-v1"
            );
            binding
        }
        Ok(Err(HotkeyError::Unsupported)) => return false,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "the compositor did not bind the launcher hotkey");
            return false;
        }
        Err(err) => {
            tracing::warn!(error = %err, "the hotkey task failed");
            return false;
        }
    };
    while let Some(event) = events.recv().await {
        match event {
            HotkeyEvent::Triggered { .. } => {
                let slot = state.read().await.window_slot();
                forward(&slot, WindowCommand::Toggle, "toggle a window").await;
            }
            HotkeyEvent::Revoked { message } => {
                tracing::warn!(
                    message,
                    "the compositor revoked the launcher hotkey; `vicinae toggle` still works"
                );
                break;
            }
        }
    }
    true
}

/// Binds the launcher hotkey and forwards activations for as long as it lives.
///
/// Returns once the portal session ends. Never returns an error: everything
/// that can go wrong here is degradation, and the caller has an engine to keep
/// running. What went wrong is logged at the level it deserves.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    // wlroots first: `xdg-desktop-portal-wlr` has no GlobalShortcuts, so the
    // portal below cannot help there. Never reached on GNOME.
    if let Some(wlroots) = crate::wlroots::detect().await {
        if run_xx_hotkey(&state).await {
            return;
        }
        let hint = compass_wayland::hotkey::manual_binding_hint(wlroots.desktop.as_deref());
        tracing::info!(
            "this compositor has no xx-hotkey-v1; trying the GlobalShortcuts portal, \
             and if that is missing too, {hint}"
        );
    }

    let portals = match Portals::connect(PortalConfig::default()).await {
        Ok(portals) => portals,
        Err(err) => {
            tracing::warn!(error = %err, "no portal connection; the launcher hotkey is unbound");
            return;
        }
    };

    let session = match portals.global_shortcuts().await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "no GlobalShortcuts portal; bind a hotkey to `vicinae toggle` in your \
                 compositor instead. `vicinae doctor` explains the options"
            );
            return;
        }
    };

    let mut descriptor = ShortcutDescriptor::new(TOGGLE_ID, TOGGLE_DESCRIPTION);
    if let Ok(trigger) = Trigger::parse(TOGGLE_TRIGGER) {
        descriptor = descriptor.with_trigger(trigger);
    } else {
        // A constant that no longer parses is a bug in this file, not a user
        // problem -- but binding with no preference still gives the user a
        // shortcut they can assign themselves, which beats no shortcut.
        tracing::error!(
            trigger = TOGGLE_TRIGGER,
            "the built-in trigger does not parse"
        );
    }

    match session.bind(&[descriptor]).await {
        Ok(ShortcutsOutcome::Granted { shortcuts }) => {
            for bound in &shortcuts {
                tracing::info!(
                    id = %bound.id,
                    trigger = bound.trigger_description,
                    "launcher hotkey bound"
                );
            }
        }
        Ok(outcome) => {
            tracing::warn!(
                ?outcome,
                "the desktop did not grant the launcher hotkey; `vicinae toggle` still works"
            );
            return;
        }
        Err(err) => {
            tracing::warn!(error = %err, "could not bind the launcher hotkey");
            return;
        }
    }

    let mut events = session.subscribe();
    while let Some(event) = events.recv().await {
        on_event(&state, event).await;
    }

    tracing::info!("the global shortcuts session ended; the launcher hotkey is unbound");
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use compass_core::{AppIndex, JsonFrecencyStore, SystemClock};
    use compass_ipc::{ErrorKind, SocketPath};

    /// An engine with no window attached.
    fn engine() -> Arc<RwLock<EngineState>> {
        let clock = Arc::new(SystemClock);
        Arc::new(RwLock::new(EngineState::with_index(
            AppIndex::builder().build(),
            Box::new(JsonFrecencyStore::in_memory(clock)),
            SocketPath::exact("/nonexistent/ipc.sock"),
            10,
        )))
    }

    fn activated(id: &str) -> ShortcutEvent {
        ShortcutEvent::Activated {
            id: id.to_owned(),
            timestamp: Duration::from_secs(1),
            activation_token: None,
        }
    }

    /// Whether the event reached the window path at all.
    ///
    /// With no window attached the engine refuses, so an `Unsupported` error is
    /// the signature of "this event was treated as our activation" and `None`
    /// is the signature of "it was ignored". Asserting on the refusal rather
    /// than on a window keeps these tests free of a socket.
    fn reached_the_window(response: Option<Response>) -> bool {
        matches!(
            response,
            Some(Response::Error(ref err)) if err.kind == ErrorKind::Unsupported
        )
    }

    #[tokio::test]
    async fn our_activation_reaches_the_window() {
        let state = engine();
        assert!(reached_the_window(
            on_event(&state, activated(TOGGLE_ID)).await
        ));
    }

    #[tokio::test]
    async fn another_applications_activation_is_ignored() {
        let state = engine();
        assert!(!reached_the_window(
            on_event(&state, activated("someone-elses-shortcut")).await
        ));
    }

    #[tokio::test]
    async fn releasing_the_key_does_not_toggle_again() {
        // The failure this prevents is invisible in a log and obvious to a
        // user: press toggles it open, release toggles it shut, and the hotkey
        // appears to do nothing at all.
        let state = engine();
        let released = ShortcutEvent::Deactivated {
            id: TOGGLE_ID.to_owned(),
            timestamp: Duration::from_secs(1),
        };
        assert!(!reached_the_window(on_event(&state, released).await));
    }

    #[tokio::test]
    async fn a_rebinding_notice_is_not_an_activation() {
        let state = engine();
        let changed = ShortcutEvent::Changed { shortcuts: vec![] };
        assert!(!reached_the_window(on_event(&state, changed).await));
    }

    #[test]
    fn the_built_in_trigger_is_super_plus_space() {
        // A constant string parsed at runtime, so nothing but this notices if
        // it stops meaning what it says. Asserted on the *parts* rather than on
        // a round trip: `Trigger::parse` passes the key name through verbatim
        // -- it deliberately does not validate against a keysym table -- so
        // `LOGO+spacebar` round-trips just as happily, and a control confirmed
        // an earlier round-trip version of this test did not catch that.
        let trigger = Trigger::parse(TOGGLE_TRIGGER).expect("the built-in trigger must parse");

        assert_eq!(trigger.modifiers, compass_portals::Modifiers::LOGO);
        assert_eq!(trigger.key, "space");
    }
}
