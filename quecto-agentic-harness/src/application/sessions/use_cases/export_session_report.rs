//! Export a retained session report (#1859): the latest eligible assistant
//! report of the active session — the newest assistant message that is
//! neither a tool-call step nor blank, resolved through the ledger's full
//! copy so a context-collapsed stub is never reported as the text — with
//! its bounded preview and recovery ref, and, on request, a raw export of
//! every retained message and every available spill entry.
//!
//! The export is one transaction of the use case: the records are read
//! under one consistent view, the spill entries after it, and the artifact
//! is written only if the session's epoch is still the one the records
//! were read at. Raw exports are bounded: two may run at once, a third is
//! refused outright; an admitted export releases its slot when it
//! completes or is dropped. Cursors are never consumed: the report is a
//! read of the published transcript.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{
    ExportManifest, ExportRecord, RawExportReceipt, ReportError, ReportPreview, SessionReport,
};
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::ports::export::SessionExportPort;
use crate::domain::message::{Message, Role};
use crate::domain::session_identity::{SessionIdentity, SpillId};

/// How many raw exports may run at once, per composed loop (one loop per
/// process today: `uds_lifecycle.rs`, single or multi client), so the
/// bound is the process-wide one it always was.
pub const MAX_CONCURRENT_EXPORTS: usize = 2;

pub struct ExportSessionReport {
    state: ActiveSessionHandle,
    /// `None`: composed without an export directory; raw exports are
    /// refused as unavailable.
    export: Option<Arc<dyn SessionExportPort>>,
    admissions: Arc<tokio::sync::Semaphore>,
}

/// What one consistent read of the session yields for a raw export.
struct ExportSource {
    epoch: u64,
    revision: u64,
    messages: Vec<Message>,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    identity: SessionIdentity,
}

/// An admitted raw export: the report with its export, holding one of the
/// bounded slots until it resolves or is dropped.
pub type AdmittedExport =
    Pin<Box<dyn Future<Output = Result<SessionReport, ReportError>> + Send + 'static>>;

impl ExportSessionReport {
    pub fn new(state: ActiveSessionHandle, export: Option<Arc<dyn SessionExportPort>>) -> Self {
        Self {
            state,
            export,
            admissions: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_EXPORTS)),
        }
    }

    /// The latest eligible report of `messages`, each candidate resolved
    /// through `full_copy` (the id-addressable ledger) before use.
    fn latest_report<'a>(
        messages: &'a [Message],
        full_copy: impl Fn(&str) -> Option<&'a Message>,
    ) -> Option<ReportPreview> {
        let candidate = messages.iter().rev().find(|message| {
            message.role == Role::Assistant
                && message.tool_calls.is_empty()
                && !message.content.trim().is_empty()
        })?;
        let id = candidate.id().to_string();
        Some(ReportPreview::of(full_copy(&id).unwrap_or(candidate)))
    }

    /// The report, with a raw export when `export_raw`. Unbounded: the
    /// caller serialises its requests (the dispatch loop) or admits them
    /// through [`Self::admit_export`] first.
    pub async fn execute(&self, export_raw: bool) -> Result<SessionReport, ReportError> {
        let (report, source) = {
            let state = self.state.read().await;
            let ledger = state.conversation();
            let report = Self::latest_report(ledger.live_messages(), |id| ledger.full_copy(id));
            let source = export_raw.then(|| ExportSource {
                epoch: ledger.epoch(),
                revision: ledger.rev(),
                messages: ledger.retained_messages(),
                spill_store: ledger.spill_store().cloned(),
                identity: state.identity().clone(),
            });
            (report, source)
        };
        let raw_export = match source {
            Some(source) => Some(self.export(source).await?),
            None => None,
        };
        Ok(SessionReport { report, raw_export })
    }

    /// Admit one bounded raw export, or refuse it when both slots are
    /// taken. The admitted export runs when awaited and frees its slot on
    /// completion or drop, so a caller may schedule it off its own task.
    pub fn admit_export(self: &Arc<Self>) -> Result<AdmittedExport, ReportError> {
        let permit = self
            .admissions
            .clone()
            .try_acquire_owned()
            .map_err(|_| ReportError::ExportBusy)?;
        let this = self.clone();
        Ok(Box::pin(async move {
            let _permit = permit;
            this.execute(true).await
        }))
    }

    async fn export(&self, source: ExportSource) -> Result<RawExportReceipt, ReportError> {
        let export = self.export.as_ref().ok_or(ReportError::ExportUnavailable)?;
        let ExportSource {
            epoch,
            revision,
            messages,
            spill_store,
            identity,
        } = source;
        let mut records: Vec<ExportRecord> = messages
            .into_iter()
            .map(|message| ExportRecord::Message(Box::new(message)))
            .collect();
        let mut spill_count = 0;
        if let Some(store) = spill_store {
            for entry in store
                .list_entries(&identity)
                .await
                .map_err(ReportError::Spill)?
                .iter()
            {
                let spill = store
                    .recall(&identity, &SpillId::new(entry.id.as_str()))
                    .await
                    .map_err(ReportError::Spill)?
                    .ok_or_else(|| ReportError::SpillDisappeared(entry.id.clone()))?;
                records.push(ExportRecord::Spill(spill));
                spill_count += 1;
            }
        }
        if self.state.read().await.conversation().epoch() != epoch {
            return Err(ReportError::EpochChanged);
        }
        let manifest = ExportManifest {
            epoch,
            revision,
            record_count: records.len(),
            spill_count,
        };
        export
            .write_export(records, manifest)
            .await
            .map_err(ReportError::Writer)
    }
}

impl std::fmt::Debug for ExportSessionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportSessionReport")
            .field("export_available", &self.export.is_some())
            .field("free_export_slots", &self.admissions.available_permits())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "export_session_report_tests.rs"]
mod tests;
