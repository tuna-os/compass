//! Socket adapter for the UI's shared application search/history service.

use std::time::Duration;

use compass_ipc::{Request, SocketPath};
use compass_ui::backend::{
    ApplicationBackend, BackendFuture, ClipboardBackend, ClipboardContent, ClipboardRow,
    ClipboardRowKind, WindowBackend, WindowRow,
};

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

impl DaemonBackend {
    /// One request, with the engine's own sentence as the error on a refusal:
    /// the window shows it to the user, who does not need to be told that it
    /// came over a socket.
    async fn ask(&self, request: Request, what: &str) -> Result<compass_ipc::Response, String> {
        let exchange = async {
            let mut client = compass_ipc::Client::connect(self.socket.as_path())
                .await
                .map_err(|_| "The Compass engine is not running".to_owned())?;
            client
                .request(request)
                .await
                .map_err(|error| format!("Could not reach the engine: {error}"))
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, exchange).await {
            Err(_) => Err(format!("{what} timed out")),
            Ok(Err(message)) => Err(message),
            Ok(Ok(compass_ipc::Response::Error(error))) => Err(sentence(&error.message)),
            Ok(Ok(response)) => Ok(response),
        }
    }
}

/// The engine's message with its first letter capitalised, for display.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl ClipboardBackend for DaemonBackend {
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardHistory { query, limit },
                    "Clipboard history",
                )
                .await?
            {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardContent { id },
                    "Reading the clipboard entry",
                )
                .await?
            {
                compass_ipc::Response::ClipboardContent { mime_type, data } => {
                    Ok(ClipboardContent { mime_type, data })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::ClipboardPaste { id }, "Pasting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

impl WindowBackend for DaemonBackend {
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>> {
        Box::pin(async move {
            match self.ask(Request::ListWindows, "Listing windows").await? {
                compass_ipc::Response::Windows { windows } => Ok(windows
                    .into_iter()
                    .map(|window| WindowRow {
                        id: window.id,
                        app: window.app_name.unwrap_or_else(|| window.wm_class.clone()),
                        title: window.title,
                        wm_class: window.wm_class,
                        pid: window.pid,
                        can_close: window.can_close,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ActivateWindow { id }, "Switching windows")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseWindow { id }, "Closing a window")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn row(entry: compass_ipc::ClipboardEntry) -> ClipboardRow {
    ClipboardRow {
        id: entry.id,
        preview: entry.preview,
        kind: match entry.kind {
            compass_ipc::ClipboardKind::Text => ClipboardRowKind::Text,
            compass_ipc::ClipboardKind::Link => ClipboardRowKind::Link,
            compass_ipc::ClipboardKind::Image => ClipboardRowKind::Image,
            compass_ipc::ClipboardKind::File => ClipboardRowKind::File,
            compass_ipc::ClipboardKind::Unknown => ClipboardRowKind::Unknown,
        },
        pinned: entry.pinned,
        url_host: entry.url_host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_refusal_reads_as_a_sentence() {
        assert_eq!(
            sentence("clipboard history is unavailable: no keyring"),
            "Clipboard history is unavailable: no keyring"
        );
        assert_eq!(sentence(""), "");
    }
}
