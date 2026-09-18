//! Per-extension key/value storage, as the `LocalStorage` Raycast API sees it.
//!
//! Ports `LocalStorageService` and `ScopedLocalStorage`
//! (`src/server/src/services/local-storage/`). The store is one table in the
//! `vicinae` database, keyed by `(namespace_id, key)`, and an extension sees
//! only its own namespace — the C++ builds that namespace as
//! `QString("%1:data").arg(provider)`, which [`namespace_for`] reproduces.
//!
//! # The typing is lossy, and that is the format
//!
//! The column stores a string plus a `value_type` discriminant, and the C++
//! `serializeValue` handles exactly three cases:
//!
//! ```cpp
//! if (value.isString()) { return {value.toString(), ValueType::String}; }
//! if (value.isDouble()) { return {QString::number(value.toDouble()), ValueType::Number}; }
//! if (value.isBool()) { return {value.toBool() ? "1" : "0", ValueType::Boolean}; }
//! return {"", ValueType::String};
//! ```
//!
//! An object, an array or a null therefore becomes **the empty string**, and
//! reads back as `""`. That is not a bug this port fixes: a database written by
//! either engine is read by the other, and an extension that stored an object
//! yesterday must get the same answer today. [`Value::from_json`] does the same
//! thing, and [`store_an_object_and_it_comes_back_empty`] pins it so that
//! nobody "corrects" it by accident.
//!
//! Extensions that want structured data go through `JSON.stringify` on their
//! side, which is why `LocalStorageService` also has `getItemAsJson` /
//! `setItemAsJson`: those store a *string* whose content is JSON, which the
//! three cases above handle.

pub mod schema;

use compass_sqlcipher_sys::Database;

/// The `value_type` discriminant, as it is written in the column.
///
/// `enum ValueType { Number, String, Boolean };` — unscoped and unnumbered, so
/// the values are 0, 1 and 2 in that order. They are on disk, so they cannot
/// be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum ValueType {
    /// A number, stored as its decimal text.
    Number = 0,
    /// A string, stored as itself. Also what an object, array or null becomes.
    String = 1,
    /// A boolean, stored as `"1"` or `"0"`.
    Boolean = 2,
}

impl ValueType {
    /// Reads a discriminant back.
    ///
    /// An unknown discriminant is `None` rather than a default: it means the
    /// row was written by something this build does not understand, and
    /// guessing would hand the extension a plausible wrong answer.
    #[must_use]
    pub fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Number),
            1 => Some(Self::String),
            2 => Some(Self::Boolean),
            _ => None,
        }
    }
}

/// A stored value: the text in the column and its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    /// The `value` column.
    pub text: String,
    /// The `value_type` column.
    pub kind: ValueType,
}

impl Value {
    /// What the C++ `serializeValue` would store for `json`.
    ///
    /// Anything that is not a string, number or boolean becomes an empty
    /// string — see the module docs.
    #[must_use]
    pub fn from_json(json: &serde_json::Value) -> Self {
        match json {
            serde_json::Value::String(s) => Self {
                text: s.clone(),
                kind: ValueType::String,
            },
            serde_json::Value::Number(n) => Self {
                text: format_number(n.as_f64().unwrap_or_default()),
                kind: ValueType::Number,
            },
            serde_json::Value::Bool(b) => Self {
                text: if *b { "1" } else { "0" }.to_owned(),
                kind: ValueType::Boolean,
            },
            _ => Self {
                text: String::new(),
                kind: ValueType::String,
            },
        }
    }

    /// What the C++ `deserializeValue` would hand back.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self.kind {
            ValueType::String => serde_json::Value::String(self.text.clone()),
            ValueType::Boolean => serde_json::Value::Bool(self.text == "1"),
            ValueType::Number => self
                .text
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
        }
    }
}

