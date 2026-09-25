//! A stand-in for a compositor's IPC socket, replaying canned replies.
//!
//! Hyprland's request socket and niri's socket are both "connect, write one
//! request, read the answer": Hyprland then closes, niri answers one line. A
//! [`FakeSocket`] listens on a path in a temporary directory, answers each
//! request from a table of replies, and records what it was asked, so a test
//! can check both sides without a compositor anywhere near it.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How a request ends and a reply is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// Hyprland: the request is whatever arrives first; the reply is written
    /// and the connection closed.
    Hyprland,
    /// niri: the request is one JSON line; so is the reply.
    Niri,
}

type Answer = dyn Fn(&str) -> String + Send + Sync;

/// A fake compositor socket. Stops listening when dropped.
pub struct FakeSocket {
    path: PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for FakeSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSocket")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl FakeSocket {
    /// Listens at `path`, answering each request with `answer(request)`.
    ///
    /// # Panics
    ///
    /// When the socket cannot be bound.
    pub fn serve(
        path: &Path,
        framing: Framing,
        answer: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Self {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the fake socket's directory");
        }
        let listener = UnixListener::bind(path).expect("bind the fake compositor socket");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let answer: Arc<Answer> = Arc::new(answer);
        let thread = std::thread::spawn({
            let (seen, stop) = (Arc::clone(&seen), Arc::clone(&stop));
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let request = match framing {
                                Framing::Hyprland => {
                                    let mut buf = [0_u8; 8192];
                                    let read = (&stream).read(&mut buf).unwrap_or(0);
                                    String::from_utf8_lossy(&buf[..read]).into_owned()
                                }
                                Framing::Niri => {
                                    let mut line = String::new();
                                    let _ = BufReader::new(&stream).read_line(&mut line);
                                    line.trim_end().to_owned()
                                }
                            };
                            let mut reply = answer(&request);
                            if framing == Framing::Niri && !reply.ends_with('\n') {
                                reply.push('\n');
                            }
                            seen.lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .push(request);
                            let _ = (&stream).write_all(reply.as_bytes());
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        Self {
            path: path.to_path_buf(),
            seen,
            stop,
            thread: Some(thread),
        }
    }

    /// Listens at `path`, answering a request that starts with a key of
    /// `replies` with its value, and anything else with `unknown request`.
    #[must_use]
    pub fn replaying(path: &Path, framing: Framing, replies: Vec<(String, String)>) -> Self {
        Self::serve(path, framing, move |request| {
            replies
                .iter()
                .find(|(prefix, _)| request.starts_with(prefix.as_str()))
                .map_or_else(|| "unknown request".to_owned(), |(_, reply)| reply.clone())
        })
    }

    /// The Hyprland request socket for `signature` under `runtime_dir`.
    #[must_use]
    pub fn hyprland_path(runtime_dir: &Path, signature: &str) -> PathBuf {
        runtime_dir
            .join("hypr")
            .join(signature)
            .join(".socket.sock")
    }

    /// Where it listens.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every request so far, in order.
    #[must_use]
    pub fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for FakeSocket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
