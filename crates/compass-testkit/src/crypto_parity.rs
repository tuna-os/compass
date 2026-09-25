//! Suite 0: the C++ and Rust clipboard crypto must be interchangeable.
//!
//! `PLAN.md` §6 Phase 3 gates on the clipboard store being "readable and
//! writable by both engines interchangeably". This is what checks that, against
//! the real `vicinae::crypto` rather than against a description of it.
//!
//! # Why this is not a diff
//!
//! `scorer-parity`, the other rung-1 harness, compares outputs directly,
//! because scoring is a pure function. Encryption is not: the IV comes from the
//! system CSPRNG, so two CORRECT implementations produce different bytes on
//! every call. A harness that compared ciphertexts would fail on a correct
//! port — the same error as a ratchet that fires when a number improves.
//!
//! So the instrument is **cross-decryption**: each engine decrypts what the
//! other produced, in both directions. That is what "interchangeably" means,
//! and it is strictly stronger than a byte comparison would have been, because
//! it exercises each side as both reader and writer.
//!
//! `deriveKey` is deterministic (HKDF-SHA256), so that one *is* compared
//! byte for byte.
//!
//! # Why the controls are not optional
//!
//! Cross-decryption alone can be passed by an implementation that ignores the
//! GCM tag: it would decrypt the other side's output happily, and also
//! everything else. Every run therefore also asserts that both engines
//! REFUSE the inputs they should, with the specific error each should give:
//!
//!   * a single flipped bit, at every offset, is `AuthFailed`;
//!   * the right blob under the wrong key is `AuthFailed`;
//!   * a buffer too short to hold an IV and a tag is `DataTooShort`.
//!
//! A "parity" run in which nothing was rejected would be evidence of nothing.
//!
//! # Scope, so a green run is not over-read
//!
//! This is the CRYPTO, not the clipboard store. `clipboard-db.hpp` and
//! `clipboard-encrypter.cpp` wrap it in a SQLite schema, key management and a
//! mime-type model, none of which is exercised here. Format parity is
//! necessary for the Phase 3 gate and is not the whole of it.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};

/// The C++ probe, held open across the whole run.
///
/// Spawned once rather than per request: the probe is a request/response loop
/// precisely so that a few thousand operations cost one process.
struct Probe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Requests sent, so the driver can assert the probe answered all of them.
    sent: usize,
}

/// What the C++ side said.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reply {
    Ok(Vec<u8>),
    Err(String),
}

impl Probe {
    fn start(path: &str) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not run the C++ probe at {path}"))?;
        let stdin = child.stdin.take().context("probe stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("probe stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            sent: 0,
        })
    }

    fn request(&mut self, line: &str) -> Result<Reply> {
        writeln!(self.stdin, "{line}").context("writing to the probe")?;
        self.stdin.flush().context("flushing to the probe")?;
        self.sent += 1;

        let mut reply = String::new();
        let read = self
            .stdout
            .read_line(&mut reply)
            .context("reading from the probe")?;
        if read == 0 {
            bail!(
                "the C++ probe closed its output after {} request(s); it exited instead of \
                 answering {line:?}",
                self.sent - 1
            );
        }

        let reply = reply.trim_end();
        match reply.split_once(' ') {
            Some(("ok", "-")) => Ok(Reply::Ok(Vec::new())),
            Some(("ok", payload)) => Ok(Reply::Ok(unhex(payload)?)),
            Some(("err", name)) => Ok(Reply::Err(name.to_owned())),
            _ => {
                bail!("the probe answered {reply:?}, which is neither `ok <hex>` nor `err <name>`")
            }
        }
    }

    fn derive(&mut self, master: &[u8], label: &str) -> Result<Reply> {
        let request = format!("derive {} {}", hex(master), hex(label.as_bytes()));
        self.request(&request)
    }

    fn encrypt(&mut self, key: &[u8], plaintext: &[u8]) -> Result<Reply> {
        let request = format!("encrypt {} {}", hex(key), hex(plaintext));
        self.request(&request)
    }

    fn decrypt(&mut self, key: &[u8], blob: &[u8]) -> Result<Reply> {
        let request = format!("decrypt {} {}", hex(key), hex(blob));
        self.request(&request)
    }

    fn finish(mut self) -> Result<()> {
        drop(self.stdin);
        let status = self.child.wait().context("waiting for the probe")?;
        if !status.success() {
            bail!("the C++ probe exited with {status}");
        }
        Ok(())
    }
}

