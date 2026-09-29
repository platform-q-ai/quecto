//! `BoardFiles` over the SQLite store (#2272, #2275): the file
//! reservations, by Python's SQL (`swarm_tasks.py`). A task id, a claim
//! token or an ownership token is the caller's value, bound as Python's
//! `sqlite3` binds it ([`loose`]) and numbered by its position in Python's
//! statement, so the column affinity finds and stores rows as Python's
//! board does, and a value Python cannot bind is refused with its text.
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::binding;
use super::repository::{SqliteBoard, cell_at, failed, fetched, loose};
use crate::application::swarm::dto::{FileRow, NewReservation};
use crate::application::swarm::ports::BoardFiles;
use crate::domain::swarm::{BoardError, RefusalKind};

impl BoardFiles for SqliteBoard<'_> {
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError> {
        // Any number of reservations, none included.
        self.run(
            "DELETE FROM files WHERE task=? AND claim=?",
            &[loose(1, task)?, loose(2, claim)?],
        )
        .map(|_| ())
    }

    fn file_count(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM files")
    }

    fn file_reserved(&self, path: &str) -> Result<bool, BoardError> {
        let parameters = [SqlValue::Text(path.to_owned())];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT 1 FROM files WHERE path=?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        rows.next().map(|row| row.is_some()).map_err(failed)
    }

    fn insert_files(&self, reservation: &NewReservation) -> Result<(), BoardError> {
        debug_assert!(
            reservation.paths.windows(2).all(|pair| pair[0] < pair[1]),
            "the paths are distinct and sorted"
        );
        let task = loose(2, &reservation.task)?;
        let claim = loose(4, &reservation.claim)?;
        for path in &reservation.paths {
            let inserted = self.run(
                "INSERT INTO files VALUES(?,?,?,?,?)",
                &[
                    SqlValue::Text(path.clone()),
                    task.clone(),
                    SqlValue::Text(reservation.owner.clone()),
                    claim.clone(),
                    SqlValue::Text(reservation.token.clone()),
                ],
            )?;
            debug_assert_eq!(inserted, 1, "one VALUES row inserts one reservation");
        }
        Ok(())
    }

    fn delete_reservation(
        &self,
        task: &Value,
        owner: &str,
        claim: &Value,
        token: &Value,
    ) -> Result<(), BoardError> {
        // Any number of reservations, none included.
        self.run(
            "DELETE FROM files WHERE task=? AND owner=? AND claim=? AND token=?",
            &[
                loose(1, task)?,
                SqlValue::Text(owner.to_owned()),
                loose(3, claim)?,
                loose(4, token)?,
            ],
        )
        .map(|_| ())
    }

    fn task_file_count(&self, task: &Value) -> Result<i64, BoardError> {
        self.bound_count(
            "SELECT count(*) FROM files WHERE task=?",
            &[loose(1, task)?],
        )
    }

    fn delete_task_files(&self, task: &Value) -> Result<(), BoardError> {
        self.run("DELETE FROM files WHERE task=?", &[loose(1, task)?])
            .map(|_| ())
    }

    fn owner_file_count(&self, owner: &Value) -> Result<i64, BoardError> {
        self.owned_file_count(owner)
    }

    fn delete_owner_files(&self, owner: &Value) -> Result<(), BoardError> {
        self.delete_owned_files(owner)
    }

    fn file_page(&self, offset: u64, limit: i64) -> Result<Vec<FileRow>, BoardError> {
        let parameters = [SqlValue::Integer(limit), loose(2, &Value::from(offset))?];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT * FROM files ORDER BY path LIMIT ? OFFSET ?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        let mut page = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            fetched(row).map_err(failed)?;
            let columns = (0..row.as_ref().column_count())
                .map(|index| {
                    Ok((
                        row.as_ref().column_name(index)?.to_owned(),
                        cell_at(row, index)?,
                    ))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(failed)?;
            page.push(FileRow { columns });
        }
        debug_assert!(
            page.len() <= usize::try_from(limit).unwrap_or(usize::MAX),
            "a page holds at most its limit"
        );
        Ok(page)
    }
}

impl SqliteBoard<'_> {
    /// A `count(*)` with its parameters already bound as Python binds
    /// them.
    pub(super) fn bound_count(
        &self,
        sql: &str,
        parameters: &[SqlValue],
    ) -> Result<i64, BoardError> {
        debug_assert!(sql.starts_with("SELECT count(*) "), "a count: {sql}");
        let mut statement =
            binding::bound_statement(self.connection, sql, parameters).map_err(failed)?;
        let mut rows = statement.raw_query();
        match rows.next().map_err(failed)? {
            Some(row) => row.get(0).map_err(failed),
            None => Err(BoardError::new(
                RefusalKind::Internal,
                "a count answered no row",
            )),
        }
    }
}
