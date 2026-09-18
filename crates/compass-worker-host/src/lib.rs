//! The host side of the extension worker boundary: framing, for now.
//!
//! # Why this crate starts with framing and nothing else
//!
//! Phase 4 is costed at six to eight weeks and the wire format is the first
//! thing a host commits to, so it is the first thing that should be pinned. It
//! is also the one piece that survives every open design decision in #101: a
//! host that speaks figura, a host that speaks something else, and a rewritten
//! worker all still have to agree where one message ends and the next begins.
//!
//! # The format is the worker's, not the plan's
//!
//! PLAN.md §6 says Phase 4 spawns the worker *"over UDS with JSON-RPC 2.0"*.
//! The worker does neither, and §11.4a and #101 record the discrepancy. What
//! `src/typescript/extension-manager/src/index.ts` actually writes is:
//!
//! ```ts
//! const packet = Buffer.allocUnsafe(message.length + 4);
//! packet.writeUint32BE(message.length, 0);
//! message.copy(packet, 4, 0);
//! process.stdout.write(packet);
//! ```
//!
//! A four-byte big-endian length, then that many bytes, over stdio. Inbound it
//! reads a `UInt32BE`, waits until that many bytes have arrived, and slices.
//! That is the whole contract at this layer; what the payload *means* is
//! `figura/manager.fig`'s business, and the worker's own header is explicit
//! that the manager is "unaware what the payload is made of".
//!
//! So this crate deliberately knows nothing about the payload. It is a length
//! codec, and `cpp_framing` pins it against the TypeScript rather than
//! restating it.

use thiserror::Error;

/// The length prefix is four bytes, big-endian.
pub const LENGTH_PREFIX: usize = 4;

/// The largest frame this host will accept.
///
/// A length prefix read from a pipe is attacker-controlled in the same sense
/// any parsed input is: a worker that has gone wrong — or been replaced — can
/// claim four gigabytes and make the host allocate it. The worker writes
/// `Buffer.allocUnsafe(message.length + 4)` with no ceiling of its own, so the
/// ceiling has to live here.
///
/// 64 MiB is far above any real extension payload and far below a number that
/// hurts. It is a guard, not a tuning parameter: if a legitimate payload ever
/// approaches it, the fix is chunking at a higher layer, not a bigger constant.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// What can go wrong reading a frame.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    /// The declared length exceeds [`MAX_FRAME`].
    #[error("frame declares {declared} bytes, over the {MAX_FRAME}-byte limit")]
    TooLarge {
        /// The length the prefix claimed.
        declared: usize,
    },
}

