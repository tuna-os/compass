//! Own an engine only when this graphical session had to start it.

use std::io::ErrorKind;
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use compass_ipc::{Client, Request, Response, SocketPath};

/// Dropping a session stops only the child it started, never an existing engine.
#[derive(Debug)]
pub struct EngineSession(Option<Child>);

impl Drop for EngineSession {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn listening(socket: &SocketPath) -> Result<bool> {
    let mut client = match Client::connect(socket.as_path()).await {
        Ok(client) => client,
        Err(compass_ipc::Error::Io(error))
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    match client.request(Request::Ping).await? {
        Response::Pong { .. } => Ok(true),
        response => bail!("unexpected engine startup response: {response:?}"),
    }
}

/// Reuse a responsive engine or start `command`, waiting for its IPC readiness.
/// The caller configures the executable and environment; no shell is involved.
pub async fn ensure(
    socket: &SocketPath,
    command: &mut Command,
    timeout: Duration,
) -> Result<EngineSession> {
    tokio::time::timeout(timeout, async {
        if listening(socket).await? {
            return Ok(EngineSession(None));
        }
        let mut session = EngineSession(Some(
            command.spawn().context("starting the Compass engine")?,
        ));
        loop {
            if let Some(status) = session.0.as_mut().unwrap().try_wait()? {
                bail!("the Compass engine exited before becoming ready: {status}");
            }
            if listening(socket).await? {
                return Ok(session);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("timed out waiting for the Compass engine")?
}
