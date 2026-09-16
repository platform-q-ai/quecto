//! Controller of the `get_report` command (#1859, #1974): maps the one
//! wire option (`export_raw`) onto the application's report request. The
//! idle dispatch loop asks for the report directly; the busy reader task
//! asks for a bounded admission first and awaits the admitted export off
//! its own task. No policy: what is reported, exported, refused or
//! bounded is the use case's.
use std::sync::Arc;

use crate::application::sessions::dto::{ReportError, SessionReport};
use crate::application::sessions::use_cases::ExportSessionReport;
use crate::application::sessions::use_cases::export_session_report::AdmittedExport;

pub struct ExportSessionReportController {
    export_report: Arc<ExportSessionReport>,
}

impl ExportSessionReportController {
    pub fn new(export_report: Arc<ExportSessionReport>) -> Self {
        Self { export_report }
    }

    /// The report, with a raw export when `export_raw` (the idle loop,
    /// which serialises its own requests).
    pub async fn report(&self, export_raw: bool) -> Result<SessionReport, ReportError> {
        self.export_report.execute(export_raw).await
    }

    /// One bounded raw export, admitted or refused (the busy reader task).
    pub fn admit_raw_export(&self) -> Result<AdmittedExport, ReportError> {
        self.export_report.admit_export()
    }
}

impl std::fmt::Debug for ExportSessionReportController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportSessionReportController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "export_report_controller_tests.rs"]
mod tests;