/// Encodes `payload` as one frame: a four-byte big-endian length, then the bytes.
///
/// # Errors
///
/// [`FrameError::TooLarge`] if `payload` is longer than [`MAX_FRAME`]. Encoding
/// is checked as well as decoding so the host cannot emit a frame it would
/// refuse to read back — an asymmetry there is how one side ends up able to
/// wedge the other.
pub fn encode(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_FRAME {
        return Err(FrameError::TooLarge {
            declared: payload.len(),
        });
    }

    // `as u32` is sound because MAX_FRAME is well under u32::MAX and the length
    // was just checked against it.
    let len = u32::try_from(payload.len()).expect("checked against MAX_FRAME above");

    let mut out = Vec::with_capacity(LENGTH_PREFIX + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// One decoded frame and how many bytes of the buffer it consumed.
#[derive(Debug, PartialEq, Eq)]
pub struct Decoded<'a> {
    /// The payload, borrowed from the input.
    pub payload: &'a [u8],
    /// Total bytes consumed, including the length prefix.
    pub consumed: usize,
}

/// Decodes the first frame in `buf`, if a whole one has arrived.
///
/// Returns `Ok(None)` when the buffer holds a partial frame — the ordinary case
/// on a pipe, where a `read` returns whatever happened to be available. The
/// caller keeps the unconsumed bytes and calls again, which is exactly the loop
/// the worker runs on its own side.
///
/// # Errors
///
/// [`FrameError::TooLarge`] if the prefix declares more than [`MAX_FRAME`]. This
/// is returned *before* waiting for the bytes, so an absurd length fails
/// immediately rather than after the host has sat waiting for four gigabytes
/// that will never come.
pub fn decode(buf: &[u8]) -> Result<Option<Decoded<'_>>, FrameError> {
    let Some(header) = buf.get(..LENGTH_PREFIX) else {
        return Ok(None);
    };

    let declared = u32::from_be_bytes(
        header
            .try_into()
            .expect("the slice is exactly LENGTH_PREFIX bytes"),
    ) as usize;

    if declared > MAX_FRAME {
        return Err(FrameError::TooLarge { declared });
    }

    let end = LENGTH_PREFIX + declared;
    match buf.get(LENGTH_PREFIX..end) {
        Some(payload) => Ok(Some(Decoded {
            payload,
            consumed: end,
        })),
        // The length is plausible but the body has not all arrived yet.
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let framed = encode(b"hello").expect("encode");
        assert_eq!(&framed[..LENGTH_PREFIX], &[0, 0, 0, 5]);

        let got = decode(&framed).expect("decode").expect("a whole frame");
        assert_eq!(got.payload, b"hello");
        assert_eq!(got.consumed, LENGTH_PREFIX + 5);
    }

    #[test]
    fn an_empty_payload_is_a_frame_rather_than_an_absence() {
        // A zero-length message is legitimate and must not read as "nothing has
        // arrived", or the caller loops for ever on a frame it already has.
        let framed = encode(b"").expect("encode");
        assert_eq!(framed, vec![0, 0, 0, 0]);

        let got = decode(&framed).expect("decode").expect("a whole frame");
        assert!(got.payload.is_empty());
        assert_eq!(got.consumed, LENGTH_PREFIX);
    }

    #[test]
    fn a_partial_frame_is_not_an_error() {
        // The ordinary case on a pipe: `read` returns what happened to be there.
        let framed = encode(b"hello").expect("encode");
        for cut in 0..framed.len() {
            assert_eq!(
                decode(&framed[..cut]).expect("a short buffer is not an error"),
                None,
                "{cut} bytes should read as incomplete"
            );
        }
        assert!(decode(&framed).expect("decode").is_some());
    }

    #[test]
    fn frames_are_decoded_one_at_a_time_from_a_coalesced_read() {
        // Two writes on the worker's side can arrive as one read on ours. The
        // decoder has to hand back the first and say where the second starts.
        let mut stream = encode(b"first").expect("encode");
        stream.extend_from_slice(&encode(b"second").expect("encode"));

        let one = decode(&stream).expect("decode").expect("first frame");
        assert_eq!(one.payload, b"first");

        let two = decode(&stream[one.consumed..])
            .expect("decode")
            .expect("second frame");
        assert_eq!(two.payload, b"second");
        assert_eq!(one.consumed + two.consumed, stream.len());
    }

    #[test]
    fn an_absurd_length_is_refused_before_the_body_is_waited_for() {
        // u32::MAX declared, four bytes present. Without the ceiling this reads
        // as "incomplete" and the host waits for 4 GiB that will never arrive.
        let mut buf = vec![0xff, 0xff, 0xff, 0xff];
        buf.extend_from_slice(b"short");

        assert_eq!(
            decode(&buf),
            Err(FrameError::TooLarge {
                declared: u32::MAX as usize
            })
        );
    }

    #[test]
    fn the_largest_allowed_frame_is_accepted_and_one_more_byte_is_not() {
        // The boundary itself, both sides of it. A ceiling that is off by one is
        // a ceiling nobody has tested.
        let mut at_limit = (MAX_FRAME as u32).to_be_bytes().to_vec();
        at_limit.resize(LENGTH_PREFIX + MAX_FRAME, 0);
        assert!(
            decode(&at_limit)
                .expect("exactly MAX_FRAME is allowed")
                .is_some(),
            "MAX_FRAME itself must be accepted"
        );

        let over = (MAX_FRAME as u32 + 1).to_be_bytes().to_vec();
        assert_eq!(
            decode(&over),
            Err(FrameError::TooLarge {
                declared: MAX_FRAME + 1
            })
        );
    }

    #[test]
    fn encoding_refuses_what_decoding_would_refuse() {
        // An asymmetry here is how one side emits a frame the other cannot read.
        let too_big = vec![0u8; MAX_FRAME + 1];
        assert_eq!(
            encode(&too_big),
            Err(FrameError::TooLarge {
                declared: MAX_FRAME + 1
            })
        );
    }
}

#[cfg(test)]
mod cpp_framing {
    //! The framing is read back out of the TypeScript worker.
    //!
    //! WHY THIS IS NOT PARANOIA
    //!
    //! This crate's whole contract is "four bytes, big-endian, then that many".
    //! Nothing but a comment tied that to the worker, and the worker is the
    //! party that has to agree — PLAN.md's own description of this boundary is
    //! wrong in both transport and encoding (§11.4a, #101), so the document is
    //! demonstrably not a safe source for it.
    //!
    //! A silent change on the worker's side — `writeUint32LE`, or a two-byte
    //! prefix — would not fail any test here. It would fail at runtime, as a
    //! host that blocks for ever on a length it misread.
    //!
    //! So the two facts this crate depends on are parsed out of the worker:
    //! that the prefix is written big-endian, and that it is read back the same
    //! way. Endianness is the sharp one, because on a little-endian machine a
    //! short message with the wrong byte order still decodes to *a* number.
    //!
    //! WHAT IT CANNOT DO
    //!
    //! It greps for the calls; it does not evaluate the TypeScript. A worker
    //! that framed its messages somewhere else entirely would still pass.

