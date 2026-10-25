//! Capture, prepare, and prove the complete uncommitted history before commit.
use super::ExportBoardHistory;
use crate::application::project_board::ports::history::*;
use std::sync::Arc;
pub struct ImportBoardHistory {
    source: Arc<dyn BoardHistorySource>,
    store: Arc<dyn BoardHistoryStore>,
    export: ExportBoardHistory,
}
impl ImportBoardHistory {
    pub fn new(
        source: Arc<dyn BoardHistorySource>,
        store: Arc<dyn BoardHistoryStore>,
        export: ExportBoardHistory,
    ) -> Self {
        Self {
            source,
            store,
            export,
        }
    }
    pub fn execute(&self) -> HistoryResult<HistoryImportReport> {
        let history = self.source.capture()?;
        let prepared = self.store.prepare_import(&history)?;
        let proof = self.export.execute_snapshot(&*prepared, true, None)?;
        if proof.revisions
            == u64::try_from(history.revisions.len())
                .map_err(|_| HistoryError::Bounds("import count".into()))?
            && proof.git_head == history.source_head
            && proof.frontier == prepared.frontier()?
        {
        } else {
            return Err(HistoryError::Malformed(
                "full import self-proof mismatch".into(),
            ));
        }
        for original in &history.objects {
            if prepared.object(&original.oid)? == *original {
            } else {
                return Err(HistoryError::Malformed(
                    "prepared original object mismatch".into(),
                ));
            }
        }
        prepared.commit()
    }
}
