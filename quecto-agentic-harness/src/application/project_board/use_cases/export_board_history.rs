//! Unimplemented B3 RED compilation shell; not supported export behavior.
use crate::application::project_board::ports::history::*;
use std::sync::Arc;

pub struct ExportBoardHistory {
    _store: Arc<dyn BoardHistoryRead>,
    _sink: Arc<dyn BoardHistorySink>,
}
impl ExportBoardHistory {
    pub fn new(store: Arc<dyn BoardHistoryRead>, sink: Arc<dyn BoardHistorySink>) -> Self {
        Self {
            _store: store,
            _sink: sink,
        }
    }
    pub fn execute(
        &self,
        _dry_run: bool,
        _expected: Option<&str>,
    ) -> HistoryResult<HistoryExportReport> {
        Err(HistoryError::Unavailable(
            "B3 export is not implemented (RED compilation shell)".into(),
        ))
    }
}
