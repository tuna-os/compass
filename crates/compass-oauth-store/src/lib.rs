//! Where an extension's OAuth tokens are kept.
//!
//! Ports `OAuth::TokenStore` (`src/server/src/services/oauth/oauth-token-store.cpp`):
//! one row per `(extension_id, provider_id)` in the `vicinae` database's
//! `oauth_token_set` table. `figura/tsapi.fig`'s `OAuth/getTokens`,
//! `setTokens` and `removeTokens` are this table; `authorize` is not, because
//! it needs a browser and an overlay.
//!
//! # Three details that are on disk
//!
//! **An absent provider is the empty string.** The C++ binds
//! `providerId.value_or(QString(""))` on every path, and the column is
//! `NOT NULL DEFAULT ''`. So an extension with one provider and an extension
//! that passes no provider id share a row, deliberately. [`list`] turns `""`
//! back into `None`; a lookup does not need to, because it was given the id it
//! is looking for.
//!
//! **`updated_at` is the store's, not the caller's.** `setTokenSet` binds
//! `QDateTime::currentSecsSinceEpoch()` and ignores anything the payload might
//! have said. `TokenSet.updatedAt` is therefore something an extension reads
//! and never writes.
//!
//! **Setting replaces; it does not merge.** The `ON CONFLICT DO UPDATE` sets
//! every nullable column, so a `setTokens` without a refresh token clears the
//! refresh token that was there. An extension refreshing an access token has
//! to send the refresh token back with it — which is what the Raycast API's
//! `TokenSet` shape pushes it to do, and what this reproduces.
//!
//! # No encryption beyond the database's own
//!
//! These are bearer tokens in a plain column, exactly as the C++ leaves them.
//! The `vicinae` database is not SQLCipher-encrypted (only the clipboard's is),
//! so the protection is the file's permissions. That is a property of the
//! format this port has to read, not a decision made here; changing it is a
//! change to both engines at once and belongs in its own proposal.

use compass_sqlcipher_sys::Database;

/// A stored token set.
///
/// The field names are `figura/tsapi.fig`'s `TokenSet`; the column names are
/// the table's. They differ in case, and both are fixed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenSet {
    /// Which extension.
    pub extension_id: String,
    /// Which provider, or `None` for the unnamed one.
    pub provider_id: Option<String>,
    /// The bearer token.
    pub access_token: String,
    /// The refresh token, if the provider issued one.
    pub refresh_token: Option<String>,
    /// The id token, if the provider issued one.
    pub id_token: Option<String>,
    /// The granted scope, verbatim.
    pub scope: Option<String>,
    /// Lifetime in seconds, as the provider reported it.
    pub expires_in: Option<i64>,
    /// When this row was last written, in seconds since the epoch.
    pub updated_at: i64,
}

/// Something wrong with reading or writing the token store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database refused something.
    #[error(transparent)]
    Database(#[from] compass_sqlcipher_sys::Error),
}

/// The token store, over an already-migrated `vicinae` database.
#[derive(Debug)]
pub struct TokenStore<'a> {
    db: &'a Database,
}

/// What the C++ binds for an absent provider id.
const NO_PROVIDER: &str = "";

fn provider_or_empty(provider_id: Option<&str>) -> &str {
    provider_id.unwrap_or(NO_PROVIDER)
}

impl<'a> TokenStore<'a> {
    /// Wraps `db`.
    #[must_use]
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Writes `set`, replacing any row for the same extension and provider.
    ///
    /// `set.updated_at` is ignored; the stored value is `now`, which the caller
    /// passes so that this stays testable and so that the clock is not read
    /// from three layers down. The C++ reads it here, which is the same thing
    /// with less control.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the write fails.
    pub fn set(&self, set: &TokenSet, now: i64) -> Result<(), Error> {
        let mut stmt = self.db.prepare(
            "INSERT INTO oauth_token_set(extension_id, provider_id, access_token, refresh_token, \
             id_token, scope, expires_in, updated_at) \
             VALUES (:extension_id, :provider_id, :access_token, :refresh_token, :id_token, \
             :scope, :expires_in, :updated_at) \
             ON CONFLICT DO UPDATE SET access_token = :access_token, \
             refresh_token = :refresh_token, id_token = :id_token, scope = :scope, \
             expires_in = :expires_in, updated_at = :updated_at",
        )?;

        stmt.bind_text(":extension_id", &set.extension_id)?;
        stmt.bind_text(
            ":provider_id",
            provider_or_empty(set.provider_id.as_deref()),
        )?;
        stmt.bind_text(":access_token", &set.access_token)?;
        bind_optional_text(&mut stmt, ":refresh_token", set.refresh_token.as_deref())?;
        bind_optional_text(&mut stmt, ":id_token", set.id_token.as_deref())?;
        bind_optional_text(&mut stmt, ":scope", set.scope.as_deref())?;
        match set.expires_in {
            Some(seconds) => stmt.bind_int64(":expires_in", seconds)?,
            None => stmt.bind_null(":expires_in")?,
        }
        stmt.bind_int64(":updated_at", now)?;
        stmt.step()?;
        Ok(())
    }

