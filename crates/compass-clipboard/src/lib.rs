//! `compass-clipboard` — the clipboard history store, ported from
//! `src/server/src/services/clipboard/clipboard-db.cpp`.
//!
//! Landing in slices rather than at once. Present so far:
//!
//! * [`kind`] — what an entry is and whether it is encrypted, as the integers
//!   SQLite actually stores.
//! * [`schema`] — the migrations, and applying them to a database.
//! * [`search`] — how a query in the search box becomes the FTS match phrases
//!   and substring terms the SQL layer binds.
//!
//! Still on the C++ side: the schema and migrations, insert/evict, pinning and
//! keywords, and the paginated read itself. See `docs/rust-engine/PARITY.md`.

#![deny(missing_docs)]

pub mod kind;
pub mod schema;
pub mod search;
