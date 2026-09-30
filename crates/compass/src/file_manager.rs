//! `org.freedesktop.FileManager1`: showing a file in the file manager with
//! it selected, as the C++'s `showInFileBrowser` does on Linux.

use std::time::Duration;

/// How long the file manager may take to answer; Nautilus is activated on
/// demand, so its first call can be slow.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

#[zbus::proxy(
    interface = "org.freedesktop.FileManager1",
    default_service = "org.freedesktop.FileManager1",
    default_path = "/org/freedesktop/FileManager1"
)]
trait FileManager1 {
    /// Opens the folders containing `uris` with the items selected.
    fn show_items(&self, uris: &[&str], startup_id: &str) -> zbus::Result<()>;
}

/// Why the file manager did not show the item.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No session bus, or the call failed (no file manager implements it).
    #[error(transparent)]
    Bus(#[from] zbus::Error),
    /// It did not answer in time.
    #[error("the file manager did not answer within {0:?}")]
    Timeout(Duration),
}

/// Asks the session's file manager to show `uri` selected.
///
/// # Errors
///
/// [`Error`] when there is no session bus, nothing owns the name or it
/// refuses, or it does not answer in time; the caller then opens the folder.
pub async fn show_items(uri: &str) -> Result<(), Error> {
    let connection = zbus::Connection::session().await?;
    let proxy = FileManager1Proxy::new(&connection).await?;
    tokio::time::timeout(CALL_TIMEOUT, proxy.show_items(&[uri], ""))
        .await
        .map_err(|_| Error::Timeout(CALL_TIMEOUT))??;
    Ok(())
}
