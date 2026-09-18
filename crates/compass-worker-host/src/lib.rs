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