    use super::LENGTH_PREFIX;
    use std::path::{Path, PathBuf};

    const WORKER: &str = "src/typescript/extension-manager/src/index.ts";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/compass-worker-host sits two levels below the repository root")
            .to_path_buf()
    }

    #[test]
    fn the_worker_still_frames_big_endian_on_both_directions() {
        let path = repo_root().join(WORKER);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        assert!(
            text.contains("writeUint32BE"),
            "{WORKER} no longer writes a big-endian u32 length prefix. This crate's \
             decoder reads one, so the two have diverged and the host will block on a \
             length it misread rather than fail."
        );
        assert!(
            text.contains("readUInt32BE"),
            "{WORKER} no longer reads a big-endian u32 length prefix, so frames this \
             crate encodes will be misread on the worker's side."
        );

        // The prefix width is the other half of the contract, and it appears in
        // the worker as the +4 / subarray(4, ...) arithmetic rather than as a
        // named constant. Asserting the allocation is the closest honest proxy.
        assert!(
            text.contains("message.length + 4"),
            "{WORKER} no longer allocates a 4-byte prefix; LENGTH_PREFIX is {LENGTH_PREFIX} here."
        );
    }
}

/// Reads whole frames from a byte stream.
///
/// The worker writes to a pipe, so a `read` returns whatever happened to be
/// there: half a frame, three frames, or a frame split across two reads. This
/// holds the leftover and hands back one payload at a time, which is the same
/// loop `index.ts` runs on its own side.
///
/// It is generic over [`std::io::Read`] rather than taking a child's stdout
/// directly, so the tests drive it with an in-memory stream that can reproduce
/// an awkward split exactly. A reader that can only be tested against a real
/// process is a reader whose partial-read path is never exercised.
#[derive(Debug)]
pub struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
    /// How much of `buf` has been consumed by frames already returned.
    start: usize,
}

/// Why reading a frame stopped.
#[derive(Debug, Error)]
pub enum ReadError {
    /// The stream ended part-way through a frame.
    ///
    /// Distinguished from a clean end deliberately: a worker that exits between
    /// writing a length and writing the body has crashed, and reporting that as
    /// "no more messages" would turn a crash into silence.
    #[error("stream ended with {have} bytes of an incomplete frame")]
    Truncated {
        /// Bytes left over.
        have: usize,
    },

