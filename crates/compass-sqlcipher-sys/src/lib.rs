//! SQLCipher, the `fuzzy_trigram` tokenizer, and the `spellfix1` virtual
//! table, built from `vendor/` and wrapped in the small amount of API the
//! database owners need.
//!
//! # Why this crate exists at all
//!
//! The clipboard history is an encrypted SQLCipher database whose FTS5 table
//! declares a tokenizer that ships in this repository as C. Neither is
//! optional: a stock SQLite cannot open the file, and without the tokenizer
//! *every* access to `selection_fts` fails, including a plain `SELECT`. See
//! [ADR-0014](../../../docs/rust-engine/adr/0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md).
//!
//! # Why the `unsafe` is here and not elsewhere
//!
//! The workspace sets `unsafe_code = "forbid"`. This crate declines to inherit
//! that (see its `Cargo.toml`) because the tokenizer registration is a call on
//! a raw `sqlite3*`. The bargain is that the exception is confined: callers get
//! [`Database`] and [`Statement`], which own their handles and expose no raw
//! pointers, and no other crate in the workspace needs `unsafe` to read a
//! clipboard history.
//!
//! # The ordering this crate exists to enforce
//!
//! `clipboard-db.cpp` opens, keys, registers the tokenizer, then sets pragmas —
//! in that order. It is not stylistic. Registering an FTS5 tokenizer queries
//! the database for the `fts5` API pointer, and on an encrypted database that
//! query fails until the key is set. [`Database::open`] does all of it, so a
//! caller cannot get the order wrong; `tests/ordering.rs` shows what happens
//! when it is.

#![deny(missing_docs)]

pub mod ffi;

use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

/// Something SQLite refused to do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// SQLite reported a failure.
    #[error("{context}: {message} (code {code})")]
    Sqlite {
        /// What was being attempted.
        context: &'static str,
        /// SQLite's own message.
        message: String,
        /// The result code.
        code: i32,
    },
    /// A path or SQL string contained an interior NUL, which C cannot carry.
    #[error("{0} contains a NUL byte and cannot be passed to SQLite")]
    InteriorNul(&'static str),
    /// A path was not valid UTF-8.
    #[error("the database path is not valid UTF-8")]
    NonUtf8Path,
    /// A bound parameter name is not in the statement.
    #[error("no parameter named {0} in this statement")]
    NoSuchParameter(String),
}

