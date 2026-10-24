//! Unimplemented B3 Git history compilation shells. No Git command, hash,
//! transport, parsing, serialization or publication behavior is implemented.
use crate::application::project_board::ports::history::*;
use std::path::PathBuf;

pub struct GitBoardHistorySink {
    _repo: PathBuf,
    _target_ref: String,
}
impl GitBoardHistorySink {
    pub fn new(repo: PathBuf, target_ref: String) -> Self {
        Self {
            _repo: repo,
            _target_ref: target_ref,
        }
    }
}
impl BoardHistorySink for GitBoardHistorySink {
    fn raw(
        &self,
        _kind: HistoryKind,
        _bytes: &[u8],
        _expected_oid: Option<&str>,
        _dry_run: bool,
    ) -> HistoryResult<String> {
        Err(HistoryError::Unavailable(
            "B3 Git raw objects are not implemented (RED compilation shell)".into(),
        ))
    }
    fn tree(&self, _entries: &[HistoryEntry], _dry_run: bool) -> HistoryResult<String> {
        Err(HistoryError::Unavailable(
            "B3 Git tree serialization is not implemented (RED compilation shell)".into(),
        ))
    }
    fn commit(&self, _revision: HistoryCommit<'_>, _dry_run: bool) -> HistoryResult<String> {
        Err(HistoryError::Unavailable(
            "B3 Git commit serialization is not implemented (RED compilation shell)".into(),
        ))
    }
    fn publish(&self, _expected: Option<&str>, _new_head: &str) -> HistoryResult<()> {
        Err(HistoryError::Unavailable(
            "B3 Git lease publication is not implemented (RED compilation shell)".into(),
        ))
    }
    fn finish_dry_run(&self) -> HistoryResult<()> {
        Err(HistoryError::Unavailable(
            "B3 Git dry-run is not implemented (RED compilation shell)".into(),
        ))
    }
}
