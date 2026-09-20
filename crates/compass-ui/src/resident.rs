//! The launcher window as a resident process.
//!
//! [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
//! made the window long-lived: it no longer exits when something is launched,
//! it hides, and an engine tells it when to come back. The 120 ms
//! summon-to-first-frame SLA is what forces this — a process started per summon
//! spends longer than that bringing up wgpu alone.
//!
//! # This module knows nothing about sockets
//!
//! The engine talks postcard over a Unix socket; none of that appears here.
//! What this crate takes is a pair of channels carrying [`UiCommand`] in and
//! [`UiOutcome`] out, and `vicinae` is the only place that knows those channels
//! are fed by `compass-ipc`. Same reason the launcher is injected rather than
//! reached for (ADR-0013): a UI crate that imports a transport can only be
//! tested with that transport running.
//!
//! # Hiding on Wayland means destroying the surface
//!
//! There is no "hide" in `xdg_toplevel`. A hidden window is a closed one, so
//! [`UiCommand::Hide`] closes the window and [`UiCommand::Show`] opens a new
//! one. What residency preserves is everything *around* the surface — the
//! process, the wgpu adapter, the font atlas, the application index — which is
//! where the seconds were going.
//!
//! How much of the summon cost that actually removes is **not measured yet**:
//! the VM tier times cold start to first frame, and show latency on a warm
//! process is a different number that nothing records. PLAN.md §8's SLA row
//! needs to split before either can be claimed.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// What an engine asks the resident window to do.
///
/// Mirrors `compass_ipc::WindowCommand` without depending on it, for the reason
/// in the module docs. `vicinae` converts between them in one place, and a test
/// there fails if the two ever disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCommand {
    /// Become visible and take focus.
    Show,
    /// Become hidden.
    Hide,
    /// Hide if visible, show if not.
    Toggle,
}

/// What the window reports back, as the state it ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiOutcome {
    /// The window is now visible.
    Shown,
    /// The window is now hidden.
    Hidden,
    /// The window could not carry the command out.
    Failed(String),
}

/// The window's end of the engine link: commands in, outcomes out.
///
/// # Why the receiver is behind an `Arc<Mutex<Option<…>>>`
///
/// Iced builds a subscription's stream from a plain `fn` pointer, which cannot
/// capture anything — so the receiver has to be reachable through the
/// subscription's *data*, which must be `Hash` and `Clone`. Taking it out of
/// the `Option` on first use gives the stream sole ownership without the data
/// itself having to be owned. A second take yields `None` and the subscription
/// ends, which is correct: there is only ever one engine link.
#[derive(Debug, Clone)]
pub struct EngineLink {
    /// Identity for Iced's subscription tracking. Two links are the same
    /// subscription only if this matches.
    id: u64,
    commands: Arc<Mutex<Option<mpsc::UnboundedReceiver<UiCommand>>>>,
    outcomes: mpsc::UnboundedSender<UiOutcome>,
}

impl EngineLink {
    /// Wraps a command receiver and an outcome sender.
    #[must_use]
    pub fn new(
        commands: mpsc::UnboundedReceiver<UiCommand>,
        outcomes: mpsc::UnboundedSender<UiOutcome>,
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            commands: Arc::new(Mutex::new(Some(commands))),
            outcomes,
        }
    }

    /// Reports what the window did.
    ///
    /// A closed channel is logged rather than propagated: the engine going away
    /// is not the window's failure, and there is nothing useful to do about it
    /// in the middle of an `update`.
    pub fn report(&self, outcome: UiOutcome) {
        if self.outcomes.send(outcome).is_err() {
            tracing::debug!("the engine is no longer listening for window outcomes");
        }
    }

    /// Takes the receiver, leaving `None` behind.
    ///
    /// A poisoned lock is treated as an empty slot rather than a panic: the
    /// only thing this lock guards is a one-shot handover, so there is no
    /// invariant left to violate.
    fn take_commands(&self) -> Option<mpsc::UnboundedReceiver<UiCommand>> {
        self.commands.lock().ok()?.take()
    }
}

impl Hash for EngineLink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl EngineLink {
    /// Commands followed by one `None` when the engine disconnects.
    /// The application chooses whether disconnection should end its session.
    pub fn subscription(&self) -> iced::Subscription<Option<UiCommand>> {
        iced::Subscription::run_with(self.clone(), |link| command_events(link.take_commands()))
    }
}

fn command_events(
    commands: Option<mpsc::UnboundedReceiver<UiCommand>>,
) -> impl iced::futures::Stream<Item = Option<UiCommand>> {
    iced::futures::stream::unfold(commands, |commands| async move {
        let mut commands = commands?;
        match commands.recv().await {
            Some(command) => Some((Some(command), Some(commands))),
            None => Some((None, None)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{StreamExt, executor::block_on};

    #[test]
    fn disconnect_is_delivered_once_after_queued_commands() {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender.send(UiCommand::Show).unwrap();
        drop(sender);
        let events = block_on(command_events(Some(receiver)).collect::<Vec<_>>());
        assert_eq!(events, vec![Some(UiCommand::Show), None]);
        assert!(block_on(command_events(None).collect::<Vec<_>>()).is_empty());
    }
}