    /// Reads one extension's token set for `provider_id`.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails.
    pub fn get(
        &self,
        extension_id: &str,
        provider_id: Option<&str>,
    ) -> Result<Option<TokenSet>, Error> {
        let mut stmt = self.db.prepare(
            "SELECT access_token, refresh_token, id_token, scope, expires_in, updated_at \
             FROM oauth_token_set WHERE extension_id = :extension_id \
             AND provider_id = :provider_id",
        )?;
        stmt.bind_text(":extension_id", extension_id)?;
        stmt.bind_text(":provider_id", provider_or_empty(provider_id))?;

        if !stmt.step()? {
            return Ok(None);
        }

        Ok(Some(TokenSet {
            extension_id: extension_id.to_owned(),
            provider_id: provider_id.map(ToOwned::to_owned),
            access_token: stmt.column_text(0).unwrap_or_default(),
            refresh_token: optional_text(&stmt, 1),
            id_token: optional_text(&stmt, 2),
            scope: optional_text(&stmt, 3),
            expires_in: (!stmt.is_null(4)).then(|| stmt.column_int64(4)),
            updated_at: stmt.column_int64(5),
        }))
    }

    /// Removes one extension's token set for `provider_id`.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the delete fails.
    pub fn remove(&self, extension_id: &str, provider_id: Option<&str>) -> Result<(), Error> {
        let mut stmt = self.db.prepare(
            "DELETE FROM oauth_token_set WHERE extension_id = :extension_id \
             AND provider_id = :provider_id",
        )?;
        stmt.bind_text(":extension_id", extension_id)?;
        stmt.bind_text(":provider_id", provider_or_empty(provider_id))?;
        stmt.step()?;
        Ok(())
    }

    /// Every token set, for whatever wants to show the user what is stored.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails.
    pub fn list(&self) -> Result<Vec<TokenSet>, Error> {
        let mut stmt = self.db.prepare(
            "SELECT extension_id, provider_id, access_token, refresh_token, id_token, scope, \
             expires_in, updated_at FROM oauth_token_set",
        )?;

        let mut out = Vec::new();
        while stmt.step()? {
            let provider = stmt.column_text(1).unwrap_or_default();
            out.push(TokenSet {
                extension_id: stmt.column_text(0).unwrap_or_default(),
                provider_id: (!provider.is_empty()).then_some(provider),
                access_token: stmt.column_text(2).unwrap_or_default(),
                refresh_token: optional_text(&stmt, 3),
                id_token: optional_text(&stmt, 4),
                scope: optional_text(&stmt, 5),
                expires_in: (!stmt.is_null(6)).then(|| stmt.column_int64(6)),
                updated_at: stmt.column_int64(7),
            });
        }
        Ok(out)
    }
}

fn bind_optional_text(
    stmt: &mut compass_sqlcipher_sys::Statement<'_>,
    name: &str,
    value: Option<&str>,
) -> Result<(), compass_sqlcipher_sys::Error> {
    match value {
        Some(text) => stmt.bind_text(name, text),
        None => stmt.bind_null(name),
    }
}

