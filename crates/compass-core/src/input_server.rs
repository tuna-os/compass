//! Talking to the snippet input server, and restarting it when it dies.
//!
//! A port of `InputServerBus` and `LinuxInputServer`'s lifecycle
//! (`src/server/src/services/input-server/`), minus the process and the
//! generated RPC client.
//!
//! # Two things here fail quietly when they are wrong
//!
//! A framing bug does not produce an error: it produces a stream that is
//! misaligned from the first bad length onward, so every later message is
//! garbage and none of them says why. And a backoff bug does not produce an
//! error either: it produces a process that is restarted in a tight loop, or
//! one that silently stops being restarted at all. Both are pinned here.

/// How many times the server is restarted before giving up.
pub const MAX_RESTART_ATTEMPTS: u32 = 5;

/// The first restart delay, in milliseconds; each attempt doubles it.
pub const BASE_RESTART_DELAY_MS: u64 = 1000;

/// The length prefix's width in bytes.
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Frame `payload` for the wire: a four-byte length, then the bytes.
///
/// # Native byte order, not network order
///
/// The C++ writes the `uint32_t` through a `reinterpret_cast`, so the prefix is
/// in the machine's own order — little-endian everywhere this runs. That is
/// *different* from the extension worker's protocol, which is big-endian, and
/// the two must not be confused: a host that framed this stream big-endian
/// would announce a 16-million-byte message for a 256-byte one and then wait
/// for ever.
#[must_use]
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut framed = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
    framed.extend_from_slice(&length.to_le_bytes());
    framed.extend_from_slice(payload);
    framed
}

/// Reassembles messages from a stream that arrives in arbitrary chunks.
#[derive(Debug, Clone, Default)]
pub struct MessageBuffer {
    /// Bytes read but not yet used.
    data: Vec<u8>,
}

impl MessageBuffer {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many bytes are held back waiting for the rest of a message.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.data.len()
    }

    /// Add `chunk` and take out every whole message it completed.
    ///
    /// A read can end anywhere — mid-payload, mid-prefix, or across several
    /// messages at once — so this returns however many are now complete,
    /// which may be none and may be several.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.data.extend_from_slice(chunk);
        let mut messages = Vec::new();

        loop {
            if self.data.len() < LENGTH_PREFIX_BYTES {
                break;
            }
            let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
            prefix.copy_from_slice(&self.data[..LENGTH_PREFIX_BYTES]);
            let length = u32::from_le_bytes(prefix) as usize;

            if self.data.len() - LENGTH_PREFIX_BYTES < length {
                break;
            }

            let end = LENGTH_PREFIX_BYTES + length;
            messages.push(self.data[LENGTH_PREFIX_BYTES..end].to_vec());
            self.data.drain(..end);
        }

        messages
    }
}

/// What a crash should lead to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashAction {
    /// Do nothing: the server is not wanted.
    Ignore,
    /// Restart after this many milliseconds.
    RestartAfter(u64),
    /// Stop trying.
    GiveUp,
}

/// The server's restart policy.
#[derive(Debug, Clone, Default)]
pub struct RestartPolicy {
    /// Whether the server is wanted at all.
    enabled: bool,
    /// How many times it has crashed since it was last ready.
    crash_count: u32,
}

impl RestartPolicy {
    /// A policy for a server that is not yet enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the server is wanted.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// How many crashes have been counted since the last ready.
    #[must_use]
    pub const fn crash_count(&self) -> u32 {
        self.crash_count
    }

    /// Turn the server on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Note that the server came up and answered.
    ///
    /// This resets the count, so a server that runs for a week and then
    /// crashes gets its five attempts again rather than inheriting the ones it
    /// used when it was first starting up.
    pub fn ready(&mut self) {
        self.crash_count = 0;
    }

    /// Decide what to do about a crash.
    ///
    /// A disabled server is not restarted and its crash is not counted — it
    /// was closed on purpose, and counting it would spend an attempt that a
    /// real crash later would need.
    pub fn handle_crash(&mut self) -> CrashAction {
        if !self.enabled {
            return CrashAction::Ignore;
        }

        self.crash_count += 1;

        if self.crash_count > MAX_RESTART_ATTEMPTS {
            return CrashAction::GiveUp;
        }

        CrashAction::RestartAfter(restart_delay_ms(self.crash_count))
    }
}

/// The delay before the `attempt`-th restart, counting from one.
///
/// Doubling, so five attempts span sixteen seconds rather than five: a server
/// failing because something else is not ready yet gets time for that to
/// change, and one failing because it cannot work at all stops quickly enough
/// to notice.
#[must_use]
pub fn restart_delay_ms(attempt: u32) -> u64 {
    BASE_RESTART_DELAY_MS * (1u64 << attempt.saturating_sub(1).min(31))
}

/// The largest message the server accepts: `MAX_MESSAGE_SIZE` in
/// `src/snippet/src/server.cpp`. A longer announced length closes the stream.
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;

impl MessageBuffer {
    /// The length the held partial message announces, once its prefix is in.
    ///
    /// The server compares this against [`MAX_MESSAGE_SIZE`] before waiting
    /// for the rest: a peer announcing four gigabytes is not waited for.
    #[must_use]
    pub fn announced(&self) -> Option<usize> {
        let prefix: [u8; LENGTH_PREFIX_BYTES] =
            self.data.get(..LENGTH_PREFIX_BYTES)?.try_into().ok()?;
        Some(u32::from_le_bytes(prefix) as usize)
    }
}

pub mod expansion;
pub mod wire;
