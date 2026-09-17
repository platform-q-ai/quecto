//! The summary walk's cache: per record path, the stamp the summary was
//! validated at and the summary itself. A cold process seeds it once from the
//! derived index the home catalogue publishes, so an unchanged directory is
//! listed with one `stat` per file; every entry is still reused only while
//! the file carries its recorded stamp (the walk's own rule).
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

use super::super::super::session_home_catalogue::persisted_summaries;
use super::super::super::session_layout::FlatSessionLayout;

#[derive(Debug, Default)]
pub(in crate::infrastructure::persistence) struct SummaryCache {
    pub(in crate::infrastructure::persistence) reads: usize,
    pub(super) entries: std::collections::BTreeMap<std::path::PathBuf, (Vec<u64>, SessionSummary)>,
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
            cache.entries = persisted_summaries(layout);
        }
        Ok(cache)
    }

    /// The summary validated for `path` at exactly `stamp`, if any.
    pub(in crate::infrastructure::persistence) fn summary_at(
        &self,
        path: &std::path::Path,
        stamp: &[u64],
    ) -> Option<&SessionSummary> {
        self.entries
            .get(path)
            .filter(|(validated, _)| validated == stamp)
            .map(|(_, summary)| summary)
    }
}
