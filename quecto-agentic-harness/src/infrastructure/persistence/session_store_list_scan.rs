//! One pass over the layout's flat directory for summary listing.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;
use crate::domain::message::Role;
use crate::domain::session::SessionSummary;
use crate::domain::session_identity::SessionIdentity;

use super::super::super::session_home_catalogue::stamp;
use super::super::super::session_layout::FlatSessionLayout;
use super::super::session_store_records::SessionHeader;
use super::super::{first_user_message, parse_session_header, str_to_role};
use super::SummaryCache;

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
            // Every skip is logged: a record never vanishes from the list
            // silently (a hand-renamed or symlinked file, an unreadable or
            // invalid one).
            let before = match stamp(&path) {
                Ok(before) => before,
                Err(error) => {
                    skipped(&path, "not a regular session record", &error);
                    continue;
                }
            };
            let cached = cache
                .entries
                .get(&path)
                .filter(|(version, _)| *version == before);
            let summary = match cached {
                Some((_, summary)) => summary.clone(),
                None => {
                    cache.reads += 1;
                    let content = match std::fs::read_to_string(&path) {
                        Ok(content) => content,
                        Err(error) => {
                            skipped(&path, "unreadable session file", &error);
                            continue;
                        }
                    };
                    let header = match parse_session_header(&content) {
                        Ok(header) => header,
                        Err(error) => {
                            skipped(&path, "invalid session file", &error);
                            continue;
                        }
                    };
                    let identity = SessionIdentity::from_persisted_key(header.key.as_ref());
                    if layout.session_file(&identity) != path {
                        skipped(
                            &path,
                            "session file name does not match its key",
                            &format_args!("key {:?}", header.key),
                        );
                        continue;
                    }
                    if !stamp(&path).is_ok_and(|after| before == after) {
                        skipped(&path, "session file changed while listing", &"retry");
                        continue;
                    }
                    let summary = summarize(header, &path);
                    cache
                        .entries
                        .insert(path.clone(), (before.clone(), summary.clone()));
                    summary
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

fn skipped(path: &std::path::Path, why: &str, detail: &dyn std::fmt::Display) {
    tracing::warn!(
        path = %path.display(),
        detail = %detail,
        "skipping {why} while listing sessions"
    );
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
