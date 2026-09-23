//! The raw entry points this crate calls, and nothing else.
//!
//! Declared by hand rather than generated. The set is small and fixed — it is
//! what `db::Database` and `db::Statement` in the C++ engine use — and a
//! hand-written list is one that can be read in full, which matters more here
//! than breadth.
//!
//! These are SQLite's own C entry points, documented at
//! <https://sqlite.org/c3ref/intro.html>. Repeating that here in Rust doc
//! comments would produce a second, worse copy that drifts, so the crate's
//! `missing_docs` is relaxed for this module alone. Anything that is *not*
//! plain SQLite — the tokenizer entry point, and the `SQLITE_TRANSIENT`
//! sentinel — is documented, because those are the parts a reader cannot look
//! up.
#![allow(missing_docs)]

use libc::{c_char, c_int, c_void};

/// An open database. Opaque: only SQLite may dereference it.
#[repr(C)]
pub struct Sqlite3 {
    _private: [u8; 0],
}

/// A prepared statement. Opaque.
#[repr(C)]
pub struct Sqlite3Stmt {
    _private: [u8; 0],
}

/// `SQLITE_OK`.
pub const OK: c_int = 0;
/// `SQLITE_ROW`.
pub const ROW: c_int = 100;
/// `SQLITE_DONE`.
pub const DONE: c_int = 101;

/// `SQLITE_OPEN_READWRITE`.
pub const OPEN_READWRITE: c_int = 0x0000_0002;
/// `SQLITE_OPEN_CREATE`.
pub const OPEN_CREATE: c_int = 0x0000_0004;

/// `SQLITE_TRANSIENT`: tells SQLite to copy the bound value.
///
/// The alternative, `SQLITE_STATIC`, promises the pointer outlives the
/// statement, and nothing in this crate is in a position to promise that — a
/// bound `&str` borrows from the caller, who is free to drop it before the
/// statement is stepped.
///
/// It is a function rather than a `const` because `SQLITE_TRANSIENT` is the
/// sentinel `-1` reinterpreted as a function pointer, and const evaluation
/// rejects that: a `const` function pointer must point at a real function.
/// Producing it at runtime is what every other binding does, and the value is
/// never called — SQLite compares it against the sentinel.
#[must_use]
pub fn transient() -> Option<unsafe extern "C" fn(*mut c_void)> {
    Some(unsafe { std::mem::transmute::<isize, unsafe extern "C" fn(*mut c_void)>(-1) })
}

unsafe extern "C" {
    pub fn sqlite3_open_v2(
        filename: *const c_char,
        db: *mut *mut Sqlite3,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    pub fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    pub fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    pub fn sqlite3_errstr(code: c_int) -> *const c_char;
    pub fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    pub fn sqlite3_free(ptr: *mut c_void);

    pub fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        sql: *const c_char,
        n_byte: c_int,
        stmt: *mut *mut Sqlite3Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    pub fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
    pub fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;

    pub fn sqlite3_bind_parameter_index(stmt: *mut Sqlite3Stmt, name: *const c_char) -> c_int;
    pub fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        text: *const c_char,
        n: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    pub fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, idx: c_int, value: i64) -> c_int;
    pub fn sqlite3_bind_null(stmt: *mut Sqlite3Stmt, idx: c_int) -> c_int;

    pub fn sqlite3_column_count(stmt: *mut Sqlite3Stmt) -> c_int;
    pub fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, col: c_int) -> *const u8;
    pub fn sqlite3_column_bytes(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
    pub fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, col: c_int) -> i64;
    pub fn sqlite3_column_type(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;

    /// `vendor/fuzzy-trigram/register.c`. The two trailing arguments exist to
    /// match `sqlite3_auto_extension`'s signature and are ignored, which is why
    /// this crate passes null for both — see [`crate::Database::open`] for why
    /// it is called directly instead of through that mechanism.
    pub fn vicinaeFuzzyTrigramInit(
        db: *mut Sqlite3,
        errmsg: *mut *mut c_char,
        api: *const c_void,
    ) -> c_int;

    /// `vendor/spellfix/register.c`: the `spellfix1` virtual table in the
    /// same static-linkage form, registered right after the tokenizer.
    pub fn vicinaeSpellfixInit(
        db: *mut Sqlite3,
        errmsg: *mut *mut c_char,
        api: *const c_void,
    ) -> c_int;
}

/// `SQLITE_NULL`.
pub const TYPE_NULL: c_int = 5;
