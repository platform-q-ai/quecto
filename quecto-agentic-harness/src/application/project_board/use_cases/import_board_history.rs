//! RED-only inward import use case; complete shared proof belongs to GREEN.
use crate::application::project_board::ports::history::*;
use std::sync::Arc;
pub struct ImportBoardHistory;
impl ImportBoardHistory {
    pub fn new(
        _: Arc<dyn BoardHistorySource>,
        _: Arc<dyn BoardHistoryStore>,
        _: Arc<dyn BoardHistorySink>,
    ) -> Self {
        Self
    }
    pub fn execute(&self) -> HistoryResult<HistoryImportReport> {
        Err(HistoryError::Unavailable(
            "B application import not implemented".into(),
        ))
    }
}
