//! One pass over the layout's flat directory for summary listing.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

use super::super::super::session_home_catalogue::stamp;
use super::super::super::session_layout::FlatSessionLayout;
use super::SummaryCache;

#[path = "session_store_list_record.rs"]
mod session_store_list_record;
use session_store_list_record::summary_of;

pub(super) fn scan(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: &std::sync::Mutex<SummaryCache>,
) -> Result<Vec<SessionSummary>, DomainError> {
    let mut cache = cache
        .lock()
        .map_err(|e| DomainError::Session(e.to_string()))?;
    let mut summaries = Vec::new();
    let entries = match std::fs::read_dir(layout.sessions_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(summaries),
        Err(e) => {
            return Err(DomainError::Session(format!(
                "failed to read sessions dir: {e}"
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|e| DomainError::Session(e.to_string()))?;
        let path = entry.path();
        let admitted = FlatSessionLayout::is_session_record(&path)
            && query.key_prefix().is_none_or(|prefix| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&layout.record_name_prefix(prefix))
            });
        if admitted {
            let Some(summary) = summary_of(layout, &path, &mut cache) else {
                continue;
            };
            if query
                .key_prefix()
                .is_none_or(|prefix| prefix.admits(&summary.identity))
            {
                summaries.push(summary);
            }
        }
    }
    summaries.retain(|summary| {
        let path = layout.session_file(&summary.identity);
        let unchanged = cache
            .entries
            .get(&path)
            .is_some_and(|(version, _)| stamp(&path).is_ok_and(|current| *version == current));
        if !unchanged {
            tracing::warn!(
                target: "session_store",
                path = %path.display(),
                "session record changed while it was being listed; skipped this listing"
            );
        }
        unchanged
    });
    cache.entries.retain(|path, _| path.exists());
    summaries.sort_by(|a, b| {
        b.updated_unix_secs
            .cmp(&a.updated_unix_secs)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(summaries)
}
