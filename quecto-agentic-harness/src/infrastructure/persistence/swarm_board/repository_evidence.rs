//! Pending (#2273).
use serde_json::Value;

use super::repository::SqliteBoard;
use crate::application::swarm::dto::{
    AmendedContract, CompletionState, NewEvidence, PriorEvidence, StoredContract,
};
use crate::application::swarm::ports::BoardEvidence;
use crate::domain::swarm::BoardError;

fn pending<T>() -> Result<T, BoardError> {
    Err(BoardError::new("pending #2273"))
}

impl BoardEvidence for SqliteBoard<'_> {
    fn completion_state(&self) -> Result<CompletionState, BoardError> {
        pending()
    }

    fn replace_task_evidence(&self, _id: &Value, _evidence: &Value) -> Result<(), BoardError> {
        pending()
    }

    fn delete_all_evidence(&self) -> Result<(), BoardError> {
        pending()
    }

    fn prior_evidence(
        &self,
        _criterion: &Value,
        _actor: &str,
    ) -> Result<Option<PriorEvidence>, BoardError> {
        pending()
    }

    fn record_evidence(&self, _evidence: &NewEvidence) -> Result<(), BoardError> {
        pending()
    }
}

pub(super) fn run_criteria(_board: &SqliteBoard<'_>) -> Result<Option<Value>, BoardError> {
    pending()
}

pub(super) fn run_contract(_board: &SqliteBoard<'_>) -> Result<Option<StoredContract>, BoardError> {
    pending()
}

pub(super) fn amend_contract(
    _board: &SqliteBoard<'_>,
    _contract: &AmendedContract,
) -> Result<(), BoardError> {
    pending()
}
