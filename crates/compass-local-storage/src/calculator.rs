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

/// The names the history is grouped under, in order.
///
/// Every group is produced on every call, including the empty ones — the view
/// is what drops those. Splitting it that way is the C++'s, and it means a
/// caller can rely on the shape of the result without checking which groups
/// happen to have rows.
pub const GROUP_NAMES: &[&str] = &[
    "Pinned",
    "Today",
    "This week",
    "This month",
    "This year",
    "A few years ago",
];

/// The instants that separate the time groups, as seconds since the epoch.
///
/// The C++ computes these from `QDateTime::currentDateTime()` against the
/// local calendar. They are passed in here instead: the calendar arithmetic
/// belongs to whoever owns the clock, and keeping it out is what lets the
/// grouping itself — which is the part with the surprises in it — be tested
/// without freezing a timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBoundaries {
    /// Midnight at the start of today.
    pub start_of_day: i64,
    /// The last instant of today.
    pub end_of_day: i64,
    /// Midnight on the first day of this week.
    pub start_of_week: i64,
    /// Midnight seven days after that.
    pub end_of_week: i64,
    /// Midnight on the first day of this month.
    pub start_of_month: i64,
    /// Midnight on the first day of next month.
    pub end_of_month: i64,
    /// Midnight on the first day of this year.
    pub start_of_year: i64,
    /// Midnight on the first day of next year.
    pub end_of_year: i64,
}

/// One named group of history rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// What the section is called.
    pub name: String,
    /// The rows in it.
    pub records: Vec<Record>,
}

/// Group history rows by when they were made.
///
/// This is a **sequential scan, not a classification**, and the difference
/// matters. The rows arrive in the query's order — pinned first, then newest
/// first — and each group takes a *prefix*: it consumes rows while they match
/// and stops at the first that does not, leaving the rest to the next group.
/// Nothing rewinds and nothing is reconsidered.
///
/// Two things follow that a per-row classification would not do. A row is
/// tested against each later group in turn until one takes it, so a row older
/// than every boundary ends up in `A few years ago` without any group needing
/// a lower bound. And a row out of order — newer than one already placed —
/// cannot go back to an earlier group; it lands wherever the scan has reached.
/// The query's `ORDER BY` is what makes that safe, so the two are one
/// mechanism and not two.
#[must_use]
pub fn group_records_by_time(records: &[Record], at: TimeBoundaries) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::with_capacity(GROUP_NAMES.len());
    let mut index = 0usize;

    let mut take = |name: &str, index: &mut usize, keep: &dyn Fn(&Record) -> bool| {
        let mut taken = Vec::new();
        while *index < records.len() && keep(&records[*index]) {
            taken.push(records[*index].clone());
            *index += 1;
        }
        groups.push(Group {
            name: name.to_owned(),
            records: taken,
        });
    };

    take("Pinned", &mut index, &|r| r.pinned_at.is_some());
    take("Today", &mut index, &|r| {
        r.created_at >= at.start_of_day && r.created_at <= at.end_of_day
    });
    take("This week", &mut index, &|r| {
        r.created_at >= at.start_of_week && r.created_at <= at.end_of_week
    });
    take("This month", &mut index, &|r| {
        r.created_at >= at.start_of_month && r.created_at <= at.end_of_month
    });
    take("This year", &mut index, &|r| {
        r.created_at >= at.start_of_year && r.created_at <= at.end_of_year
    });
    take("A few years ago", &mut index, &|_| true);

    groups
}

