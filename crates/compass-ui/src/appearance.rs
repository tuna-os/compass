//! The desktop's light/dark preference, as the window receives it.
//!
//! This crate must not know which desktop it is on (ADR-0013), and reading
//! `org.freedesktop.appearance` means a portal, a bus and `ashpd`. So the
//! preference arrives the same way engine commands do: over a channel, fed by
//! `vicinae`, which is the one place allowed to know that a portal is what is
//! on the other end. A test feeds it by hand.
//!
//! The initial value is *not* carried here. It is [`crate::AppFlags`]'s
//! `appearance`, because a window has to draw before any change can arrive and
//! something has to decide what it draws first.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::design::Appearance;

/// Distinguishes two links so Iced does not treat their subscriptions as one.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The window's end of the appearance channel.
#[derive(Debug, Clone)]
pub struct AppearanceLink {
    id: u64,
    updates: Arc<Mutex<Option<mpsc::UnboundedReceiver<Appearance>>>>,
}

/// The feeder's end.
pub type AppearanceSender = mpsc::UnboundedSender<Appearance>;

impl AppearanceLink {
    /// A new link, and the sender that drives it.
    #[must_use]
    pub fn new() -> (Self, AppearanceSender) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                updates: Arc::new(Mutex::new(Some(receiver))),
            },
            sender,
        )
    }

    /// Takes the receiver, leaving `None` behind.
    fn take_updates(&self) -> Option<mpsc::UnboundedReceiver<Appearance>> {
        self.updates.lock().ok()?.take()
    }

    /// A subscription yielding each new appearance.
    ///
    /// Ends when the feeder drops its sender, which leaves the window on
    /// whatever it last drew rather than reverting to a default. Losing the
    /// portal is not a reason to change colour.
    pub fn subscription(&self) -> iced::Subscription<Appearance> {
        iced::Subscription::run_with(self.clone(), |link| {
            let updates = link.take_updates();
            iced::futures::stream::unfold(updates, |updates| async move {
                let mut updates = updates?;
                let next = updates.recv().await?;
                Some((next, Some(updates)))
            })
        })
    }
}

impl Hash for AppearanceLink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn what_the_feeder_sends_is_what_the_window_receives() {
        let (link, sender) = AppearanceLink::new();
        sender.send(Appearance::Light).expect("send");
        let mut updates = link.take_updates().expect("the receiver is there once");
        assert_eq!(updates.recv().await, Some(Appearance::Light));
    }

    #[tokio::test]
    async fn the_receiver_is_handed_out_once() {
        // Iced may re-evaluate a subscription; handing the same receiver out
        // twice would give two streams sharing one channel, and each update
        // would reach exactly one of them.
        let (link, _sender) = AppearanceLink::new();
        assert!(link.take_updates().is_some());
        assert!(link.take_updates().is_none());
    }

    #[tokio::test]
    async fn a_dropped_feeder_ends_the_stream_rather_than_yielding_a_default() {
        let (link, sender) = AppearanceLink::new();
        drop(sender);
        let mut updates = link.take_updates().expect("receiver");
        assert_eq!(updates.recv().await, None);
    }

    #[test]
    fn two_links_do_not_collide() {
        use std::collections::hash_map::DefaultHasher;

        let (first, _a) = AppearanceLink::new();
        let (second, _b) = AppearanceLink::new();
        let hash = |link: &AppearanceLink| {
            let mut hasher = DefaultHasher::new();
            link.hash(&mut hasher);
            hasher.finish()
        };
        // Iced identifies a subscription by this hash. Two windows sharing one
        // would leave the second without updates.
        assert_ne!(hash(&first), hash(&second));
    }

    #[test]
    fn a_clone_is_the_same_link() {
        use std::collections::hash_map::DefaultHasher;

        let (link, _sender) = AppearanceLink::new();
        let clone = link.clone();
        let hash = |link: &AppearanceLink| {
            let mut hasher = DefaultHasher::new();
            link.hash(&mut hasher);
            hasher.finish()
        };
        // The subscription clones the link on every re-evaluation, and a clone
        // that hashed differently would restart the stream each time.
        assert_eq!(hash(&link), hash(&clone));
        assert!(link.take_updates().is_some());
        assert!(clone.take_updates().is_none(), "the receiver is shared");
    }
}
