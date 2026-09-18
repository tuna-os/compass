//! The calculator's history.
//!
//! Ports the persistence half of `CalculatorService`
//! (`src/server/src/services/calculator-service/calculator-service.cpp`): one
//! row per answer in the `vicinae` database's `calculator_history` table,
//! with pinning.
//!
//! # Not the arithmetic
//!
//! The C++ computes through a pluggable backend — qalculate, numen or
//! soulver-core — and which one a Rust engine should use is a decision with
//! licence, size and accuracy trade-offs that nobody has taken. This crate
//! stores answers and does not produce them, and says so rather than picking
//! a library by accident.
//!
//! # The order is the query's, not the caller's
//!
//! `ORDER BY pinned_at DESC, created_at DESC`. Pinned rows come first, newest
//! first within each group, and an unpinned row has `pinned_at` NULL — which
//! sorts *last* under `DESC` in SQLite. That is what puts pinned rows on top,
//! and it is the kind of thing that looks like an accident until it is
//! written down.

use compass_sqlcipher_sys::Database;

use crate::Error;

/// What kind of answer a row holds.
///
/// `CalculatorAnswerType` in the C++: `NORMAL` is 0 and `CONVERSION` is 1.
/// Both are on disk in `type_hint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum AnswerType {
    /// Ordinary arithmetic.
    Normal = 0,
    /// A unit or currency conversion.
    Conversion = 1,
}

impl AnswerType {
    /// Reads a `type_hint` back.
    ///
    /// The C++ casts the integer straight to the enum, so a value it does not
    /// know is undefined behaviour there. Here an unknown value is `None`.
    #[must_use]
    pub fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Conversion),
            _ => None,
        }
    }
}

/// One remembered calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The row's id. The C++ mints a braceless UUID.
    pub id: String,
    /// Arithmetic or conversion.
    pub answer_type: AnswerType,
    /// What was asked.
    pub question: String,
    /// What came back.
    pub answer: String,
    /// When, in seconds since the epoch.
    pub created_at: i64,
    /// When it was pinned, if it is.
    pub pinned_at: Option<i64>,
}

/// The history table.
#[derive(Debug)]
pub struct History<'a> {
    db: &'a Database,
}

impl<'a> History<'a> {
    /// Wraps an already-migrated `vicinae` database.
    #[must_use]
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Adds a row.
    ///
    /// `id` and `created_at` are the caller's: the C++ mints a UUID and reads
    /// the clock here, and passing them in is what makes this testable without
    /// a clock or a UUID generator in the dependency list.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the write fails.
    pub fn add(
        &self,
        id: &str,
        answer_type: AnswerType,
        question: &str,
        answer: &str,
        created_at: i64,
    ) -> Result<(), Error> {
        let mut stmt = self.db.prepare(
            "INSERT INTO calculator_history (id, type_hint, question, answer, created_at) \
             VALUES (:id, :type_hint, :question, :answer, :epoch)",
        )?;
        stmt.bind_text(":id", id)?;
        stmt.bind_int64(":type_hint", answer_type as i64)?;
        stmt.bind_text(":question", question)?;
        stmt.bind_text(":answer", answer)?;
        stmt.bind_int64(":epoch", created_at)?;
        stmt.step()?;
        Ok(())
    }

    /// Every row, pinned first and newest first.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the query fails, [`Error::UnknownValueType`] if
    /// a row carries a `type_hint` this build does not know.
    pub fn list(&self) -> Result<Vec<Record>, Error> {
        let mut stmt = self.db.prepare(
            "SELECT id, type_hint, question, answer, created_at, pinned_at \
             FROM calculator_history ORDER BY pinned_at DESC, created_at DESC",
        )?;

        let mut out = Vec::new();
        while stmt.step()? {
            let id = stmt.column_text(0).unwrap_or_default();
            let raw = stmt.column_int64(1);
            let answer_type = AnswerType::from_i64(raw).ok_or_else(|| Error::UnknownValueType {
                namespace: "calculator_history".to_owned(),
                key: id.clone(),
                found: raw,
            })?;

            out.push(Record {
                id,
                answer_type,
                question: stmt.column_text(2).unwrap_or_default(),
                answer: stmt.column_text(3).unwrap_or_default(),
                created_at: stmt.column_int64(4),
                pinned_at: (!stmt.is_null(5)).then(|| stmt.column_int64(5)),
            });
        }
        Ok(out)
    }

    /// Pins a row at `now`.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the write fails.
    pub fn pin(&self, id: &str, now: i64) -> Result<(), Error> {
        let mut stmt = self
            .db
            .prepare("UPDATE calculator_history SET pinned_at = :epoch WHERE id = :id")?;
        stmt.bind_text(":id", id)?;
        stmt.bind_int64(":epoch", now)?;
        stmt.step()?;
        Ok(())
    }

