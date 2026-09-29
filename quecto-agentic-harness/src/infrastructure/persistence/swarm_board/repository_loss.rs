//! The loss and death statements over the SQLite store (#2277): red-phase
//! stub.
use serde_json::Value;

use super::repository::SqliteBoard;
use crate::application::swarm::dto::ScopeObservation;
use crate::domain::swarm::BoardError;

fn pending() -> BoardError {
    todo!("S12 serves the loss statements")
}

impl SqliteBoard<'_> {
    pub(super) fn launcher_of(&self, _id: &Value) -> Result<Option<Option<String>>, BoardError> {
        Err(pending())
    }

    pub(super) fn lost_since_activation(&self, _member: &Value) -> Result<bool, BoardError> {
        Err(pending())
    }

    pub(super) fn observations(&self) -> Result<Vec<ScopeObservation>, BoardError> {
        Err(pending())
    }

    pub(super) fn block_owned(&self, _owner: &Value, _blocker: &str) -> Result<(), BoardError> {
        Err(pending())
    }

    pub(super) fn owned_file_count(&self, _owner: &Value) -> Result<i64, BoardError> {
        Err(pending())
    }

    pub(super) fn delete_owned_files(&self, _owner: &Value) -> Result<(), BoardError> {
        Err(pending())
    }

    pub(super) fn failed_hold(&self, _reason: &str) -> Result<(), BoardError> {
        Err(pending())
    }
}
