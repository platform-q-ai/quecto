//! One pass over the layout's flat directory for summary listing.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

use super::super::super::session_layout::FlatSessionLayout;
use super::SummaryCache;

#[path = "session_store_list_record.rs"]
mod session_store_list_record;
use session_store_list_record::summary_of;

/// One walk's answer: the summaries, newest first, and every admitted record
/// it could not list this time, as `(file name, why)`.
pub(in crate::infrastructure::persistence) type Walk = (Vec<SessionSummary>, Vec<(String, String)>);

pub(super) fn scan(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: &std::sync::Mutex<SummaryCache>,
) -> Result<Walk, DomainError> {
    let mut cache = SummaryCache::seeded(cache, layout)?;
    let (mut summaries, mut skipped) = (Vec::new(), Vec::new());
    let entries = match std::fs::read_dir(layout.sessions_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((summaries, skipped)),
        Err(e) => {
            return Err(DomainError::Session(format!(
                "failed to read sessions dir: {e}"
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|e| DomainError::Session(e.to_string()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let prefix = query.key_prefix();
        let admitted = FlatSessionLayout::is_session_record(&path)
            && prefix.is_none_or(|p| name.starts_with(&layout.record_name_prefix(p)));
        if admitted {
            match summary_of(layout, &path, &mut cache) {
                Ok(summary) if prefix.is_none_or(|p| p.admits(&summary.identity)) => {
                    summaries.push(summary);
                }
                Ok(_) => {}
                Err(why) => skipped.push((name, why)),
            }
        }
    }
    // Each row was re-stamped around its own read (`summary_of`); a record
    // rewritten after that is at worst one autosave stale, never partial.
    cache.entries.retain(|path, _| path.exists());
    summaries.sort_by(|a, b| {
        b.updated_unix_secs
            .cmp(&a.updated_unix_secs)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok((summaries, skipped))
}
