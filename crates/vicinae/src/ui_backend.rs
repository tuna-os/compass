//! Socket adapter for the UI's shared application search/history service.

use std::time::Duration;

use compass_ipc::{Request, SocketPath};
use compass_ui::backend::{ApplicationBackend, BackendFuture};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Uses the same engine/socket as the resident window link.
#[derive(Debug)]
pub struct DaemonBackend {
    socket: SocketPath,
}

impl DaemonBackend {
    /// Bind the adapter to a resolved socket without connecting yet.
    pub fn new(socket: SocketPath) -> Self {
        Self { socket }
    }
}

impl ApplicationBackend for DaemonBackend {
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            let hits =
                tokio::time::timeout(REQUEST_TIMEOUT, crate::ipc::query(&self.socket, &query))
                    .await
                    .map_err(|_| "Application search timed out".to_owned())?
                    .map_err(|error| error.to_string())?;
            Ok(hits.into_iter().map(|hit| hit.id).collect())
        })
    }

    fn record_launch(&self, key: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                crate::ipc::send_ack(&self.socket, Request::RecordLaunch { key }),
            )
            .await
            .map_err(|_| "Launch history update timed out".to_owned())?
            .map_err(|error| error.to_string())
        })
    }
}
