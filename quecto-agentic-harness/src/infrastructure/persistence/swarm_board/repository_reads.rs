//! The read models' statements over the SQLite store (#2277): red-phase
//! stub.
use serde_json::Value;

use super::repository::SqliteBoard;
use crate::application::swarm::dto::{DictRow, LatestActivity, TaskState};
use crate::domain::swarm::BoardError;

fn pending() -> BoardError {
    todo!("S12 serves the read models' statements")
}

impl SqliteBoard<'_> {
    pub(super) fn run_dict(&self) -> Result<Option<DictRow>, BoardError> {
        Err(pending())
    }

    pub(super) fn statuses_of(
        &self,
        _ids: &[&str],
    ) -> Result<Vec<(String, Option<String>)>, BoardError> {
        Err(pending())
    }

    pub(super) fn latest_of(&self, _actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError> {
        Err(pending())
    }

    pub(super) fn time_of(&self, _id: i64) -> Result<Option<Value>, BoardError> {
        Err(pending())
    }

    pub(super) fn events_after(
        &self,
        _after: u64,
        _limit: i64,
    ) -> Result<Vec<DictRow>, BoardError> {
        Err(pending())
    }

    pub(super) fn evidence_dicts(&self) -> Result<Vec<DictRow>, BoardError> {
        Err(pending())
    }

    pub(super) fn ids_page(&self, _offset: u64, _limit: i64) -> Result<Vec<Value>, BoardError> {
        Err(pending())
    }

    pub(super) fn states(&self) -> Result<Vec<TaskState>, BoardError> {
        Err(pending())
    }

    pub(super) fn owners_of_claims(&self) -> Result<Vec<Value>, BoardError> {
        Err(pending())
    }
}
