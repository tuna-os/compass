//! `compass.json` as the window receives it after a change on disk.
//!
//! Watching a file is the binary's business, like the portal behind
//! [`crate::appearance`]: `compass` watches the file and sends each newly
//! read configuration here, and the window applies what it holds of it
//! (`LauncherApp::apply_reloaded_config`). A test feeds it by hand.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use compass_core::Config;
use tokio::sync::mpsc;

/// Distinguishes two links so Iced does not treat their subscriptions as one.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The window's end of the configuration channel.
#[derive(Debug, Clone)]
pub struct ConfigLink {
    id: u64,
    updates: Arc<Mutex<Option<mpsc::UnboundedReceiver<Arc<Config>>>>>,
}

/// The feeder's end.
pub type ConfigSender = mpsc::UnboundedSender<Arc<Config>>;

impl ConfigLink {
    /// A new link, and the sender that drives it.
    #[must_use]
    pub fn new() -> (Self, ConfigSender) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                updates: Arc::new(Mutex::new(Some(receiver))),
            },
            sender,
        )
    }

    fn take_updates(&self) -> Option<mpsc::UnboundedReceiver<Arc<Config>>> {
        self.updates.lock().ok()?.take()
    }

    /// A subscription yielding each configuration read after a change.
    pub fn subscription(&self) -> iced::Subscription<Arc<Config>> {
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

impl Hash for ConfigLink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn what_the_watcher_sends_is_what_the_window_receives_once() {
        let (link, sender) = ConfigLink::new();
        sender.send(Arc::new(Config::default())).expect("send");
        let mut updates = link.take_updates().expect("the receiver is there once");
        assert!(link.take_updates().is_none());
        assert_eq!(updates.recv().await.as_deref(), Some(&Config::default()));
    }
}