/// Hex, with `-` for the empty string.
///
/// The probe's fields are whitespace separated, which cannot express "present
/// and empty". An empty KDF label and an empty plaintext are both legitimate
/// inputs and both in the corpus below, so they need a spelling: `-`, which is
/// not a hex digit and so cannot collide with data. This was not a design
/// decision made up front -- the empty-label case failed on the first run with
/// `BadRequest`, which is exactly the kind of hole a corpus of only
/// comfortable inputs would have left in place.
fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".to_owned();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("odd-length hex from the probe: {text:?}");
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .with_context(|| format!("bad hex from the probe: {text:?}"))
        })
        .collect()
}

/// Plaintexts worth trying, and why each is here.
///
/// Not random: each targets a boundary where an implementation could plausibly
/// differ and a round-trip of `b"hello"` would not notice.
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        // The empty message is the classic off-by-one: the blob is then exactly
        // IV+TAG, the boundary at which `DataTooShort` stops applying.
        ("empty", Vec::new()),
        ("one byte", vec![0x00]),
        ("ascii", b"hello clipboard".to_vec()),
        // Clipboard content is arbitrary bytes, not text; a UTF-8 assumption
        // anywhere would show up here.
        ("invalid utf-8", vec![0xff, 0xfe, 0x80, 0x00, 0x41]),
        (
            "utf-8 multibyte",
            "héllo — clipboard 📋".as_bytes().to_vec(),
        ),
        ("embedded nul", b"before\0after".to_vec()),
        ("all zeroes, 1 KiB", vec![0x00; 1024]),
        ("all ones, 1 KiB", vec![0xff; 1024]),
        // Larger than a single GCM block and not a multiple of 16, so any
        // block-alignment assumption is exercised.
        (
            "unaligned 4095 B",
            (0..4095).map(|i| (i % 251) as u8).collect(),
        ),
        ("64 KiB", (0..65536).map(|i| (i % 256) as u8).collect()),
    ]
}

/// Labels for the KDF comparison.
fn labels() -> Vec<&'static str> {
    vec![
        "",
        "clipboard",
        "clipboard-history",
        // A label with a space and a non-ASCII byte: the probe protocol hex
        // encodes labels precisely so these cannot be mangled in transit, and
        // this is what proves that claim rather than asserting it.
        "with space",
        "ünïcøde",
    ]
}

struct Counts {
    kdf_compared: usize,
    cpp_to_rust: usize,
    rust_to_cpp: usize,
    controls: usize,
}

