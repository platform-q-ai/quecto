//! `BoardFiles` and `BoardMessages` over the SQLite store (#2272, #2275).
use serde_json::Value;

use super::repository::{SqliteBoard, loose};
use crate::application::swarm::dto::{FileRow, NewReservation};
use crate::application::swarm::ports::{BoardFiles, BoardMessages};
use crate::domain::swarm::BoardError;

fn unserved() -> BoardError {
    BoardError::new("not served yet")
}

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
        Err(unserved())
    }

    fn file_reserved(&self, _path: &str) -> Result<bool, BoardError> {
        Err(unserved())
    }

    fn insert_files(&self, _reservation: &NewReservation) -> Result<(), BoardError> {
        Err(unserved())
    }

    fn delete_reservation(
        &self,
        _task: &Value,
        _owner: &str,
        _claim: &Value,
        _token: &Value,
    ) -> Result<(), BoardError> {
        Err(unserved())
    }

    fn task_file_count(&self, _task: &Value) -> Result<i64, BoardError> {
        Err(unserved())
    }

    fn delete_task_files(&self, _task: &Value) -> Result<(), BoardError> {
        Err(unserved())
    }

    fn file_page(&self, _offset: u64, _limit: i64) -> Result<Vec<FileRow>, BoardError> {
        Err(unserved())
    }
}

impl BoardMessages for SqliteBoard<'_> {
    fn inbox_count(&self, _recipient: &Value) -> Result<i64, BoardError> {
        Err(unserved())
    }

    fn insert_message(
        &self,
        _sender: &str,
        _recipient: &Value,
        _body: &str,
    ) -> Result<i64, BoardError> {
        Err(unserved())
    }
}
