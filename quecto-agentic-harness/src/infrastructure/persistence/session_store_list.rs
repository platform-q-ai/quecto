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

pub(super) async fn list_summaries(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
) -> Result<Vec<SessionSummary>, DomainError> {
    let key_prefix = query.key_prefix();
    let file_prefix = key_prefix.map(|prefix| layout.record_name_prefix(prefix));
    let mut summaries = Vec::new();
    let sessions_dir = layout.sessions_dir();
    if !sessions_dir.exists() {
        return Ok(summaries);
    }
    let mut entries = tokio::fs::read_dir(sessions_dir)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read sessions dir: {}", e)))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| DomainError::Session(format!("failed to read sessions dir entry: {}", e)))?
    {
        let path = entry.path();
        // Affirmative allowlist and prefix skip BEFORE any read: a ~22k-file
        // directory is walked once and only records that can match are
        // opened.
        if !FlatSessionLayout::is_session_record(&path) {
            continue;
        }
        if let Some(ref fp) = file_prefix {
            if !entry.file_name().to_string_lossy().starts_with(fp.as_str()) {
                continue;
            }
        }
        let metadata = entry.metadata().await.ok();
        let updated_unix_secs = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "skipping unreadable session file while listing sessions"
                );
                continue;
            }
        };
        let header: SessionHeader = match parse_session_header(&content) {
            Ok(header) => header,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "skipping invalid session file while listing sessions"
                );
                continue;
            }
        };
        if let Some(prefix) = key_prefix {
            if !prefix.admits(&SessionIdentity::from_persisted_key(header.key.as_ref())) {
                continue;
            }
        }
        let title = first_user_message(&header.messages);
        let message_count = header
            .messages
            .iter()
            .filter(|m| matches!(str_to_role(&m.role), Role::User | Role::Assistant))
            .count();
        summaries.push(SessionSummary {
            title,
            key: header.key.into_owned(),
            message_count,
            updated_unix_secs,
            scope: header.scope,
        });
    }
    summaries.sort_by(|a, b| {
        b.updated_unix_secs
            .cmp(&a.updated_unix_secs)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(summaries)
}
