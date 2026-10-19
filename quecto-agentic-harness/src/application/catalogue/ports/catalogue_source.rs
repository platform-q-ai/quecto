//! The catalogue source port: one named, ordered input the resolve use case
//! loads, with the per-record diagnostics and the per-source load error.

use crate::domain::catalogue::entities::entry::CatalogueEntry;
use crate::domain::catalogue::value_objects::precedence::SourceLayer;

/// One named, ordered catalogue input. Infrastructure implements this port;
/// the application owns precedence between layers.
pub trait CatalogueSource: Send + Sync {
    /// Stable identifier used in structured per-source errors.
    fn id(&self) -> &str;
    /// The domain precedence layer this source feeds.
    fn layer(&self) -> SourceLayer;
    /// Load this source's entries. Parsing stays in infrastructure. A record
    /// the source cannot map into a domain entry is reported in
    /// [`SourceEntries::skipped`] instead of failing the whole layer, so one
    /// bad record cannot erase its valid neighbours.
    fn load(&self) -> Result<SourceEntries, String>;
}

/// What one source load yields: the entries that mapped cleanly plus the
/// records that did not (kept as diagnostics, never silently dropped).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceEntries {
    pub entries: Vec<CatalogueEntry>,
    pub skipped: Vec<SkippedRecord>,
}

impl From<Vec<CatalogueEntry>> for SourceEntries {
    fn from(entries: Vec<CatalogueEntry>) -> Self {
        Self {
            entries,
            skipped: Vec::new(),
        }
    }
}

/// One record a source could not map into a domain entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRecord {
    pub record: String,
    pub error: String,
}

/// A source layer that could not be loaded; the layer is skipped so one
/// malformed input cannot erase the other layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueSourceError {
    pub source: String,
    pub error: String,
}