    /// The frame was malformed.
    #[error(transparent)]
    Frame(#[from] FrameError),

    /// The underlying stream failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl<R: std::io::Read> FrameReader<R> {
    /// Wraps a stream.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            start: 0,
        }
    }

    /// Reads the next whole frame, or `None` at a clean end of stream.
    ///
    /// # Errors
    ///
    /// [`ReadError::Truncated`] if the stream ends mid-frame,
    /// [`ReadError::Frame`] if a length prefix is absurd, and
    /// [`ReadError::Io`] if the stream itself fails.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ReadError> {
        loop {
            // `decode` borrows, so the payload is copied out before the buffer
            // is touched. Frames are small and this keeps the borrow checker
            // out of the control flow, which matters more here than the copy.
            if let Some(found) = decode(&self.buf[self.start..])? {
                let payload = found.payload.to_vec();
                self.start += found.consumed;
                return Ok(Some(payload));
            }

            // Reclaim, rather than growing for ever on a long-lived worker.
            if self.start > 0 {
                self.buf.drain(..self.start);
                self.start = 0;
            }

            let mut chunk = [0u8; 8192];
            let n = self.inner.read(&mut chunk)?;
            if n == 0 {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(ReadError::Truncated {
                        have: self.buf.len(),
                    })
                };
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    /// A stream that hands back exactly the chunks it was given, so a test can
    /// reproduce a specific awkward split rather than hoping for one.
    struct Chunks {
        chunks: Vec<Vec<u8>>,
        at: usize,
    }

    impl std::io::Read for Chunks {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.at >= self.chunks.len() {
                return Ok(0);
            }
            let chunk = &self.chunks[self.at];
            self.at += 1;
            let n = chunk.len().min(out.len());
            out[..n].copy_from_slice(&chunk[..n]);
            Ok(n)
        }
    }

    fn reader(chunks: Vec<Vec<u8>>) -> FrameReader<Chunks> {
        FrameReader::new(Chunks { chunks, at: 0 })
    }

    #[test]
    fn frames_split_across_reads_are_reassembled() {
        let framed = encode(b"a message").expect("encode");
        // Split inside the length prefix AND inside the body: both halves of
        // the partial path in one case.
        let mut r = reader(vec![
            framed[..2].to_vec(),
            framed[2..6].to_vec(),
            framed[6..].to_vec(),
        ]);
        assert_eq!(
            r.next_frame().expect("read").as_deref(),
            Some(&b"a message"[..])
        );
        assert_eq!(r.next_frame().expect("read"), None);
    }

    #[test]
    fn several_frames_in_one_read_come_back_one_at_a_time() {
        let mut both = encode(b"first").expect("encode");
        both.extend_from_slice(&encode(b"second").expect("encode"));
        let mut r = reader(vec![both]);

        assert_eq!(
            r.next_frame().expect("read").as_deref(),
            Some(&b"first"[..])
        );
        assert_eq!(
            r.next_frame().expect("read").as_deref(),
            Some(&b"second"[..])
        );
        assert_eq!(r.next_frame().expect("read"), None);
    }

    #[test]
    fn a_clean_end_of_stream_is_not_an_error() {
        let mut r = reader(vec![]);
        assert!(r.next_frame().expect("a clean end is Ok(None)").is_none());
    }

    #[test]
    fn a_worker_that_dies_mid_frame_is_an_error_not_a_silence() {
        // THE DISTINCTION THAT MATTERS. A worker killed between writing its
        // length and its body would otherwise look exactly like one that
        // finished, and the host would carry on with a missing reply.
        let framed = encode(b"never finished").expect("encode");
        let mut r = reader(vec![framed[..6].to_vec()]);

        match r.next_frame() {
            Err(ReadError::Truncated { have }) => assert_eq!(have, 6),
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn an_absurd_length_surfaces_as_an_error_rather_than_a_hang() {
        let mut buf = vec![0xff, 0xff, 0xff, 0xff];
        buf.extend_from_slice(b"short");
        let mut r = reader(vec![buf]);

        match r.next_frame() {
            Err(ReadError::Frame(FrameError::TooLarge { declared })) => {
                assert_eq!(declared, u32::MAX as usize);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn the_buffer_does_not_grow_without_bound_across_many_frames() {
        // A long-lived worker sends thousands of messages down one pipe. If the
        // consumed prefix is never reclaimed the host's memory tracks total
        // bytes ever received rather than the largest frame.
        let chunks: Vec<Vec<u8>> = (0..500)
            .map(|i| encode(format!("message {i}").as_bytes()).expect("encode"))
            .collect();
        let mut r = reader(chunks);

        for i in 0..500 {
            let got = r.next_frame().expect("read").expect("a frame");
            assert_eq!(got, format!("message {i}").into_bytes());
            assert!(
                r.buf.len() < 1024,
                "buffer grew to {} bytes by frame {i}; the consumed prefix is not being reclaimed",
                r.buf.len()
            );
        }
    }
}

/// The JSON-RPC 2.0 envelope the worker speaks inside each frame.
///
/// # Read from the generator, not from the plan
///
/// PLAN.md describes this boundary and is wrong about the transport; an earlier
/// revision of §11.4a was then wrong about the encoding, by reading the import
/// and inferring what it generated. So the shapes here are taken from
/// `src/lib/figura/src/codegen/typescript.hpp`, which is what actually emits
/// the worker's side:
///
/// ```ts
/// this.sendMessage({ jsonrpc: '2.0', method, params });          // event
/// this.sendMessage({ jsonrpc: '2.0', id, method, params });      // request
/// this.sendMessage({ jsonrpc: '2.0', id, result });              // response
/// if (msg.id === undefined && msg.method) { /* ... an event */ }
/// ```
///
/// Two details that a reasonable guess would get wrong, and which are therefore
/// worth stating: **params is a named object, not a positional array** — the
/// codegen builds `{ paramName: value, ... }` — and the method string is
/// `"<Service>/<method>"`, from
/// `std::format("{}/{}", s.name, method.name)`.
pub mod extension_manager;
pub mod tsapi;

pub mod rpc {
    use serde::{Deserialize, Serialize};

    /// The only `jsonrpc` value this protocol uses.
    pub const VERSION: &str = "2.0";

    /// A message from the worker: a response to one of our requests, or an
    /// event it raised on its own.
    ///
    /// The worker distinguishes them by the presence of `id`, so this does too
    /// rather than by which fields happen to deserialise.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Incoming {
        /// Always [`VERSION`].
        pub jsonrpc: String,
        /// Present on a response, absent on an event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<u64>,
        /// Present on an event, absent on a response.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub method: Option<String>,
        /// An event's arguments.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub params: Option<serde_json::Value>,
        /// A response's value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub result: Option<serde_json::Value>,
    }

    impl Incoming {
        /// Whether this is an event rather than a response.
        ///
        /// Mirrors the worker's own test, `msg.id === undefined && msg.method`.
        /// A message with neither is malformed and is not an event.
        #[must_use]
        pub fn is_event(&self) -> bool {
            self.id.is_none() && self.method.is_some()
        }
    }

    /// A request to the worker.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Request {
        /// Always [`VERSION`].
        pub jsonrpc: String,
        /// Correlates the response.
        pub id: u64,
        /// `"<Service>/<method>"`.
        pub method: String,
        /// Named arguments, keyed by parameter name.
        pub params: serde_json::Value,
    }

    impl Request {
        /// Builds a request for `method` with named `params`.
        #[must_use]
        pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
            Self {
                jsonrpc: VERSION.to_owned(),
                id,
                method: method.into(),
                params,
            }
        }
    }

    /// The `Manager` service's methods, as they appear on the wire.
    ///
    /// Pinned to `figura/manager.fig` by `fig_methods`, because a method name
    /// that has drifted produces `No handler for method ...` at runtime and
    /// nothing earlier.
    pub mod manager {
        /// Load an extension. Returns a session id.
        pub const LOAD: &str = "Manager/load";
        /// Unload a session.
        pub const UNLOAD: &str = "Manager/unload";
        /// Tell the worker we are ready to receive that session's messages.
        ///
        /// `manager.fig` explains why this exists: without it the host can miss
        /// an extension's first messages if it loads faster than the host
        /// stores the session id.
        pub const READY: &str = "Manager/ready";
        /// Forward a payload to an extension.
        pub const MESSAGE_EXTENSION: &str = "Manager/messageExtension";

        /// An extension sent us something.
        pub const EVENT_EXTENSION_MESSAGE: &str = "Manager/extensionMessage";
        /// An extension died.
        pub const EVENT_EXTENSION_CRASH: &str = "Manager/extensionCrash";
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::rpc::{self, manager};
    use serde_json::json;

    #[test]
    fn a_response_is_not_an_event() {
        let msg: rpc::Incoming =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"result":{"session_id":"s"}}"#)
                .expect("parse");
        assert!(!msg.is_event());
        assert_eq!(msg.id, Some(7));
        assert_eq!(msg.result, Some(json!({"session_id": "s"})));
    }

    #[test]
    fn an_event_has_a_method_and_no_id() {
        let msg: rpc::Incoming = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"Manager/extensionMessage","params":{"session_id":"s","payload":"x"}}"#,
        )
        .expect("parse");
        assert!(msg.is_event());
        assert_eq!(
            msg.method.as_deref(),
            Some(manager::EVENT_EXTENSION_MESSAGE)
        );
    }

    #[test]
    fn a_message_with_neither_id_nor_method_is_not_an_event() {
        // The worker's own test is `id === undefined && method`. A malformed
        // message must not be routed as an event with no name.
        let msg: rpc::Incoming = serde_json::from_str(r#"{"jsonrpc":"2.0"}"#).expect("parse");
        assert!(!msg.is_event());
    }

    #[test]
    fn a_request_serialises_with_named_params() {
        // Positional params would be accepted by JSON-RPC generally and
        // rejected by this worker: the codegen builds `{ name: value }`.
        let req = rpc::Request::new(1, manager::UNLOAD, json!({ "session_id": "abc" }));
        let text = serde_json::to_string(&req).expect("serialise");

        let back: serde_json::Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(back["jsonrpc"], "2.0");
        assert_eq!(back["method"], "Manager/unload");
        assert_eq!(back["params"]["session_id"], "abc");
        assert!(
            back["params"].is_object(),
            "params must be a named object, not an array: {text}"
        );
    }
}

