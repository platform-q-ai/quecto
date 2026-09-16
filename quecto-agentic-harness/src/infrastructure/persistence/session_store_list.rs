//! The summary walk behind `SessionStore::list` (#1861): one pass over the
//! layout's flat directory, the `.json` allowlist and the identity-prefix
//! skip applied before any read, one lightweight header parse per admitted
//! record, tolerant skipping of what cannot be read or summarised, newest
//! first. Summary-only: nothing here loads a session in full.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;
use crate::domain::message::Role;
use crate::domain::session::SessionSummary;
use crate::domain::session_identity::SessionIdentity;

use super::super::session_layout::FlatSessionLayout;
use super::session_store_records::SessionHeader;
use super::{first_user_message, parse_session_header, str_to_role};

#[derive(Debug, Default)]
pub(super) struct SummaryCache {
    pub(super) reads: usize,
    entries: std::collections::BTreeMap<std::path::PathBuf, (Vec<u64>, SessionSummary)>,
}

pub(super) async fn list_summaries(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: std::sync::Arc<std::sync::Mutex<SummaryCache>>,
) -> Result<Vec<SessionSummary>, DomainError> {
    let layout = layout.clone();
    let query = query.clone();
    tokio::task::spawn_blocking(move || scan(&layout, &query, &cache))
        .await
        .map_err(|e| DomainError::Session(e.to_string()))?
}

fn scan(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: &std::sync::Mutex<SummaryCache>,
) -> Result<Vec<SessionSummary>, DomainError> {
    use super::super::session_home_catalogue::stamp;
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
            let Ok(before) = stamp(&path) else { continue };
            let cached = cache
                .entries
                .get(&path)
                .filter(|(version, _)| *version == before);
            let summary = match cached {
                Some((_, summary)) => summary.clone(),
                None => {
                    cache.reads += 1;
                    let Ok(content) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let Ok(header) = parse_session_header(&content) else {
                        continue;
                    };
                    let identity = SessionIdentity::from_persisted_key(header.key.as_ref());
                    if layout.session_file(&identity) == path
                        && stamp(&path).is_ok_and(|after| before == after)
                    {
                        let summary = summarize(header, &path);
                        cache
                            .entries
                            .insert(path.clone(), (before.clone(), summary.clone()));
                        summary
                    } else {
                        continue;
                    }
                }
            };
            if query.key_prefix().is_none_or(|prefix| {
                prefix.admits(&SessionIdentity::from_persisted_key(&summary.key))
            }) {
                summaries.push(summary);
            }
        }
    }
    summaries.retain(|summary| {
        let path = layout.session_file(&SessionIdentity::from_persisted_key(&summary.key));
        cache
            .entries
            .get(&path)
            .is_some_and(|(version, _)| stamp(&path).is_ok_and(|current| *version == current))
    });
    cache.entries.retain(|path, _| path.exists());
    summaries.sort_by(|a, b| {
        b.updated_unix_secs
            .cmp(&a.updated_unix_secs)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(summaries)
}

fn summarize(header: SessionHeader<'_>, path: &std::path::Path) -> SessionSummary {
    SessionSummary {
        title: first_user_message(&header.messages),
        message_count: header
            .messages
            .iter()
            .filter(|m| matches!(str_to_role(&m.role), Role::User | Role::Assistant))
            .count(),
        key: header.key.into_owned(),
        updated_unix_secs: std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()),
    }
}
