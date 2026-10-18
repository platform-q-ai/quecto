//! The immutable, versioned catalogue snapshot and a resolution's outcome.

use super::entry::CatalogueEntry;
use crate::domain::catalogue::value_objects::error::CatalogueDomainError;
use crate::domain::catalogue::value_objects::identity::ModelRef;
use crate::domain::catalogue::value_objects::precedence::SourceLayer;

/// Why an entry was rejected during resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedEntry {
    pub entry: CatalogueEntry,
    pub layer: SourceLayer,
    pub error: CatalogueDomainError,
}

/// Immutable, versioned view over the resolved catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSnapshot {
    pub(in crate::domain::catalogue) generation: u64,
    pub(in crate::domain::catalogue) entries: Vec<CatalogueEntry>,
}

impl CatalogueSnapshot {
    pub fn empty(generation: u64) -> Self {
        Self {
            generation,
            entries: Vec::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn entries(&self) -> &[CatalogueEntry] {
        &self.entries
    }

    pub fn find(&self, reference: &ModelRef) -> Option<&CatalogueEntry> {
        self.entries
            .iter()
            .find(|entry| entry.reference() == reference)
    }

    /// A narrowed view over this snapshot: same generation, only the entries
    /// `keep` accepts. A projection narrows the entry list, never the
    /// generation, so consumers can prove which publication they render.
    pub fn filtered(&self, keep: impl Fn(&CatalogueEntry) -> bool) -> Self {
        Self {
            generation: self.generation,
            entries: self
                .entries
                .iter()
                .filter(|entry| keep(entry))
                .cloned()
                .collect(),
        }
    }
}

/// Outcome of resolving source layers: the snapshot plus every rejected entry.
/// Invalid entries never corrupt the rest of a resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueResolution {
    pub snapshot: CatalogueSnapshot,
    pub rejected: Vec<RejectedEntry>,
}