#[cfg(test)]
mod fig_methods {
    //! The `Manager` method names are read back out of `figura/manager.fig`.
    //!
    //! A drifted method name fails at runtime as the worker's
    //! `No handler for method ${msg.method}` and nothing earlier, so it is
    //! worth catching here. The `"<Service>/<method>"` shape is not a guess:
    //! `src/lib/figura/src/codegen/typescript.hpp` builds it with
    //! `std::format("{}/{}", s.name, method.name)`.
    //!
    //! WHAT IT CANNOT DO
    //!
    //! It reads the IDL, not the generated code, so a change to the codegen's
    //! naming scheme would pass here and break at runtime. The scheme is
    //! asserted separately, by reading the format string out of the codegen.

    use super::rpc::manager;
    use std::path::{Path, PathBuf};

    const FIG: &str = "figura/manager.fig";
    const CODEGEN: &str = "src/lib/figura/src/codegen/typescript.hpp";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/compass-worker-host sits two levels below the repository root")
            .to_path_buf()
    }

    fn read(rel: &str) -> String {
        let path = repo_root().join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    #[test]
    fn the_wire_names_are_service_slash_method() {
        assert!(
            read(CODEGEN).contains(r#"std::format("{}/{}", s.name, method.name)"#),
            "{CODEGEN} no longer builds wire method names as `<Service>/<method>`; every \
             constant in `rpc::manager` is then wrong."
        );
    }

    #[test]
    fn every_manager_method_in_the_idl_has_a_constant() {
        let fig = read(FIG);
        let service = fig
            .split("service Manager {")
            .nth(1)
            .unwrap_or_else(|| panic!("{FIG} no longer declares `service Manager`"));
        let body = service
            .split('}')
            .next()
            .expect("the service block is not closed");

        let mut declared: Vec<String> = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            let Some(rest) = line
                .strip_prefix("fn ")
                .or_else(|| line.strip_prefix("event "))
            else {
                continue;
            };
            let Some(name) = rest.split('(').next() else {
                continue;
            };
            declared.push(format!("Manager/{}", name.trim()));
        }

        assert!(
            declared.len() >= 6,
            "parsed only {} methods from {FIG}; the parser has drifted from the IDL's shape \
             and is no longer checking anything: {declared:?}",
            declared.len()
        );

        let known = [
            manager::LOAD,
            manager::UNLOAD,
            manager::READY,
            manager::MESSAGE_EXTENSION,
            manager::EVENT_EXTENSION_MESSAGE,
            manager::EVENT_EXTENSION_CRASH,
        ];
        for name in &declared {
            assert!(
                known.contains(&name.as_str()),
                "{FIG} declares {name}, which has no constant in `rpc::manager`. A host that \
                 does not know a method the worker offers will not use it; one that spells a \
                 method wrongly gets `No handler for method` at runtime."
            );
        }
    }
}

