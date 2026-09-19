//! What the persisted index hands the store's summary walk in a new process:
//! the listing summaries, keyed by record path with the stamp each was
//! validated at. Only summaries: a record the walk could not summarise is
//! never seeded (R2-H2) — each process reads it once for itself. An absent,
//! unreadable or incompatible index seeds nothing. The walk re-stamps every
//! file, so a stale entry is read again, never trusted.
use super::super::session_layout::FlatSessionLayout;
use super::STAMP_LEN;
use super::session_home_catalogue_index::Catalogue;
use crate::domain::session::SessionSummary;
use std::{collections::BTreeMap, path::PathBuf};

/// Per record path: the stamp it was read at and what the walk made of that
/// version — its summary, or why it could not be summarised (a verdict on
/// bytes that were read in full; never an I/O failure).
pub(in crate::infrastructure::persistence) type WalkEntries =
    BTreeMap<PathBuf, (Vec<u64>, Result<SessionSummary, String>)>;

pub(in crate::infrastructure::persistence) fn persisted_walk(
    layout: &FlatSessionLayout,
) -> WalkEntries {
    let mut walk = WalkEntries::new();
    let Ok(bytes) = std::fs::read(layout.home_catalogue_file()) else {
        return walk;
    };
    let Ok(index) = Catalogue::decode(&bytes) else {
        return walk;
    };
    for (key, entry) in index.records {
        let identity = super::persisted_identity(key.as_str());
        let valid = identity.persisted_key().is_some() && entry.stamp.len() == STAMP_LEN;
        if let (true, Some(listed)) = (valid, entry.summary) {
            let summary = SessionSummary {
                key,
                title: listed.title,
                message_count: listed.message_count,
                updated_unix_secs: Some(entry.stamp[4]),
                identity: identity.clone(),
            };
            walk.insert(layout.session_file(&identity), (entry.stamp, Ok(summary)));
        }
    }
    walk
}
