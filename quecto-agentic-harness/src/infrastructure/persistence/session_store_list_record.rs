//! One admitted record of the summary walk: what the cache holds for its
//! unchanged stamp — a summary or a remembered verdict — else one read, its
//! header validated against the layout. Every skip is logged and returned —
//! a hand-renamed or symlinked file, an unreadable or invalid one, a file
//! replaced mid-read — so no record ever vanishes from the list silently.
use crate::domain::message::Role;
use crate::domain::session::SessionSummary;
use crate::domain::session_identity::SessionIdentity;

use super::super::super::super::session_home_catalogue::stamp;
use super::super::super::super::session_layout::FlatSessionLayout;
use super::super::super::super::session_record_read::{ReadRefusal, read_record};
use super::super::super::session_store_records::SessionHeader;
use super::super::super::{first_user_message, parse_session_header, str_to_role};
use super::super::SummaryCache;

pub(super) fn summary_of(
    layout: &FlatSessionLayout,
    path: &std::path::Path,
    cache: &mut SummaryCache,
) -> Result<SessionSummary, String> {
    let before = stamp(path).map_err(|e| skipped(path, "not a regular session record", &e))?;
    if let Some((_, known)) = cache.entries.get(path).filter(|(at, _)| *at == before) {
        // A cached verdict is not read — or logged — again, but is still
        // named in every answer it is missing from (R2-H1).
        return known.clone();
    }
    // An I/O failure is no verdict on the content: never cached (R2-H1); a
    // size above the cap is one, on this stamp, and nothing was read.
    let bytes = match read_record(path, &before) {
        Ok(bytes) => bytes,
        Err(ReadRefusal::TooLarge { len, cap }) => {
            let verdict = Err(skipped(
                path,
                "record too large",
                &format!("{len} bytes, cap {cap}"),
            ));
            cache
                .entries
                .insert(path.to_path_buf(), (before, verdict.clone()));
            return verdict;
        }
        Err(e) => return Err(skipped(path, "unreadable session file", &e)),
    };
    cache.reads += 1;
    let verdict = summary_in(layout, path, &bytes, &before);
    if !stamp(path).is_ok_and(|after| before == after) {
        return Err(skipped(
            path,
            "session file changed while listing",
            &"retry",
        ));
    }
    let version = (before, verdict.clone());
    cache.entries.insert(path.to_path_buf(), version);
    verdict
}

/// The verdict on a record's bytes, read in full — here or by the one pass (#2042).
pub(in crate::infrastructure::persistence) fn summary_in(
    layout: &FlatSessionLayout,
    path: &std::path::Path,
    bytes: &[u8],
    stamp: &[u64],
) -> Result<SessionSummary, String> {
    let content =
        std::str::from_utf8(bytes).map_err(|e| skipped(path, "invalid session file", &e))?;
    let header =
        parse_session_header(content).map_err(|e| skipped(path, "invalid session file", &e))?;
    let identity = SessionIdentity::from_persisted_key(header.key.as_ref());
    if layout.session_file(&identity) != path {
        let key = format!("key {:?}", header.key);
        return Err(skipped(
            path,
            "session file name does not match its key",
            &key,
        ));
    }
    Ok(summarize(header, identity, stamp))
}

/// Logs the skip and returns it as the walk's diagnostic text.
fn skipped(path: &std::path::Path, why: &str, detail: &dyn std::fmt::Display) -> String {
    tracing::warn!(
        path = %path.display(),
        detail = %detail,
        "skipping {why} while listing sessions"
    );
    format!("{why}: {detail}")
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
