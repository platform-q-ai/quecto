//! One admitted record of the summary walk: the cached summary when its
//! signature is unchanged, else one header read validated against the
//! layout. Every skip is logged — a hand-renamed or symlinked file, an
//! unreadable or invalid one, a file replaced mid-read — so no record ever
//! vanishes from the list silently.
use crate::domain::message::Role;
use crate::domain::session::SessionSummary;
use crate::domain::session_identity::SessionIdentity;

use super::super::super::super::session_home_catalogue::stamp;
use super::super::super::super::session_layout::FlatSessionLayout;
use super::super::super::session_store_records::SessionHeader;
use super::super::super::{first_user_message, parse_session_header, str_to_role};
use super::super::SummaryCache;

pub(super) fn summary_of(
    layout: &FlatSessionLayout,
    path: &std::path::Path,
    cache: &mut SummaryCache,
) -> Option<SessionSummary> {
    let before = match stamp(path) {
        Ok(before) => before,
        Err(error) => {
            skipped(path, "not a regular session record", &error);
            return None;
        }
    };
    if let Some((_, summary)) = cache
        .entries
        .get(path)
        .filter(|(version, _)| *version == before)
    {
        return Some(summary.clone());
    }
    cache.reads += 1;
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            skipped(path, "unreadable session file", &error);
            return None;
        }
    };
    let header = match parse_session_header(&content) {
        Ok(header) => header,
        Err(error) => {
            skipped(path, "invalid session file", &error);
            return None;
        }
    };
    let identity = SessionIdentity::from_persisted_key(header.key.as_ref());
    if layout.session_file(&identity) != path {
        skipped(
            path,
            "session file name does not match its key",
            &format_args!("key {:?}", header.key),
        );
        return None;
    }
    if !stamp(path).is_ok_and(|after| before == after) {
        skipped(path, "session file changed while listing", &"retry");
        return None;
    }
    let summary = summarize(header, identity, &before);
    cache
        .entries
        .insert(path.to_path_buf(), (before, summary.clone()));
    Some(summary)
}

fn skipped(path: &std::path::Path, why: &str, detail: &dyn std::fmt::Display) {
    tracing::warn!(
        path = %path.display(),
        detail = %detail,
        "skipping {why} while listing sessions"
    );
}

fn summarize(
    header: SessionHeader<'_>,
    identity: SessionIdentity,
    stamp: &[u64],
) -> SessionSummary {
    SessionSummary {
        identity,
        title: first_user_message(&header.messages),
        message_count: header
            .messages
            .iter()
            .filter(|m| matches!(str_to_role(&m.role), Role::User | Role::Assistant))
            .count(),
        key: header.key.into_owned(),
        // The mtime observed inside the validated window (stamp[4]), so the
        // sort key can never come from a file replaced after the header read.
        updated_unix_secs: Some(stamp[4]),
    }
}