/// A result from this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn message(code: i32) -> String {
    // Safe to call with any code; returns a static string.
    let ptr = unsafe { ffi::sqlite3_errstr(code) };
    if ptr.is_null() {
        return format!("code {code}");
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// An open, keyed database with the tokenizer registered.
///
/// Closes on drop. `Send` but not `Sync`, like `rusqlite::Connection`: the
/// handle moves to another thread, but two threads must never touch it at
/// once without serialisation this crate does not do.
#[derive(Debug)]
pub struct Database {
    handle: *mut ffi::Sqlite3,
}

// The handle is a resource, not shared state: handing it to another thread is
// safe as long as only one thread uses it at a time. `Sync` stays
// unimplemented, which is what keeps the C `sqlite3*` calls serialised.
unsafe impl Send for Database {}

impl Database {
    /// Open `path`, key it with `key`, register `fuzzy_trigram` and `spellfix1`,
    /// and apply the clipboard pragmas — in that order, because that order is
    /// load-bearing.
    ///
    /// `key` is **raw key material**, not a passphrase: the 32 bytes
    /// `compass_crypto` derives. It is passed to SQLCipher in the `x'...'` form,
    /// which uses the bytes directly rather than running them through a KDF,
    /// exactly as `db::Database::setKey` does. Handing a passphrase here would
    /// produce a database the C++ engine cannot open.
    ///
    /// Pass an empty slice for an unencrypted database; `PRAGMA key` is then
    /// skipped entirely rather than issued with an empty value, which SQLCipher
    /// treats differently.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] if the file cannot be opened, the key is
    /// rejected, or the tokenizer will not register. A wrong key is reported
    /// here rather than at the first query.
    pub fn open(path: &Path, key: &[u8]) -> Result<Self> {
        let text = path.to_str().ok_or(Error::NonUtf8Path)?;
        let c_path = CString::new(text).map_err(|_| Error::InteriorNul("the database path"))?;

        let mut handle: *mut ffi::Sqlite3 = ptr::null_mut();
        let rc = unsafe {
            ffi::sqlite3_open_v2(
                c_path.as_ptr(),
                &raw mut handle,
                ffi::OPEN_READWRITE | ffi::OPEN_CREATE,
                ptr::null(),
            )
        };
        // sqlite3_open_v2 hands back a handle even on failure, so that the
        // error can be read from it; it still has to be closed.
        if rc != ffi::OK {
            let detail = if handle.is_null() {
                message(rc)
            } else {
                let m = unsafe { last_error(handle) };
                unsafe { ffi::sqlite3_close(handle) };
                m
            };
            return Err(Error::Sqlite {
                context: "opening the database",
                message: detail,
                code: rc,
            });
        }

        let db = Self { handle };

        if !key.is_empty() {
            db.key(key)?;
        }
        db.register_tokenizer()?;
        db.register_spellfix()?;

        for pragma in PRAGMAS {
            db.execute(pragma)?;
        }

        Ok(db)
    }

    /// Set the SQLCipher key from raw bytes.
    ///
    /// `PRAGMA` does not accept bound parameters, so the key has to be part of
    /// the SQL text. Hex-encoding it into SQLCipher's `x'...'` raw-key form is
    /// what makes that safe as well as correct: the output alphabet is
    /// `0-9a-f`, so no byte of key material can close the quote or change the
    /// statement, whatever the key happens to be.
    fn key(&self, key: &[u8]) -> Result<()> {
        let mut pragma = String::with_capacity(key.len() * 2 + 20);
        pragma.push_str("PRAGMA key = \"x'");
        for byte in key {
            use std::fmt::Write as _;
            write!(pragma, "{byte:02x}").expect("writing to a String cannot fail");
        }
        pragma.push_str("'\";");
        self.execute(&pragma)?;

        // Touch the schema so a wrong key is reported by `open` rather than by
        // whatever query happens to run first. The C++ engine does the same,
        // and without it `open` succeeds on a database it cannot read.
        self.execute("SELECT count(*) FROM sqlite_master;")
            .map_err(|err| match err {
                Error::Sqlite { message, code, .. } => Error::Sqlite {
                    context: "verifying the database key",
                    message,
                    code,
                },
                other => other,
            })
    }

    /// Register the `fuzzy_trigram` tokenizer on this connection.
    ///
    /// Must happen after keying — see the module docs.
    fn register_tokenizer(&self) -> Result<()> {
        let rc = unsafe { ffi::vicinaeFuzzyTrigramInit(self.handle, ptr::null_mut(), ptr::null()) };
        if rc == ffi::OK {
            return Ok(());
        }
        Err(Error::Sqlite {
            context: "registering the fuzzy_trigram tokenizer",
            message: unsafe { last_error(self.handle) },
            code: rc,
        })
    }

    /// Register the `spellfix1` virtual table on this connection.
    ///
    /// After the tokenizer, matching the C++ engine's registration order.
    /// Without it every access to `spellfix_vocab` fails, including a plain
    /// `SELECT` — which is how the file indexer's typo correction went dark
    /// in Rust until this call existed.
    fn register_spellfix(&self) -> Result<()> {
        let rc = unsafe { ffi::vicinaeSpellfixInit(self.handle, ptr::null_mut(), ptr::null()) };
        if rc == ffi::OK {
            return Ok(());
        }
        Err(Error::Sqlite {
            context: "registering the spellfix1 module",
            message: unsafe { last_error(self.handle) },
            code: rc,
        })
    }

    /// Run SQL that returns no rows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] with SQLite's own message on failure.
    pub fn execute(&self, sql: &str) -> Result<()> {
        let c_sql = CString::new(sql).map_err(|_| Error::InteriorNul("the SQL"))?;
        let mut err: *mut libc::c_char = ptr::null_mut();
        let rc = unsafe {
            ffi::sqlite3_exec(
                self.handle,
                c_sql.as_ptr(),
                None,
                ptr::null_mut(),
                &raw mut err,
            )
        };
        if rc == ffi::OK {
            return Ok(());
        }
        let detail = if err.is_null() {
            message(rc)
        } else {
            let owned = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { ffi::sqlite3_free(err.cast()) };
            owned
        };
        Err(Error::Sqlite {
            context: "executing SQL",
            message: detail,
            code: rc,
        })
    }

    /// Prepare a statement.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] if SQLite will not compile `sql`.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        let c_sql = CString::new(sql).map_err(|_| Error::InteriorNul("the SQL"))?;
        let mut stmt: *mut ffi::Sqlite3Stmt = ptr::null_mut();
        let rc = unsafe {
            ffi::sqlite3_prepare_v2(
                self.handle,
                c_sql.as_ptr(),
                -1,
                &raw mut stmt,
                ptr::null_mut(),
            )
        };
        if rc != ffi::OK || stmt.is_null() {
            return Err(Error::Sqlite {
                context: "preparing a statement",
                message: unsafe { last_error(self.handle) },
                code: rc,
            });
        }
        Ok(Statement {
            stmt,
            db: self,
            _marker: std::marker::PhantomData,
        })
    }

    /// The single-column text result of `sql`, for one-off queries like
    /// `PRAGMA cipher_version`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] on failure. Returns `Ok(None)` when the query
    /// produced no row.
    pub fn query_one_text(&self, sql: &str) -> Result<Option<String>> {
        let mut stmt = self.prepare(sql)?;
        if stmt.step()? {
            Ok(stmt.column_text(0))
        } else {
            Ok(None)
        }
    }
}

