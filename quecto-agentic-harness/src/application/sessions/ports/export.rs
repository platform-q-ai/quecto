//! The outbound export port of the sessions capability (#1859, #1974):
//! the one effect Export a retained session report requires beyond the
//! read model and the retention store. The use case decides what is
//! exported and what the manifest states; the adapter knows directories,
//! record encoding and files, and returns where the artifact landed.
use std::future::Future;
use std::pin::Pin;

use crate::application::sessions::dto::{ExportManifest, ExportRecord, RawExportReceipt};
use crate::domain::error::DomainError;

pub type ExportOutcome<'a> =
    Pin<Box<dyn Future<Output = Result<RawExportReceipt, DomainError>> + Send + 'a>>;

/// Port: write one raw export — every record in order plus the manifest
/// describing them — as one artifact, or fail without leaving a partial
/// artifact where a later export would find it.
pub trait SessionExportPort: Send + Sync {
    fn write_export(
        &self,
        records: Vec<ExportRecord>,
        manifest: ExportManifest,
    ) -> ExportOutcome<'_>;
}
