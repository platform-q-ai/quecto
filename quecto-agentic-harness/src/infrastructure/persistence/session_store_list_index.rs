//! The summary walk's cache: per record path, the stamp the record was read
//! at and its summary — or why that version could not be summarised (a verdict
//! on bytes read in full, R1-H1; an I/O failure is never cached, R2-H1). A cold
//! process seeds the summaries once from the derived index, so an unchanged
//! directory costs one `stat` per file; an entry is reused only at its stamp.
use crate::domain::error::DomainError;

use super::super::super::session_home_catalogue::persisted_walk;
use super::super::super::session_layout::FlatSessionLayout;

#[derive(Debug, Default)]
pub(in crate::infrastructure::persistence) struct SummaryCache {
    pub(in crate::infrastructure::persistence) reads: usize,
    pub(in crate::infrastructure::persistence) entries:
        super::super::super::session_home_catalogue::WalkEntries,
    seeded: bool,
}

impl SummaryCache {
    /// The locked cache, seeded from the persisted index on its first use in
    /// this process. Seeding trusts nothing: the walk re-stamps every file.
    pub(in crate::infrastructure::persistence) fn seeded<'a>(
        cache: &'a std::sync::Mutex<Self>,
        layout: &FlatSessionLayout,
    ) -> Result<std::sync::MutexGuard<'a, Self>, DomainError> {
        let mut cache = cache
            .lock()
            .map_err(|e| DomainError::Session(e.to_string()))?;
        if !cache.seeded {
            cache.seeded = true;
            debug_assert!(cache.entries.is_empty(), "seeding an already-used cache");
            cache.entries = persisted_walk(layout);
        }
        Ok(cache)
    }
}
