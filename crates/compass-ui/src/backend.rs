//! Application search and launch-history services supplied by the composition root.

use std::future::Future;
use std::pin::Pin;

/// An asynchronous backend operation, without a socket dependency in the UI.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The shared application catalog's ranking and successful-launch history.
pub trait ApplicationBackend: std::fmt::Debug + Send + Sync {
    /// Return stable application keys in presentation order.
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>>;

    /// Record an already successful launch; never execute the application again.
    fn record_launch(&self, key: String) -> BackendFuture<'_, ()>;
}