/// A spawned worker process and the frames going to and from it.
///
/// # Why stdio rather than the socket the plan describes
///
/// PLAN.md §6 says the host talks to the worker "over UDS". It does not: the
/// worker reads `process.stdin` and writes `process.stdout`, and §11.4a records
/// how that was established. Adding a socket to the worker would contradict the
/// same phase's rule that `src/typescript/` is not rewritten, for no capability
/// the host needs, so stdio is what this implements. #101 carries the decision.
///
/// # stderr is left alone on purpose
///
/// `src/snippet/README.md` documents the convention the workers follow —
/// "stderr is used for debug logs" — so it is inherited rather than captured.
/// A host that swallowed it would make a worker's own diagnostics invisible at
/// exactly the moment they are wanted.
pub struct Worker {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    reader: FrameReader<std::process::ChildStdout>,
    next_id: u64,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("pid", &self.child.id())
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

/// Why starting or driving a worker failed.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// The process could not be started, or its pipes were not available.
    #[error("spawning the worker failed: {0}")]
    Spawn(std::io::Error),

    /// Writing to the worker failed.
    #[error("writing to the worker failed: {0}")]
    Write(std::io::Error),

    /// Reading from the worker failed.
    #[error(transparent)]
    Read(#[from] ReadError),

    /// A frame was not the JSON this protocol requires.
    #[error("the worker sent a frame that is not JSON-RPC: {0}")]
    Malformed(#[from] serde_json::Error),

    /// The payload was too large to frame.
    #[error(transparent)]
    Frame(#[from] FrameError),
}

impl Worker {
    /// Spawns `command`, wiring stdin and stdout to frames and leaving stderr
    /// inherited.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Spawn`] if the process will not start or its pipes are
    /// missing.
    pub fn spawn(mut command: std::process::Command) -> Result<Self, WorkerError> {
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(WorkerError::Spawn)?;

        // `take` rather than `as_mut`: the pipes outlive the borrow, and a
        // half-taken child is not a state this type can be left in.
        let stdin = child.stdin.take().ok_or_else(|| {
            WorkerError::Spawn(std::io::Error::other("the worker has no stdin pipe"))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            WorkerError::Spawn(std::io::Error::other("the worker has no stdout pipe"))
        })?;

        Ok(Self {
            child,
            stdin,
            reader: FrameReader::new(stdout),
            next_id: 1,
        })
    }

    /// The worker's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends a request and returns the id it was given.
    ///
    /// Does not wait for the reply: the worker is free to interleave events
    /// with responses, so correlating is the caller's job and pretending
    /// otherwise would mean dropping events that arrive first.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Write`] if the pipe is gone — which is how a worker that
    /// has died presents — or [`WorkerError::Frame`] if the payload is too big.
    pub fn request(&mut self, method: &str, params: serde_json::Value) -> Result<u64, WorkerError> {
        let id = self.next_id;
        self.next_id += 1;

        let body = serde_json::to_vec(&rpc::Request::new(id, method, params))?;
        let framed = encode(&body)?;

        use std::io::Write as _;
        self.stdin.write_all(&framed).map_err(WorkerError::Write)?;
        self.stdin.flush().map_err(WorkerError::Write)?;
        Ok(id)
    }

    /// Reads the next message from the worker, or `None` at a clean exit.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Read`] if the stream fails or ends mid-frame, and
    /// [`WorkerError::Malformed`] if a frame is not JSON-RPC.
    pub fn next_message(&mut self) -> Result<Option<rpc::Incoming>, WorkerError> {
        match self.reader.next_frame()? {
            Some(frame) => Ok(Some(serde_json::from_slice(&frame)?)),
            None => Ok(None),
        }
    }

    /// Closes the worker's stdin and waits for it to exit.
    ///
    /// Closing stdin first is the polite half: the worker's read loop ends, so
    /// it gets to shut down on its own terms rather than being killed
    /// part-way through writing a reply.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Spawn`] if waiting fails.
    pub fn shutdown(mut self) -> Result<std::process::ExitStatus, WorkerError> {
        drop(self.stdin);
        self.child.wait().map_err(WorkerError::Spawn)
    }
}

#[cfg(all(test, unix))]
mod worker_process {
    //! [`Worker`] against real processes.
    //!
    //! The worker these will eventually drive is `vicinae-worker-ts`, which is
    //! not built here, so the child is `cat`: either echoing what the host
    //! wrote, which exercises the encode/pipe/decode path end to end, or
    //! printing a file of bytes written by the test, which is how a worker's
    //! side of a conversation is staged without a worker. Both are real
    //! processes with real pipes — the part these tests exist to cover.

    use super::{ReadError, Worker, WorkerError, encode, rpc};
    use std::path::PathBuf;
    use std::process::Command;

    /// A child that prints `bytes` and exits.
    fn replaying(bytes: &[u8]) -> (tempfile::TempDir, Command) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path: PathBuf = dir.path().join("frames.bin");
        std::fs::write(&path, bytes).expect("staging the worker's output");
        let mut command = Command::new("cat");
        command.arg(&path);
        // The directory has to outlive the child, so it goes back to the caller.
        (dir, command)
    }

    fn frame(value: &serde_json::Value) -> Vec<u8> {
        encode(&serde_json::to_vec(value).expect("a JSON value serialises")).expect("a small frame")
    }

    #[test]
    fn a_request_survives_the_round_trip_through_a_real_pipe() {
        // `cat` echoes stdin to stdout, so what comes back is exactly what
        // `request` framed. That makes this a check of the whole path --
        // serialise, length-prefix, write to a pipe, read back, decode --
        // rather than of `encode` and `decode` agreeing with each other.
        let mut worker = Worker::spawn(Command::new("cat")).expect("cat is spawnable");
        assert!(worker.pid() > 0, "a spawned child has a pid");

        let id = worker
            .request(
                rpc::manager::LOAD,
                serde_json::json!({ "extension_id": "hn" }),
            )
            .expect("writing to cat's stdin");
        assert_eq!(id, 1, "ids start at 1");

        let message = worker
            .next_message()
            .expect("reading cat's stdout")
            .expect("cat echoed the frame");

        assert_eq!(message.jsonrpc, rpc::VERSION);
        assert_eq!(message.id, Some(1));
        assert_eq!(message.method.as_deref(), Some(rpc::manager::LOAD));
        assert_eq!(
            message.params,
            Some(serde_json::json!({ "extension_id": "hn" }))
        );
        assert!(
            !message.is_event(),
            "a message carrying an id is a response, not an event"
        );

        let second = worker
            .request(
                rpc::manager::READY,
                serde_json::json!({ "session_id": "s" }),
            )
            .expect("writing the second request");
        assert_eq!(second, 2, "each request gets a fresh id");
    }

    #[test]
    fn an_event_arriving_before_a_response_is_delivered_not_skipped() {
        // The reason `request` does not wait for its own reply: the worker
        // emits `extensionMessage` whenever an extension talks, including
        // between a request and its response. A host that read until it saw
        // its id would drop this event on the floor.
        let event = frame(&serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "method": rpc::manager::EVENT_EXTENSION_MESSAGE,
            "params": { "session_id": "s", "payload": "{}" },
        }));
        let response = frame(&serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "id": 1,
            "result": { "session_id": "s" },
        }));
        let mut bytes = event;
        bytes.extend_from_slice(&response);

        let (_dir, command) = replaying(&bytes);
        let mut worker = Worker::spawn(command).expect("cat is spawnable");

        let first = worker
            .next_message()
            .expect("reading the first frame")
            .expect("there is a first frame");
        assert!(
            first.is_event(),
            "the event came first on the wire and must come first here: {first:?}"
        );
        assert_eq!(
            first.method.as_deref(),
            Some(rpc::manager::EVENT_EXTENSION_MESSAGE)
        );

        let second = worker
            .next_message()
            .expect("reading the second frame")
            .expect("there is a second frame");
        assert_eq!(second.id, Some(1), "the response follows the event");
        assert_eq!(
            second.result,
            Some(serde_json::json!({ "session_id": "s" }))
        );

        assert_eq!(
            worker.next_message().expect("a clean end of stream"),
            None,
            "the worker exited after two frames"
        );
    }

    #[test]
    fn a_worker_that_dies_mid_frame_is_truncated_not_a_clean_exit() {
        // A worker killed while writing leaves a length prefix promising bytes
        // that never arrive. Reporting that as end-of-stream would make a crash
        // look like an orderly shutdown, which is the difference between
        // restarting the extension and quietly losing it.
        let whole = frame(&serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "id": 1,
            "result": {},
        }));
        let cut = &whole[..whole.len() - 3];

        let (_dir, command) = replaying(cut);
        let mut worker = Worker::spawn(command).expect("cat is spawnable");

        match worker.next_message() {
            Err(WorkerError::Read(ReadError::Truncated { .. })) => {}
            other => panic!("a half-written frame must read as Truncated, got {other:?}"),
        }
    }

    #[test]
    fn a_worker_that_says_nothing_reads_as_a_clean_exit() {
        // The control for the test above: the same code path, with nothing
        // missing, must not report Truncated.
        let (_dir, command) = replaying(b"");
        let mut worker = Worker::spawn(command).expect("cat is spawnable");

        assert_eq!(
            worker
                .next_message()
                .expect("an empty stream is not an error"),
            None
        );
    }

    #[test]
    fn a_frame_that_is_not_json_rpc_is_malformed() {
        // Framing and payload are separate failures. A worker whose payload is
        // wrong -- the wrong `jsonrpc`, a missing field, a log line that
        // escaped onto stdout -- must not present as a transport fault.
        let (_dir, command) = replaying(&encode(b"this is not JSON").expect("a small frame"));
        let mut worker = Worker::spawn(command).expect("cat is spawnable");

        match worker.next_message() {
            Err(WorkerError::Malformed(_)) => {}
            other => panic!("a non-JSON payload must read as Malformed, got {other:?}"),
        }
    }

    #[test]
    fn shutdown_closes_stdin_so_the_worker_ends_on_its_own() {
        // `cat` with a piped stdin and no file runs until its stdin closes, so
        // this hangs rather than fails if `shutdown` stopped dropping stdin
        // before waiting. That is the behaviour under test: the worker is given
        // the chance to finish, not killed part-way through a reply.
        let mut worker = Worker::spawn(Command::new("cat")).expect("cat is spawnable");
        worker
            .request(
                rpc::manager::UNLOAD,
                serde_json::json!({ "session_id": "s" }),
            )
            .expect("writing a final request");

        let status = worker.shutdown().expect("waiting for the worker");
        assert!(
            status.success(),
            "a worker whose stdin closed should exit cleanly, got {status:?}"
        );
    }

    #[test]
    fn spawning_something_that_does_not_exist_is_a_spawn_error() {
        let command = Command::new("compass-no-such-worker-binary");
        match Worker::spawn(command) {
            Err(WorkerError::Spawn(_)) => {}
            other => panic!("a missing binary must be a Spawn error, got {other:?}"),
        }
    }
}