fn run(probe_path: &str) -> Result<Counts> {
    let mut probe = Probe::start(probe_path)?;
    let mut counts = Counts {
        kdf_compared: 0,
        cpp_to_rust: 0,
        rust_to_cpp: 0,
        controls: 0,
    };

    // Fixed masters, so a failure is reproducible from the output alone.
    let masters: [Vec<u8>; 3] = [
        (0u8..32).collect(),
        vec![0x00; 32],
        vec![0xab; 48], // HKDF takes any input length; the C++ signature does too.
    ];

    // ---------------------------------------------------------------
    // 1. HKDF is deterministic, so it is compared byte for byte.
    // ---------------------------------------------------------------
    for master in &masters {
        for label in labels() {
            let theirs = match probe.derive(master, label)? {
                Reply::Ok(bytes) => bytes,
                Reply::Err(name) => bail!(
                    "the C++ probe refused to derive with label {label:?}: {name}. Both engines \
                     must derive from the same inputs, so this is a divergence, not a skip."
                ),
            };
            let ours = compass_crypto::derive_key(master, label);
            if theirs != ours[..] {
                bail!(
                    "HKDF-SHA256 disagreement for label {label:?} over a {}-byte master:\n  \
                     C++  {}\n  Rust {}\nThe KDF is deterministic, so this is a real format \
                     divergence: subkeys derived by one engine would be unusable by the other.",
                    master.len(),
                    hex(&theirs),
                    hex(&ours)
                );
            }
            counts.kdf_compared += 1;
        }
    }

    // WHY THE SHIPPED PURPOSE LABELS ARE NOT DIFFED HERE
    //
    // The obvious next step is to derive with `compass_crypto::keys`'
    // DATABASE_LABEL and CLIPBOARD_LABEL and compare. It was written, and it
    // is a tautology: the label handed to the C++ probe would come from the
    // Rust constant, so changing that constant changes what the probe is
    // asked for and the two agree again. Control-tested by setting
    // CLIPBOARD_LABEL to "vicinae-clipboard-v2" -- the run stayed green.
    //
    // The real claim decomposes into two checks that each CAN fail:
    //
    //   * the Rust labels equal the C++ SOURCE labels
    //     -- compass-crypto/tests/upstream_constants.rs, which pins them to
    //        upstream v0.29.0's database-key.cpp (it parsed the in-tree copy
    //        until the C++ engine was removed, ADR-0021);
    //   * HKDF agrees byte for byte for arbitrary labels
    //     -- the loop above, over three masters and five labels.
    //
    // Together those give "Rust derives what C++ derives, for the label C++
    // uses". Adding a third check that restates them without being able to
    // fail would make the coverage look stronger and be worth nothing.

    // ---------------------------------------------------------------
    // 2. Cross-decryption, both directions.
    // ---------------------------------------------------------------
    let key: Vec<u8> = (0u8..32).map(|b| b.wrapping_mul(7)).collect();

    for (name, plaintext) in corpus() {
        // C++ writes, Rust reads.
        let blob = match probe.encrypt(&key, &plaintext)? {
            Reply::Ok(bytes) => bytes,
            Reply::Err(err) => bail!("the C++ probe could not encrypt the {name} case: {err}"),
        };
        let expected_len = compass_crypto::IV_SIZE + plaintext.len() + compass_crypto::TAG_SIZE;
        if blob.len() != expected_len {
            bail!(
                "the C++ blob for the {name} case is {} bytes; the documented layout \
                 [iv|ciphertext|tag] over a {}-byte plaintext is {expected_len}",
                blob.len(),
                plaintext.len()
            );
        }
        match compass_crypto::decrypt(&blob, &key) {
            Ok(recovered) if recovered == plaintext => counts.cpp_to_rust += 1,
            Ok(recovered) => bail!(
                "Rust decrypted the C++ blob for the {name} case to {} bytes, not the {} that \
                 went in",
                recovered.len(),
                plaintext.len()
            ),
            Err(err) => bail!(
                "Rust could NOT read what the C++ engine wrote, for the {name} case: {err}. \
                 A clipboard row written by the C++ engine would be unreadable after migration."
            ),
        }

        // Rust writes, C++ reads.
        let blob = compass_crypto::encrypt(&plaintext, &key)
            .with_context(|| format!("Rust could not encrypt the {name} case"))?;
        match probe.decrypt(&key, &blob)? {
            Reply::Ok(recovered) if recovered == plaintext => counts.rust_to_cpp += 1,
            Reply::Ok(recovered) => bail!(
                "the C++ engine decrypted the Rust blob for the {name} case to {} bytes, not \
                 the {} that went in",
                recovered.len(),
                plaintext.len()
            ),
            Reply::Err(err) => bail!(
                "the C++ engine could NOT read what Rust wrote, for the {name} case: {err}. \
                 A clipboard row written after migration would be unreadable by the old engine."
            ),
        }
    }

    // ---------------------------------------------------------------
    // 3. Controls. Both engines must REFUSE, with the right error.
    // ---------------------------------------------------------------
    let plaintext = b"control".to_vec();
    let blob = compass_crypto::encrypt(&plaintext, &key).context("encrypting the control case")?;

    // Every byte, including the IV and the tag: GCM authenticates the nonce
    // too, so there is no region either engine may let slide.
    for index in 0..blob.len() {
        let mut tampered = blob.clone();
        tampered[index] ^= 0x01;

        if compass_crypto::decrypt(&tampered, &key).is_ok() {
            bail!("Rust accepted a blob with a flipped bit at offset {index}");
        }
        match probe.decrypt(&key, &tampered)? {
            Reply::Err(name) if name == "AuthFailed" => counts.controls += 1,
            Reply::Err(name) => bail!(
                "the C++ engine rejected a bit flip at offset {index} as {name}, not AuthFailed. \
                 The specific error matters: the harness asserts the tag was checked, not merely \
                 that something went wrong."
            ),
            Reply::Ok(_) => {
                bail!("the C++ engine ACCEPTED a blob with a flipped bit at offset {index}")
            }
        }
    }

    // The right blob under the wrong key.
    let wrong_key: Vec<u8> = key.iter().map(|b| b ^ 0xff).collect();
    if compass_crypto::decrypt(&blob, &wrong_key).is_ok() {
        bail!("Rust decrypted a blob under the wrong key");
    }
    match probe.decrypt(&wrong_key, &blob)? {
        Reply::Err(name) if name == "AuthFailed" => counts.controls += 1,
        other => bail!("the C++ engine answered {other:?} for a wrong-key decrypt, not AuthFailed"),
    }

    // Buffers too short to be blobs.
    for len in 0..(compass_crypto::IV_SIZE + compass_crypto::TAG_SIZE) {
        let short = vec![0u8; len];
        if compass_crypto::decrypt(&short, &key).is_ok() {
            bail!("Rust accepted a {len}-byte buffer as a blob");
        }
        match probe.decrypt(&key, &short)? {
            Reply::Err(name) if name == "DataTooShort" => counts.controls += 1,
            other => {
                bail!("the C++ engine answered {other:?} for a {len}-byte buffer, not DataTooShort")
            }
        }
    }

    let sent = probe.sent;
    probe.finish()?;

    // The probe answered every request or `request` would have failed, but say
    // so out loud: a harness that silently did nothing is the failure mode this
    // whole suite exists to rule out.
    if sent == 0 {
        bail!("the harness sent the probe no requests at all");
    }

    Ok(counts)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut probe_path = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--probe" {
            probe_path = args.get(i + 1).cloned();
            i += 2;
        } else {
            i += 1;
        }
    }

    let Some(probe_path) = probe_path else {
        eprintln!(
            "pass --probe <path to vicinae-crypto-probe>; build it with\n  \
             c++ -std=c++23 -O2 -Iscripts/bench/upstream/crypto/include \\\n    \
             -Iscripts/bench/upstream/crypto/src -o /tmp/vicinae-crypto-probe \\\n    \
             scripts/bench/probes/crypto-probe.cpp \\\n    \
             scripts/bench/upstream/crypto/src/{{aes-gcm,kdf,gcm-openssl}}.cpp -lcrypto"
        );
        std::process::exit(2);
    };

    match run(&probe_path) {
        Ok(counts) => {
            println!(
                "clipboard crypto parity: {} KDF vectors identical, {} C++->Rust and {} \
                 Rust->C++ cross-decryptions, {} controls refused as expected",
                counts.kdf_compared, counts.cpp_to_rust, counts.rust_to_cpp, counts.controls
            );
            println!(
                "  scope: this is the crypto, not the clipboard store. The SQLite schema, key \
                 management and mime model of the clipboard service are not exercised \
                 here."
            );
        }
        Err(err) => {
            eprintln!("clipboard crypto parity FAILED: {err:#}");
            std::process::exit(1);
        }
    }
}