fn optional_text(stmt: &compass_sqlcipher_sys::Statement<'_>, col: i32) -> Option<String> {
    (!stmt.is_null(col)).then(|| stmt.column_text(col).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = Database::open(&dir.path().join("vicinae.db"), &[]).expect("an unencrypted db");
        compass_db::vicinae::run(&db).expect("the migrations apply");
        (dir, db)
    }

    fn full(extension: &str, provider: Option<&str>) -> TokenSet {
        TokenSet {
            extension_id: extension.to_owned(),
            provider_id: provider.map(ToOwned::to_owned),
            access_token: "access".to_owned(),
            refresh_token: Some("refresh".to_owned()),
            id_token: Some("id".to_owned()),
            scope: Some("read write".to_owned()),
            expires_in: Some(3600),
            updated_at: 0,
        }
    }

    #[test]
    fn a_token_set_round_trips() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        let set = full("hn", Some("github"));
        store.set(&set, 1_700_000_000).expect("write");

        let read = store
            .get("hn", Some("github"))
            .expect("read")
            .expect("a row");
        assert_eq!(
            read,
            TokenSet {
                updated_at: 1_700_000_000,
                ..set
            }
        );
    }

    #[test]
    fn the_store_stamps_updated_at_and_ignores_the_callers() {
        // `setTokenSet` binds `QDateTime::currentSecsSinceEpoch()`. An
        // extension cannot backdate a token set by claiming an older time.
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        let mut set = full("hn", None);
        set.updated_at = 42;
        store.set(&set, 1_700_000_000).expect("write");

        assert_eq!(
            store
                .get("hn", None)
                .expect("read")
                .expect("a row")
                .updated_at,
            1_700_000_000
        );
    }

    #[test]
    fn an_absent_provider_is_the_empty_string_on_disk() {
        // Two extensions' rows are distinguished by extension_id; one
        // extension's unnamed provider is a row with provider_id = ''. A port
        // that stored NULL there would never match the C++'s lookups, which
        // bind '' unconditionally.
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store.set(&full("hn", None), 1).expect("write");

        let mut stmt = db
            .prepare("SELECT provider_id FROM oauth_token_set")
            .expect("prepare");
        assert!(stmt.step().expect("a row"));
        assert_eq!(stmt.column_text(0).as_deref(), Some(""));
    }

    #[test]
    fn a_named_and_an_unnamed_provider_are_different_rows() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store.set(&full("hn", None), 1).expect("write");
        store.set(&full("hn", Some("github")), 1).expect("write");

        assert_eq!(store.list().expect("list").len(), 2);
        assert!(store.get("hn", None).expect("read").is_some());
        assert!(store.get("hn", Some("github")).expect("read").is_some());
        assert_eq!(
            store.get("hn", Some("gitlab")).expect("read"),
            None,
            "a provider that was never stored must not match another one's row"
        );
    }

    #[test]
    fn setting_again_replaces_every_field_rather_than_merging() {
        // The ON CONFLICT clause sets all six columns. An extension that
        // refreshes an access token without sending the refresh token back
        // loses the refresh token -- in both engines.
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store.set(&full("hn", None), 1).expect("write");

        let sparse = TokenSet {
            extension_id: "hn".to_owned(),
            access_token: "newer".to_owned(),
            ..TokenSet::default()
        };
        store.set(&sparse, 2).expect("overwrite");

        let read = store.get("hn", None).expect("read").expect("a row");
        assert_eq!(read.access_token, "newer");
        assert_eq!(read.refresh_token, None, "the refresh token was merged in");
        assert_eq!(read.id_token, None);
        assert_eq!(read.scope, None);
        assert_eq!(read.expires_in, None);
        assert_eq!(store.list().expect("list").len(), 1, "a row was added");
    }

    #[test]
    fn a_missing_set_reads_as_none() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        assert_eq!(store.get("nobody", None).expect("read"), None);
    }

    #[test]
    fn removing_affects_one_row() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store.set(&full("hn", None), 1).expect("write");
        store.set(&full("hn", Some("github")), 1).expect("write");

        store.remove("hn", Some("github")).expect("remove");
        assert!(store.get("hn", None).expect("read").is_some());
        assert_eq!(store.get("hn", Some("github")).expect("read"), None);

        // Removing something that is not there is not an error, as the C++'s
        // `DELETE` is not.
        store.remove("hn", Some("github")).expect("remove again");
    }

    #[test]
    fn list_turns_the_empty_provider_back_into_none() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store.set(&full("hn", None), 1).expect("write");
        store.set(&full("other", Some("github")), 1).expect("write");

        let mut listed = store.list().expect("list");
        listed.sort_by(|a, b| a.extension_id.cmp(&b.extension_id));
        assert_eq!(listed[0].extension_id, "hn");
        assert_eq!(listed[0].provider_id, None, "'' must read back as None");
        assert_eq!(listed[1].provider_id.as_deref(), Some("github"));
    }

    #[test]
    fn a_null_column_reads_as_none_rather_than_an_empty_string() {
        // An extension has to be able to tell "no refresh token" from "a
        // refresh token that is the empty string"; the column is nullable for
        // exactly that reason.
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        store
            .set(
                &TokenSet {
                    extension_id: "hn".to_owned(),
                    access_token: "a".to_owned(),
                    refresh_token: Some(String::new()),
                    ..TokenSet::default()
                },
                1,
            )
            .expect("write");

        let read = store.get("hn", None).expect("read").expect("a row");
        assert_eq!(read.refresh_token, Some(String::new()));
        assert_eq!(read.id_token, None);
    }
}