impl Database {
    /// Begin a transaction.
    ///
    /// The returned guard rolls back when dropped unless [`Transaction::commit`]
    /// is called, so an early return cannot leave a half-applied change
    /// committed. That is the same shape as the C++ `db::Transaction`, and it
    /// matters in `evictOlderThan`, which returns early without committing when
    /// there is nothing to evict.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] if `BEGIN` fails, which it does when a
    /// transaction is already open — this crate does not nest.
    pub fn transaction(&self) -> Result<Transaction<'_>> {
        self.execute("BEGIN")?;
        Ok(Transaction {
            db: self,
            finished: false,
        })
    }
}

/// An open transaction. Rolls back on drop unless committed.
#[derive(Debug)]
pub struct Transaction<'db> {
    db: &'db Database,
    finished: bool,
}

impl Transaction<'_> {
    /// Commit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] if the commit fails, in which case the
    /// transaction is left for the drop to roll back.
    pub fn commit(mut self) -> Result<()> {
        self.db.execute("COMMIT")?;
        self.finished = true;
        Ok(())
    }

    /// Roll back explicitly, rather than by dropping.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] if the rollback fails.
    pub fn rollback(mut self) -> Result<()> {
        self.finished = true;
        self.db.execute("ROLLBACK")
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Nothing useful to do with a failure here: the caller has already
        // stopped caring, and panicking in a drop during unwinding aborts.
        if let Err(err) = self.db.execute("ROLLBACK") {
            tracing::warn!(?err, "rolling back a dropped transaction failed");
        }
    }
}

/// The pragmas `clipboard-db.cpp` applies on every connection.
const PRAGMAS: [&str; 4] = [
    "PRAGMA journal_mode = WAL",
    "PRAGMA synchronous = normal",
    "PRAGMA journal_size_limit = 6144000",
    "PRAGMA foreign_keys = ON",
];