/// `QString::number(double)`'s output for `value`.
///
/// `QString::number(double n, char format = 'g', int precision = 6)` is C's
/// `%.6g`, and that is what is in the column. Rust's `{}` for `f64` prints the
/// shortest representation that round-trips, which is a *better* format and
/// therefore the wrong one here: `1234567.0` is `"1.23457e+06"` on disk, not
/// `"1234567"`, and the two engines have to read each other's rows.
///
/// `%.6g` with precision `P` = 6: let `X` be the decimal exponent. If
/// `P > X >= -4`, print as `%.*f` with `P - 1 - X` decimals; otherwise as
/// `%.*e` with `P - 1`. Either way trailing zeros go, and so does a trailing
/// point. `the_number_format_matches_printf` pins the whole thing against a
/// table taken from `printf '%.6g'`.
fn format_number(value: f64) -> String {
    const PRECISION: i32 = 6;

    if value == 0.0 {
        // `-0.0` prints as `-0` under %g; the sign survives the comparison
        // above, so it has to be handled rather than short-circuited to "0".
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    if !value.is_finite() {
        // Qt prints "inf"/"nan"; neither can reach here from JSON, which has
        // no such literals, but a database row could carry one.
        return format!("{value}");
    }

    // The exponent %g decides with, which is the one `%e` would print --
    // taken from the rounded `%e` rather than from `log10`, because a value
    // that rounds up across a power of ten (999999.5 -> 1e+06) changes it.
    let scientific = format!("{:.*e}", (PRECISION - 1) as usize, value);
    let exponent: i32 = scientific
        .split('e')
        .nth(1)
        .and_then(|e| e.parse().ok())
        .unwrap_or(0);

    if (-4..PRECISION).contains(&exponent) {
        let decimals = usize::try_from(PRECISION - 1 - exponent).unwrap_or(0);
        let fixed = format!("{value:.decimals$}");
        return trim_trailing_zeros(&fixed);
    }

    // Rust writes `1.23457e6`; C and Qt write `1.23457e+06`.
    let (mantissa, _) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let mantissa = trim_trailing_zeros(mantissa);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

/// Drops trailing zeros in a fractional part, and a point left bare.
fn trim_trailing_zeros(text: &str) -> String {
    if !text.contains('.') {
        return text.to_owned();
    }
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The namespace an extension's storage lives in.
///
/// `QString("%1:data").arg(m_command->uniqueId().provider.c_str())` in
/// `ExtensionCommandRuntime::initialize`.
#[must_use]
pub fn namespace_for(provider: &str) -> String {
    format!("{provider}:data")
}

/// Something wrong with reading or writing the store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database refused something.
    #[error(transparent)]
    Database(#[from] compass_sqlcipher_sys::Error),

    /// A row carries a `value_type` this build does not know.
    #[error(
        "the row {namespace}/{key} has value_type {found}, which this build does not know. It \
         was probably written by a newer Compass."
    )]
    UnknownValueType {
        /// Which namespace.
        namespace: String,
        /// Which key.
        key: String,
        /// The discriminant found.
        found: i64,
    },
}

/// The whole store, across every namespace.
#[derive(Debug)]
pub struct LocalStorage<'a> {
    db: &'a Database,
}

impl<'a> LocalStorage<'a> {
    /// Wraps an already-migrated `vicinae` database.
    #[must_use]
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// One namespace's view of it, which is all an extension ever sees.
    #[must_use]
    pub fn scoped(&self, namespace: &str) -> Scoped<'_> {
        Scoped {
            storage: self,
            namespace: namespace.to_owned(),
        }
    }

    /// Every namespace that has at least one row.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails.
    pub fn namespaces(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .db
            .prepare("SELECT DISTINCT(namespace_id) FROM storage_data_item")?;
        let mut out = Vec::new();
        while stmt.step()? {
            out.push(stmt.column_text(0).unwrap_or_default());
        }
        Ok(out)
    }
}

/// One extension's namespace.
#[derive(Debug)]
pub struct Scoped<'a> {
    storage: &'a LocalStorage<'a>,
    namespace: String,
}