/// Filter history rows by a search query.
///
/// An empty query returns everything, untouched and in the query's own order —
/// it is not a search that happens to match all rows, it is a short circuit,
/// and it is what makes opening the history with an empty search box show the
/// grouped list rather than a fuzzy-ranked one.
///
/// Otherwise every row whose question (weight 1.0) or answer (weight 0.5)
/// matches is kept. The rows are *filtered*, not *ranked*: the query order is
/// preserved, so the pinned-first grouping downstream still works.
#[must_use]
pub fn query_records(records: &[Record], query: &str) -> Vec<Record> {
    if query.is_empty() {
        return records.to_vec();
    }

    let parsed = compass_search::Query::new(query);
    records
        .iter()
        .filter(|record| {
            let fields = [
                compass_search::WeightedField::new(&record.question, 1.0),
                compass_search::WeightedField::new(&record.answer, 0.5),
            ];
            compass_search::score_weighted(&fields, &parsed).accepted()
        })
        .cloned()
        .collect()
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

#[cfg(test)]
mod grouping_tests {
    use super::*;

    const DAY: i64 = 86_400;

    /// A notional "now". It has to be a realistic epoch: with a small one the
    /// start-of-year boundary lands before the epoch itself, and a row dated
    /// zero is then swept into "This year" — which cannot happen against a
    /// real clock, so a fixture that allows it tests nothing.
    const NOW: i64 = 1_700_000_000;

    /// Boundaries around [`NOW`], spaced in round numbers of days so the
    /// arithmetic in a failing test is readable.
    fn at() -> TimeBoundaries {
        TimeBoundaries {
            start_of_day: NOW,
            end_of_day: NOW + DAY - 1,
            start_of_week: NOW - 3 * DAY,
            end_of_week: NOW + 4 * DAY,
            start_of_month: NOW - 20 * DAY,
            end_of_month: NOW + 10 * DAY,
            start_of_year: NOW - 200 * DAY,
            end_of_year: NOW + 165 * DAY,
        }
    }

    fn record(id: &str, created_at: i64, pinned_at: Option<i64>) -> Record {
        Record {
            id: id.to_owned(),
            answer_type: AnswerType::Normal,
            question: "1+1".to_owned(),
            answer: "2".to_owned(),
            created_at,
            pinned_at,
        }
    }

    fn ids(groups: &[Group], name: &str) -> Vec<String> {
        groups
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.records.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn every_group_is_produced_even_when_empty() {
        // The view drops the empty ones; the grouping does not, so a caller
        // can rely on the shape of the result.
        let groups = group_records_by_time(&[], at());
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, GROUP_NAMES);
        assert!(groups.iter().all(|g| g.records.is_empty()));
    }

    #[test]
    fn pinned_rows_lead() {
        let records = vec![
            record("a", NOW + 100, Some(5)),
            record("b", NOW + 200, None),
        ];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Pinned"), ["a"]);
        assert_eq!(ids(&groups, "Today"), ["b"]);
    }

    #[test]
    fn a_pinned_row_is_not_also_counted_by_its_date() {
        // Each group consumes a prefix and nothing is reconsidered, so a
        // pinned row made today appears once, under Pinned.
        let records = vec![record("a", NOW + 100, Some(5))];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Pinned"), ["a"]);
        assert_eq!(ids(&groups, "Today"), Vec::<String>::new());
    }

    #[test]
    fn the_pinned_run_stops_at_the_first_unpinned_row() {
        // The query sorts pinned rows first, so the scan can stop rather than
        // filter — and a pinned row after an unpinned one would be missed.
        // That is the query's `ORDER BY` and this scan being one mechanism.
        let records = vec![
            record("a", NOW + 100, Some(5)),
            record("b", NOW + 200, None),
            record("c", NOW + 300, Some(6)),
        ];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Pinned"), ["a"]);
        assert_eq!(ids(&groups, "Today"), ["b", "c"]);
    }

    #[test]
    fn a_row_from_today_lands_in_today() {
        let records = vec![record("a", NOW + 500, None)];
        assert_eq!(ids(&group_records_by_time(&records, at()), "Today"), ["a"]);
    }

    #[test]
    fn the_last_second_of_today_is_still_today() {
        let records = vec![record("a", at().end_of_day, None)];
        assert_eq!(ids(&group_records_by_time(&records, at()), "Today"), ["a"]);
    }

    #[test]
    fn a_row_from_earlier_this_week_falls_to_the_week_group() {
        let records = vec![record("a", NOW - 2 * DAY, None)];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Today"), Vec::<String>::new());
        assert_eq!(ids(&groups, "This week"), ["a"]);
    }

    #[test]
    fn a_row_older_than_every_boundary_falls_all_the_way_through() {
        // No group needs a lower bound: a row that fails one is offered to the
        // next, and the last group takes whatever is left.
        let records = vec![record("a", 0, None)];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "A few years ago"), ["a"]);
    }

    #[test]
    fn each_group_takes_a_prefix_and_then_stops() {
        let records = vec![
            record("today", NOW + 100, None),
            record("week", NOW - 2 * DAY, None),
            record("month", NOW - 10 * DAY, None),
            record("year", NOW - 100 * DAY, None),
            record("ancient", 0, None),
        ];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Today"), ["today"]);
        assert_eq!(ids(&groups, "This week"), ["week"]);
        assert_eq!(ids(&groups, "This month"), ["month"]);
        assert_eq!(ids(&groups, "This year"), ["year"]);
        assert_eq!(ids(&groups, "A few years ago"), ["ancient"]);
    }

    #[test]
    fn a_row_out_of_order_cannot_go_back_to_an_earlier_group() {
        // Nothing rewinds. A today row listed after an older one lands
        // wherever the scan has reached — here, in the week group. The
        // query's ordering is what keeps that from happening, which is why
        // the two cannot be reasoned about separately.
        let records = vec![
            record("old", NOW - 2 * DAY, None),
            record("new", NOW + 100, None),
        ];
        let groups = group_records_by_time(&records, at());
        assert_eq!(ids(&groups, "Today"), Vec::<String>::new());
        assert_eq!(ids(&groups, "This week"), ["old", "new"]);
    }

    #[test]
    fn several_rows_in_one_group_keep_their_order() {
        let records = vec![
            record("a", NOW + 300, None),
            record("b", NOW + 200, None),
            record("c", NOW + 100, None),
        ];
        assert_eq!(
            ids(&group_records_by_time(&records, at()), "Today"),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn an_empty_query_returns_everything_untouched() {
        // It is a short circuit, not a search that matches all rows — which is
        // what makes an empty search box show the grouped list rather than a
        // fuzzy-ranked one.
        let records = vec![record("a", 1, None), record("b", 2, None)];
        assert_eq!(query_records(&records, ""), records);
    }

    #[test]
    fn a_query_matches_the_question() {
        let mut r = record("a", 1, None);
        r.question = "cups to litres".to_owned();
        let records = vec![r];
        assert_eq!(query_records(&records, "litres").len(), 1);
    }

    #[test]
    fn a_query_matches_the_answer_too() {
        let mut r = record("a", 1, None);
        r.question = "x".to_owned();
        r.answer = "seventeen".to_owned();
        let records = vec![r];
        assert_eq!(query_records(&records, "seventeen").len(), 1);
    }

    #[test]
    fn a_query_that_matches_nothing_returns_nothing() {
        let records = vec![record("a", 1, None)];
        assert!(query_records(&records, "zzzzzzz").is_empty());
    }

    #[test]
    fn matching_rows_keep_the_querys_order_rather_than_being_ranked() {
        // They are filtered, not ranked — the pinned-first grouping downstream
        // depends on that order surviving.
        let mut a = record("a", 3, Some(1));
        a.question = "two cups".to_owned();
        let mut b = record("b", 2, None);
        b.question = "cups".to_owned();
        let records = vec![a, b];
        let out = query_records(&records, "cups");
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }
}