/// Read the connection's last error message.
///
/// # Safety
///
/// `handle` must be a live connection.
unsafe fn last_error(handle: *mut ffi::Sqlite3) -> String {
    let ptr = unsafe { ffi::sqlite3_errmsg(handle) };
    if ptr.is_null() {
        return "no message".to_owned();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

impl Drop for Database {
    fn drop(&mut self) {
        // Statements borrow the Database, so they are all finalized by now.
        unsafe { ffi::sqlite3_close(self.handle) };
    }
}

/// A prepared statement, finalized on drop.
#[derive(Debug)]
pub struct Statement<'db> {
    stmt: *mut ffi::Sqlite3Stmt,
    db: &'db Database,
    _marker: std::marker::PhantomData<&'db ()>,
}

impl Statement<'_> {
    /// Bind text to a named parameter such as `:id`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoSuchParameter`] if the statement has no such
    /// parameter — unlike the C++ wrapper, which silently ignores the bind and
    /// runs the query with a NULL in that position.
    pub fn bind_text(&mut self, name: &str, value: &str) -> Result<()> {
        let index = self.parameter_index(name)?;
        self.bind_text_at(index, value)
    }

    /// Bind an integer to a named parameter.
    ///
    /// # Errors
    ///
    /// As [`Statement::bind_text`].
    pub fn bind_int64(&mut self, name: &str, value: i64) -> Result<()> {
        let index = self.parameter_index(name)?;
        let rc = unsafe { ffi::sqlite3_bind_int64(self.stmt, index, value) };
        self.check(rc, "binding an integer")
    }

    /// Bind SQL NULL to a named parameter.
    ///
    /// Explicit rather than implied by "did not bind": an unbound parameter is
    /// also NULL, so without this a typo'd name and a deliberate NULL would be
    /// indistinguishable — which is exactly the confusion
    /// [`Statement::bind_text`]'s error exists to prevent.
    ///
    /// # Errors
    ///
    /// As [`Statement::bind_text`].
    pub fn bind_null(&mut self, name: &str) -> Result<()> {
        let index = self.parameter_index(name)?;
        let rc = unsafe { ffi::sqlite3_bind_null(self.stmt, index) };
        self.check(rc, "binding null")
    }

    fn parameter_index(&self, name: &str) -> Result<i32> {
        let c_name = CString::new(name).map_err(|_| Error::InteriorNul("the parameter name"))?;
        let index = unsafe { ffi::sqlite3_bind_parameter_index(self.stmt, c_name.as_ptr()) };
        if index == 0 {
            return Err(Error::NoSuchParameter(name.to_owned()));
        }
        Ok(index)
    }

    fn bind_text_at(&mut self, index: i32, value: &str) -> Result<()> {
        // SQLITE_TRANSIENT: SQLite copies, so `value` need not outlive the call.
        let len = i32::try_from(value.len()).map_err(|_| Error::Sqlite {
            context: "binding text",
            message: "the value is larger than SQLite accepts".to_owned(),
            code: -1,
        })?;
        let rc = unsafe {
            ffi::sqlite3_bind_text(
                self.stmt,
                index,
                value.as_ptr().cast::<libc::c_char>(),
                len,
                ffi::transient(),
            )
        };
        self.check(rc, "binding text")
    }

    /// Advance the statement. `Ok(true)` means a row is available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sqlite`] on anything other than a row or completion.
    pub fn step(&mut self) -> Result<bool> {
        let rc = unsafe { ffi::sqlite3_step(self.stmt) };
        match rc {
            ffi::ROW => Ok(true),
            ffi::DONE => Ok(false),
            _ => Err(Error::Sqlite {
                context: "stepping a statement",
                message: unsafe { last_error(self.db.handle) },
                code: rc,
            }),
        }
    }

    /// The text in `col` of the current row, or `None` if it is NULL.
    #[must_use]
    pub fn column_text(&self, col: i32) -> Option<String> {
        if self.is_null(col) {
            return None;
        }
        let ptr = unsafe { ffi::sqlite3_column_text(self.stmt, col) };
        if ptr.is_null() {
            return None;
        }
        // sqlite3_column_bytes must be called after column_text, which is what
        // defines the length of the value column_text returned.
        let len = unsafe { ffi::sqlite3_column_bytes(self.stmt, col) };
        let len = usize::try_from(len).unwrap_or(0);
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    /// The integer in `col` of the current row.
    #[must_use]
    pub fn column_int64(&self, col: i32) -> i64 {
        unsafe { ffi::sqlite3_column_int64(self.stmt, col) }
    }

    /// Whether `col` of the current row is NULL.
    #[must_use]
    pub fn is_null(&self, col: i32) -> bool {
        unsafe { ffi::sqlite3_column_type(self.stmt, col) == ffi::TYPE_NULL }
    }

    /// Rewind for reuse: reset the cursor and clear every binding, the way the
    /// C++ engine's `db::Statement::exec` leaves a statement after running it.
    ///
    /// Stepping a finished statement without this silently replays its
    /// completion instead of running the new bindings — which is how a reused
    /// upsert quietly indexed one file and skipped the rest. Bind everything
    /// again before stepping: cleared means cleared.
    pub fn reset(&mut self) {
        unsafe { ffi::sqlite3_reset(self.stmt) };
        unsafe { ffi::sqlite3_clear_bindings(self.stmt) };
    }

    /// How many columns this statement returns.
    #[must_use]
    pub fn column_count(&self) -> i32 {
        unsafe { ffi::sqlite3_column_count(self.stmt) }
    }

    fn check(&self, rc: i32, context: &'static str) -> Result<()> {
        if rc == ffi::OK {
            return Ok(());
        }
        Err(Error::Sqlite {
            context,
            message: unsafe { last_error(self.db.handle) },
            code: rc,
        })
    }
}

impl Drop for Statement<'_> {
    fn drop(&mut self) {
        unsafe { ffi::sqlite3_finalize(self.stmt) };
    }
}

