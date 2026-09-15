//! Export a retained session report composition (#1859, #1974): the file
//! exporter under the loop's base directory — `artifacts/session-exports`
//! is decided here, a runtime input of composition, never formed by a
//! dispatch branch — and the report use case with its controller over the
//! loop's active session.
use std::path::Path;
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::ports::export::SessionExportPort;
use crate::application::sessions::use_cases::ExportSessionReport;
use crate::infrastructure::session_export::FileSessionExport;
use crate::interface::uds::sessions::export_report_controller::ExportSessionReportController;

/// Where the loop of `base_dir` writes raw session exports.
pub fn build_session_export(base_dir: &Path) -> Arc<dyn SessionExportPort> {
    Arc::new(FileSessionExport::new(
        base_dir.join("artifacts/session-exports"),
    ))
}

/// The `get_report` controller of a loop over `active_session`, exporting
/// through `export` (`None`: raw exports refused as unavailable).
pub fn build_export_report(
    active_session: ActiveSessionHandle,
    export: Option<Arc<dyn SessionExportPort>>,
) -> Arc<ExportSessionReportController> {
    Arc::new(ExportSessionReportController::new(Arc::new(
        ExportSessionReport::new(active_session, export),
    )))
}

#[cfg(test)]
#[path = "session_report_tests.rs"]
mod tests;
