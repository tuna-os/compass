# ADR-0014: The clipboard store is SQLCipher plus a vendored FTS5 tokenizer, and the Rust engine must link both

**Status:** Accepted · **Date:** 2026-09-17 · Relates to: PLAN.md §6 Phase 3, ADR-0013 (phases 9–10)

## Context

Phase 3 ports `clipboard-db.cpp`. The first two slices (`search::plan`, the stored enums) are pure
logic and needed no database, which made it easy to assume the rest is "add a SQLite crate and port
the queries". Reading the build says otherwise. Two facts about the existing on-disk format
constrain the dependency choice, and both were found by reading `CMakeLists.txt` rather than the
`.cpp`:

**1. It is not SQLite. It is SQLCipher.** `vendor/sqlcipher/` builds the amalgamation with
`SQLITE_HAS_CODEC`, and `src/server/CMakeLists.txt` links it. Every page of an existing clipboard
history is encrypted, which is what `ClipboardDatabase(std::optional<db::EncryptionKey>)` and
`database-key.cpp` are for. A stock-SQLite build cannot open one of these files at all — not
"cannot decrypt the payloads", cannot open the database.

The crypto provider is per-platform, chosen in that same file: `SQLCIPHER_CRYPTO_OPENSSL` on Linux,
`SQLCIPHER_CRYPTO_CC` (CommonCrypto) on macOS, and a custom `SQLCIPHER_CRYPTO_CUSTOM=sqlcipher_cng_setup`
hook backed by `bcrypt.dll` on Windows.

**2. The FTS5 table declares a tokenizer that ships in this repository.**
`002_trigram_fts.sql` creates `selection_fts` with `tokenize='fuzzy_trigram remove_diacritics 2'`.
`fuzzy_trigram` is not an SQLite tokenizer; it is 1331 lines of vendored C in
`vendor/fuzzy-trigram/`, registered per-connection through `sqlite3_auto_extension` via
`SQLITE_EXTRA_INIT=sqlcipher_extra_init`. `src/file-indexer` uses it too.

An engine that has not registered it does not merely lose search quality. Checked, by building an
FTS5 table and rewriting its schema to name an unregistered tokenizer — which is exactly what an
existing history looks like to such an engine:

```
plain SELECT   -> OperationalError: no such tokenizer: fuzzy_trigram
MATCH query    -> OperationalError: no such tokenizer: fuzzy_trigram
INSERT         -> OperationalError: no such tokenizer: fuzzy_trigram
```

Every access fails, including a plain `SELECT` with no search in it. The tokenizer is required to
open the table, not to search it.

## Decision

1. **The Rust clipboard store links SQLCipher, not stock SQLite.** The vendored amalgamation in
   `vendor/sqlcipher/` is the same source either language compiles, so this is a linking decision
   rather than a second copy of SQLite.

2. **The Rust clipboard store registers `fuzzy_trigram` from `vendor/fuzzy-trigram/`**, the same C
   the C++ engine registers. It is not reimplemented in Rust. A Rust reimplementation would have to
   produce byte-identical trigrams for every input or the index silently stops matching rows it
   previously matched, and nothing in the schema would say so.

3. **Neither is wrapped in `compass-platform`.** Both are portable C with per-platform *crypto
   providers*, which is a build concern, not a trait. The seam ADR-0013 requires is for behaviour
   that differs per platform; the storage format does not.

4. **The choice of Rust SQLite binding is deferred to the slice that needs it**, but it is
   constrained to one that can link an external SQLCipher and expose the raw `sqlite3*` handle for
   `sqlite3_auto_extension`. That rules out any binding that insists on bundling its own stock
   SQLite.

## Consequences

**The Rust engine inherits a C dependency it cannot drop.** ADR-0013 moves the repository off Qt;
it does not move it off C. `vendor/sqlcipher` and `vendor/fuzzy-trigram` outlive Phase 8 and
outlive Phase 10, because they are the file format, not the implementation.

**Phases 9 and 10 acquire a task ADR-0013 did not name.** SQLCipher's crypto provider is selected
at compile time per platform. Whatever replaces the CMake build has to make the same three choices,
and a Windows build that silently picks OpenSSL instead of the CNG hook produces a database the
C++ engine cannot read.

**Testing the clipboard store needs a real database, and can have one.** Once the binding is
linked, the port can be tested against actual SQLCipher files rather than by reading SQL strings —
which matters, because the two bugs found so far in `clipboard-db.cpp` (eviction orphaning blobs,
`tryBubbleUpSelection` ignoring its statement's result) are both behavioural and neither is visible
in a string comparison. This is the first part of the port where a test can exercise the real
storage engine, and it should.

**What this ADR does not settle.** Whether the Rust engine reads the *existing* history in place or
migrates it to a new file is a separate decision, and it is only forced at Phase 7 cutover. Both
options need the constraints above: reading in place obviously, and migrating because the migration
has to read the old file first.
