//! The outcome of one resolve run.

use std::sync::Arc;

use crate::application::catalogue::ports::catalogue_source::{CatalogueSourceError, SkippedRecord};
use crate::domain::catalogue::entities::snapshot::{CatalogueSnapshot, RejectedEntry};

/// Outcome of one resolve run: the snapshot that is now current, entries the
/// domain rejected, and sources that failed to load.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCatalogue {
    pub snapshot: Arc<CatalogueSnapshot>,
    pub rejected: Vec<RejectedEntry>,
    pub source_errors: Vec<CatalogueSourceError>,
    /// Per-record diagnostics from sources whose other records loaded fine:
    /// `(source id, skipped record)` pairs surfaced to consumers.
    pub skipped: Vec<(String, SkippedRecord)>,
}
