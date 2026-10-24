//! Unimplemented B3 storage compilation shell. No open, schema, transaction,
//! mutation, snapshot or epoch processing. Fresh delivery must replace it.
use crate::application::project_board::ports::BoardHead;
use crate::application::project_board::ports::history::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use std::path::PathBuf;

pub struct SqliteBoardHistoryStore {
    _path: PathBuf,
    _writer: Identity,
    _board: BoardId,
}
impl SqliteBoardHistoryStore {
    pub fn new(path: PathBuf, writer: Identity, board: BoardId) -> Self {
        Self {
            _path: path,
            _writer: writer,
            _board: board,
        }
    }
}
impl SqliteBoardHistoryStore {
    pub fn open_epoch(
        &self,
        _expected_epoch: u64,
        _reason: &str,
        _at: i64,
    ) -> HistoryResult<BoardHead> {
        Err(HistoryError::Unavailable(
            "B3 epoch transition is not implemented (RED compilation shell)".into(),
        ))
    }
}
impl BoardHistoryRead for SqliteBoardHistoryStore {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>> {
        Err(HistoryError::Unavailable(
            "B3 pinned snapshot is not implemented (RED compilation shell)".into(),
        ))
    }
}