    /// Unpins a row.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the write fails.
    pub fn unpin(&self, id: &str) -> Result<(), Error> {
        let mut stmt = self
            .db
            .prepare("UPDATE calculator_history SET pinned_at = NULL WHERE id = :id")?;
        stmt.bind_text(":id", id)?;
        stmt.step()?;
        Ok(())
    }

    /// Removes one row.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the delete fails.
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let mut stmt = self
            .db
            .prepare("DELETE FROM calculator_history WHERE id = :id")?;
        stmt.bind_text(":id", id)?;
        stmt.step()?;
        Ok(())
    }

    /// Removes everything.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the delete fails.
    pub fn clear(&self) -> Result<(), Error> {
        self.db.execute("DELETE FROM calculator_history")?;
        Ok(())
    }
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

    fn add(history: &History<'_>, id: &str, created_at: i64) {
        history
            .add(id, AnswerType::Normal, "1+1", "2", created_at)
            .expect("write");
    }

    #[test]
    fn a_record_round_trips() {
        let (_dir, db) = open();
        let history = History::new(&db);
        history
            .add(
                "a",
                AnswerType::Conversion,
                "1 m in cm",
                "100 cm",
                1_700_000_000,
            )
            .expect("write");

        let rows = history.list().expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0],
            Record {
                id: "a".to_owned(),
                answer_type: AnswerType::Conversion,
                question: "1 m in cm".to_owned(),
                answer: "100 cm".to_owned(),
                created_at: 1_700_000_000,
                pinned_at: None,
            }
        );
    }

    #[test]
    fn pinned_rows_come_first_and_the_newest_leads_each_group() {
        // `ORDER BY pinned_at DESC, created_at DESC`, and NULL sorts last
        // under DESC in SQLite -- which is exactly what puts pinned rows on
        // top. Written down because it reads like an accident.
        let (_dir, db) = open();
        let history = History::new(&db);
        add(&history, "old", 100);
        add(&history, "new", 200);
        add(&history, "pinned-old", 50);
        add(&history, "pinned-new", 60);

        history.pin("pinned-old", 1_000).expect("pin");
        history.pin("pinned-new", 2_000).expect("pin");

        let ids: Vec<String> = history
            .list()
            .expect("list")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "pinned-new".to_owned(),
                "pinned-old".to_owned(),
                "new".to_owned(),
                "old".to_owned()
            ]
        );
    }

    #[test]
    fn unpinning_puts_a_row_back_among_the_rest() {
        let (_dir, db) = open();
        let history = History::new(&db);
        add(&history, "a", 100);
        add(&history, "b", 200);
        history.pin("a", 1_000).expect("pin");
        assert_eq!(history.list().expect("list")[0].id, "a");

        history.unpin("a").expect("unpin");
        let rows = history.list().expect("list");
        assert_eq!(
            rows[0].id, "b",
            "b is newer, so it leads once nothing is pinned"
        );
        assert_eq!(rows[1].pinned_at, None);
    }

    #[test]
    fn removing_takes_one_row_and_clearing_takes_them_all() {
        let (_dir, db) = open();
        let history = History::new(&db);
        add(&history, "a", 100);
        add(&history, "b", 200);

        history.remove("a").expect("remove");
        let rows = history.list().expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "b");

        history.clear().expect("clear");
        assert!(history.list().expect("list").is_empty());

        // Neither is an error when there is nothing to do, as the C++'s
        // DELETEs are not.
        history.remove("gone").expect("removing nothing");
        history.clear().expect("clearing nothing");
    }

    #[test]
    fn the_type_hints_are_the_ones_on_disk() {
        assert_eq!(AnswerType::Normal as i64, 0);
        assert_eq!(AnswerType::Conversion as i64, 1);
        assert_eq!(AnswerType::from_i64(0), Some(AnswerType::Normal));
        assert_eq!(AnswerType::from_i64(1), Some(AnswerType::Conversion));
        assert_eq!(
            AnswerType::from_i64(7),
            None,
            "the C++ casts blindly here; this must not"
        );
    }

    #[test]
    fn a_row_with_an_unknown_type_hint_is_refused_rather_than_guessed_at() {
        let (_dir, db) = open();
        db.execute(
            "INSERT INTO calculator_history (id, type_hint, question, answer, created_at) \
             VALUES ('x', 9, 'q', 'a', 1)",
        )
        .expect("a row from a newer build");

        match History::new(&db).list() {
            Err(Error::UnknownValueType { found, key, .. }) => {
                assert_eq!(found, 9);
                assert_eq!(key, "x");
            }
            other => panic!("an unknown type_hint must be refused, got {other:?}"),
        }
    }

    #[test]
    fn two_rows_cannot_share_an_id() {
        // `id` is the primary key, so the C++'s UUID is load-bearing.
        let (_dir, db) = open();
        let history = History::new(&db);
        add(&history, "a", 100);
        assert!(
            history
                .add("a", AnswerType::Normal, "2+2", "4", 200)
                .is_err(),
            "a duplicate id was accepted"
        );
    }
}
