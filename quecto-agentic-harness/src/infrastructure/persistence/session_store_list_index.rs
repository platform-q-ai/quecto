//! The summary walk's cache: per record path, the stamp the record was read
//! at and its summary — `None` for a version that could not be summarised
//! (R1-H1: a failure is cached by stamp exactly as a success is). A cold process seeds it once from the
//! derived index the home catalogue publishes, so an unchanged directory is
//! listed with one `stat` per file; every entry is still reused only while
//! the file carries its recorded stamp (the walk's own rule).
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

use super::super::super::session_home_catalogue::persisted_walk;
use super::super::super::session_layout::FlatSessionLayout;

#[derive(Debug, Default)]
pub(in crate::infrastructure::persistence) struct SummaryCache {
    pub(in crate::infrastructure::persistence) reads: usize,
    pub(super) entries: super::super::super::session_home_catalogue::WalkEntries,
    seeded: bool,
}

impl SummaryCache {
    /// The locked cache, seeded from the persisted index on its first use in
    /// this process. Seeding trusts nothing: the walk re-stamps every file.
    pub(super) fn seeded<'a>(
        cache: &'a std::sync::Mutex<Self>,
        layout: &FlatSessionLayout,
    ) -> Result<std::sync::MutexGuard<'a, Self>, DomainError> {
        let mut cache = cache
            .lock()
            .map_err(|e| DomainError::Session(e.to_string()))?;
        if !cache.seeded {
            cache.seeded = true;
            debug_assert!(cache.entries.is_empty(), "seeding an already-used cache");
            cache.entries = persisted_walk(layout).into_entries();
        }
        Ok(cache)
    }

    /// What the walk made of `path` at exactly `stamp`, if it saw that
    /// version: its summary, or `None` when it could not be summarised.
    pub(in crate::infrastructure::persistence) fn version_at(
        &self,
        path: &std::path::Path,
        stamp: &[u64],
    ) -> Option<Option<&SessionSummary>> {
        let (validated, summary) = self.entries.get(path)?;
        (validated == stamp).then_some(summary.as_ref())
    }
}