#[cfg(test)]
mod cpp_pragmas {
    //! The connection pragmas are read back out of the C++ source.
    //!
    //! WHY THIS IS NOT PARANOIA, AND WHY A COMMENT WAS NOT ENOUGH
    //!
    //! [`PRAGMAS`] carried a comment saying it came from `clipboard-db.cpp`,
    //! and `the_real_storage_engine.rs` asserts WAL is set "because
    //! clipboard-db.cpp sets WAL on every connection". Both are *our reading*
    //! of the C++ source, restated by hand. Nothing noticed if the C++ list
    //! changed.
    //!
    //! That matters while both engines ship side by side, because they open the
    //! same file. `journal_mode` is the sharp one: WAL is a property of the
    //! DATABASE, not of the connection, so an engine that stopped setting it
    //! would leave the other reading a file in a mode it did not expect --- and
    //! `foreign_keys` is per-connection, so one engine dropping it deletes rows
    //! the other would have refused to orphan. Neither shows up as a crash.
    //!
    //! So the list is parsed rather than described, in the same way
    //! `compass-clipboard`'s `cpp_enum_values.rs` parses the stored enum
    //! numbering instead of restating it.
    //!
    //! WHAT IT CANNOT DO
    //!
    //! It compares pragma *text*, so a value changed on both sides at once
    //! still matches --- that is the point, they agree --- and a pragma applied
    //! somewhere other than this array is invisible to it. And when the C++
    //! tree goes at Phase 8 this test goes with it, because by then the pragmas
    //! are whatever Compass says they are.

    use super::PRAGMAS;
    use std::path::{Path, PathBuf};

    const SOURCE: &str = "src/server/src/services/clipboard/clipboard-db.cpp";
    const ARRAY: &str = "CLIPBOARD_PRAGMAS[] = {";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/compass-sqlcipher-sys sits two levels below the repository root")
            .to_path_buf()
    }

    /// The quoted strings of the C++ `CLIPBOARD_PRAGMAS` initialiser, in order.
    fn cpp_pragmas(text: &str) -> Vec<String> {
        let start = text
            .find(ARRAY)
            .unwrap_or_else(|| panic!("{SOURCE} no longer contains `{ARRAY}`"));
        let rest = &text[start + ARRAY.len()..];
        let end = rest
            .find('}')
            .unwrap_or_else(|| panic!("{SOURCE}'s {ARRAY} initialiser is not closed"));

        let mut out = Vec::new();
        let mut chars = rest[..end].chars();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut literal = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                literal.push(c);
            }
            out.push(literal);
        }
        out
    }

    #[test]
    fn the_pragmas_match_the_cpp_engine_exactly_and_in_order() {
        let path = repo_root().join(SOURCE);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let theirs = cpp_pragmas(&text);

        // Vacuity guard. A parser that silently found nothing would agree with
        // an empty list and this test would pass for ever.
        assert!(
            theirs.len() >= 4,
            "parsed only {} pragmas from {SOURCE}; the array's shape has probably \
             changed and this test is no longer checking anything: {theirs:?}",
            theirs.len()
        );

        // Order is compared, not just membership: `Database::open` documents the
        // key/tokenizer/pragma order as load-bearing, and journal_mode before
        // foreign_keys is part of the same contract.
        assert_eq!(
            theirs,
            PRAGMAS.to_vec(),
            "the C++ engine's connection pragmas and ours have diverged. Both engines \
             open the same clipboard database, so this is a file-format disagreement \
             rather than a style difference."
        );
    }
}
