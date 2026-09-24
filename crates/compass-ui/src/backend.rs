//! Application search and launch-history services supplied by the composition root.

use std::future::Future;
use std::pin::Pin;

/// An asynchronous backend operation, without a socket dependency in the UI.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The shared application catalog's ranking and successful-launch history.
pub trait ApplicationBackend: std::fmt::Debug + Send + Sync {
    /// Return root-item ENTRYPOINT ids in presentation order.
    ///
    /// `applications:org.mozilla.firefox`, not the launch key
    /// `org.mozilla.firefox.desktop` — this is `QueryHit.id` straight off the
    /// wire, and `AppIndex::position_by_entrypoint` is what resolves it.
    /// `record_launch` below still takes the KEY, because the two ids are
    /// different things and the frecency store is keyed by the launchable.
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>>;

    /// Record an already successful launch; never execute the application again.
    fn record_launch(&self, key: String) -> BackendFuture<'_, ()>;
}