impl Scoped<'_> {
    /// The namespace this is scoped to.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Reads `key`, or `None` if it is not set.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails, [`Error::UnknownValueType`] if
    /// the row was written by a newer build.
    pub fn get(&self, key: &str) -> Result<Option<Value>, Error> {
        let mut stmt = self.storage.db.prepare(
            "SELECT value, value_type FROM storage_data_item \
             WHERE namespace_id = :namespace_id AND key = :key",
        )?;
        stmt.bind_text(":namespace_id", &self.namespace)?;
        stmt.bind_text(":key", key)?;

        if !stmt.step()? {
            return Ok(None);
        }
        let text = stmt.column_text(0).unwrap_or_default();
        let raw = stmt.column_int64(1);
        let kind = ValueType::from_i64(raw).ok_or_else(|| Error::UnknownValueType {
            namespace: self.namespace.clone(),
            key: key.to_owned(),
            found: raw,
        })?;
        Ok(Some(Value { text, kind }))
    }

    /// Writes `key`, replacing whatever was there.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the write fails.
    pub fn set(&self, key: &str, value: &Value) -> Result<(), Error> {
        let mut stmt = self.storage.db.prepare(
            "INSERT INTO storage_data_item (namespace_id, key, value, value_type) \
             VALUES (:namespace_id, :key, :value, :value_type) \
             ON CONFLICT (namespace_id, key) DO UPDATE SET value = :value, \
             value_type = :value_type",
        )?;
        stmt.bind_text(":namespace_id", &self.namespace)?;
        stmt.bind_text(":key", key)?;
        stmt.bind_text(":value", &value.text)?;
        stmt.bind_int64(":value_type", value.kind as i64)?;
        stmt.step()?;
        Ok(())
    }

    /// Removes `key`, answering whether it was there.
    ///
    /// The C++ answers with `db.changes() != 0`. This crate's SQLite wrapper
    /// does not expose `changes()`, so the answer is read before the delete,
    /// inside one transaction — which makes it the same answer, not a
    /// near-enough one.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the delete fails.
    pub fn remove(&self, key: &str) -> Result<bool, Error> {
        let tx = self.storage.db.transaction()?;
        let existed = self.get(key)?.is_some();

        let mut stmt = self.storage.db.prepare(
            "DELETE FROM storage_data_item WHERE namespace_id = :namespace_id AND key = :key",
        )?;
        stmt.bind_text(":namespace_id", &self.namespace)?;
        stmt.bind_text(":key", key)?;
        stmt.step()?;
        drop(stmt);

        tx.commit()?;
        Ok(existed)
    }

    /// Removes everything in this namespace, and nothing outside it.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the delete fails.
    pub fn clear(&self) -> Result<(), Error> {
        let mut stmt = self
            .storage
            .db
            .prepare("DELETE FROM storage_data_item WHERE namespace_id = :namespace_id")?;
        stmt.bind_text(":namespace_id", &self.namespace)?;
        stmt.step()?;
        Ok(())
    }

    /// Everything in this namespace.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails, [`Error::UnknownValueType`] if a
    /// row was written by a newer build.
    pub fn list(&self) -> Result<Vec<(String, Value)>, Error> {
        let mut stmt = self.storage.db.prepare(
            "SELECT key, value, value_type FROM storage_data_item WHERE namespace_id = :namespace_id",
        )?;
        stmt.bind_text(":namespace_id", &self.namespace)?;

        let mut out = Vec::new();
        while stmt.step()? {
            let key = stmt.column_text(0).unwrap_or_default();
            let text = stmt.column_text(1).unwrap_or_default();
            let raw = stmt.column_int64(2);
            let kind = ValueType::from_i64(raw).ok_or_else(|| Error::UnknownValueType {
                namespace: self.namespace.clone(),
                key: key.clone(),
                found: raw,
            })?;
            out.push((key, Value { text, kind }));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = Database::open(&dir.path().join("vicinae.db"), &[]).expect("an unencrypted db");
        schema::run(&db).expect("the migrations apply");
        (dir, db)
    }

    #[test]
    fn the_namespace_is_the_one_the_cpp_builds() {
        // `QString("%1:data").arg(provider)`. An extension whose namespace is
        // spelled differently by the two engines sees an empty store rather
        // than an error, which is the worst possible failure for a key/value
        // store people keep tokens in.
        assert_eq!(namespace_for("hackernews"), "hackernews:data");
    }

    #[test]
    fn a_value_round_trips_through_the_database() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let scoped = storage.scoped(&namespace_for("hn"));

        // Numbers come back as doubles, integer-valued or not: the C++ reads
        // them with `QJsonValue::toDouble` and there is one numeric type on
        // the way back. `42` in, `42.0` out.
        for (json, expected) in [
            (serde_json::json!("a string"), serde_json::json!("a string")),
            (serde_json::json!(42), serde_json::json!(42.0)),
            (serde_json::json!(-1.5), serde_json::json!(-1.5)),
            (serde_json::json!(true), serde_json::json!(true)),
            (serde_json::json!(false), serde_json::json!(false)),
        ] {
            let value = Value::from_json(&json);
            scoped.set("k", &value).expect("the write succeeds");
            let read = scoped.get("k").expect("the read succeeds").expect("a row");
            assert_eq!(read, value, "{json} did not survive the round trip");
            assert_eq!(read.to_json(), expected, "{json} decoded to something else");
        }
    }

    #[test]
    fn store_an_object_and_it_comes_back_empty() {
        // Not a bug this port fixes -- see the module docs. `serializeValue`
        // falls through to `{"", ValueType::String}` for anything that is not a
        // string, number or boolean, and a database written by either engine is
        // read by the other.
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let scoped = storage.scoped("x:data");

        for json in [
            serde_json::json!({ "a": 1 }),
            serde_json::json!([1, 2, 3]),
            serde_json::Value::Null,
        ] {
            let value = Value::from_json(&json);
            assert_eq!(
                value,
                Value {
                    text: String::new(),
                    kind: ValueType::String
                },
                "{json} should have been flattened to an empty string"
            );
            scoped.set("k", &value).expect("the write succeeds");
            assert_eq!(
                scoped.get("k").expect("read").expect("a row").to_json(),
                serde_json::json!("")
            );
        }
    }

    #[test]
    fn the_value_type_discriminants_are_the_ones_on_disk() {
        // `enum ValueType { Number, String, Boolean };` -- unscoped and
        // unnumbered. Renumbering these would silently reinterpret every
        // existing row: a string would come back as a boolean.
        assert_eq!(ValueType::Number as i64, 0);
        assert_eq!(ValueType::String as i64, 1);
        assert_eq!(ValueType::Boolean as i64, 2);

        assert_eq!(ValueType::from_i64(0), Some(ValueType::Number));
        assert_eq!(ValueType::from_i64(1), Some(ValueType::String));
        assert_eq!(ValueType::from_i64(2), Some(ValueType::Boolean));
        assert_eq!(
            ValueType::from_i64(3),
            None,
            "an unknown discriminant must not be guessed at"
        );
    }

    #[test]
    fn a_row_from_a_newer_build_is_refused_rather_than_guessed_at() {
        let (_dir, db) = open();
        db.execute(
            "INSERT INTO storage_data_item (namespace_id, value_type, key, value) \
             VALUES ('x:data', 9, 'k', 'v')",
        )
        .expect("a row this build does not understand");

        let storage = LocalStorage::new(&db);
        match storage.scoped("x:data").get("k") {
            Err(Error::UnknownValueType { found, key, .. }) => {
                assert_eq!(found, 9);
                assert_eq!(key, "k");
            }
            other => panic!("an unknown value_type must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_boolean_is_one_or_zero_and_anything_else_is_false() {
        // `deserializeValue` is `value == "1"`, so a row holding "true" reads
        // back as false. Reproduced rather than improved, for the same reason
        // as the empty-object case.
        assert_eq!(
            Value {
                text: "1".to_owned(),
                kind: ValueType::Boolean
            }
            .to_json(),
            serde_json::json!(true)
        );
        for text in ["0", "true", "", "yes"] {
            assert_eq!(
                Value {
                    text: text.to_owned(),
                    kind: ValueType::Boolean
                }
                .to_json(),
                serde_json::json!(false),
                "{text:?} should read back as false"
            );
        }
    }

    #[test]
    fn the_number_format_matches_printf() {
        // Taken from `printf '%.6g'`, which is what
        // `QString::number(double, 'g', 6)` is. Rust's own `{}` disagrees with
        // several of these, which is the entire reason `format_number` exists.
        for (value, expected) in [
            (0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (42.0, "42"),
            (100.0, "100"),
            (1_000_000.0, "1e+06"),
            (1_234_567.0, "1.23457e+06"),
            (123_456.0, "123456"),
            (0.5, "0.5"),
            (0.1, "0.1"),
            (std::f64::consts::PI, "3.14159"),
            (0.000_001, "1e-06"),
            (0.000_000_1, "1e-07"),
            (1e15, "1e+15"),
            (1e-5, "1e-05"),
            (1e-4, "0.0001"),
            (2.5, "2.5"),
            (-0.000_123_456_789, "-0.000123457"),
            (999_999.5, "1e+06"),
            (1e300, "1e+300"),
        ] {
            assert_eq!(
                format_number(value),
                expected,
                "{value} formatted wrongly; the column would disagree with the C++ engine"
            );
        }
    }

    #[test]
    fn a_number_is_stored_in_that_format() {
        let value = Value::from_json(&serde_json::json!(1_234_567.0));
        assert_eq!(value.text, "1.23457e+06");
        assert_eq!(value.kind, ValueType::Number);
        // And it reads back as the rounded number, not the original: six
        // significant digits is what the column holds.
        assert_eq!(value.to_json(), serde_json::json!(1_234_570.0));
    }

    #[test]
    fn a_namespace_sees_only_its_own_rows() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let mine = storage.scoped("mine:data");
        let theirs = storage.scoped("theirs:data");

        let value = Value::from_json(&serde_json::json!("v"));
        mine.set("k", &value).expect("write");
        theirs.set("k", &value).expect("write");

        assert_eq!(mine.list().expect("list").len(), 1);
        mine.clear().expect("clear");
        assert!(mine.list().expect("list").is_empty());
        assert_eq!(
            theirs.list().expect("list").len(),
            1,
            "clearing one namespace emptied another"
        );
    }

    #[test]
    fn removing_says_whether_the_key_was_there() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let scoped = storage.scoped("x:data");

        assert!(
            !scoped.remove("k").expect("removing a missing key"),
            "a key that was never set was reported as removed"
        );

        scoped
            .set("k", &Value::from_json(&serde_json::json!("v")))
            .expect("write");
        assert!(scoped.remove("k").expect("removing a present key"));
        assert_eq!(scoped.get("k").expect("read"), None);
    }

    #[test]
    fn setting_twice_replaces_rather_than_conflicting() {
        // The primary key is (namespace_id, key), so a plain INSERT would fail
        // on the second write. The C++ uses ON CONFLICT DO UPDATE.
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let scoped = storage.scoped("x:data");

        scoped
            .set("k", &Value::from_json(&serde_json::json!("first")))
            .expect("write");
        scoped
            .set("k", &Value::from_json(&serde_json::json!(2)))
            .expect("overwrite");

        let read = scoped.get("k").expect("read").expect("a row");
        assert_eq!(read.to_json(), serde_json::json!(2.0));
        assert_eq!(read.kind, ValueType::Number, "the type was not updated");
        assert_eq!(scoped.list().expect("list").len(), 1);
    }

    #[test]
    fn namespaces_lists_what_has_rows() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let value = Value::from_json(&serde_json::json!("v"));
        storage.scoped("a:data").set("k", &value).expect("write");
        storage.scoped("b:data").set("k", &value).expect("write");

        let mut namespaces = storage.namespaces().expect("namespaces");
        namespaces.sort();
        assert_eq!(namespaces, vec!["a:data".to_owned(), "b:data".to_owned()]);
    }

    #[test]
    fn a_missing_key_is_none_rather_than_an_empty_value() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        // An extension that has never written must be able to tell "not set"
        // from "set to empty string" -- `LocalStorage.getItem` returns
        // undefined for the first and "" for the second.
        assert_eq!(storage.scoped("x:data").get("k").expect("read"), None);
        storage
            .scoped("x:data")
            .set("k", &Value::from_json(&serde_json::json!("")))
            .expect("write");
        assert_eq!(
            storage.scoped("x:data").get("k").expect("read"),
            Some(Value {
                text: String::new(),
                kind: ValueType::String
            })
        );
    }
}
