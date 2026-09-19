//! What the persisted index hands the store's summary walk in a new process:
//! the listing summaries, keyed by record path with the stamp each was
//! validated at, and the versions the walk could not summarise (R1-H1). An
//! absent, unreadable or incompatible index seeds nothing. The walk re-stamps
//! every file, so a stale entry is read again, never trusted.
use super::super::session_layout::FlatSessionLayout;
use super::STAMP_LEN;
use super::session_home_catalogue_index::Catalogue;
use super::session_home_catalogue_rejections::record_path;
use crate::domain::session::SessionSummary;
use std::{collections::BTreeMap, path::PathBuf};

/// Per record path: the stamp it was read at and its summary (`None`: the
/// walk could not summarise that version).
pub(in crate::infrastructure::persistence) type WalkEntries =
    BTreeMap<PathBuf, (Vec<u64>, Option<SessionSummary>)>;

#[derive(Debug, Default)]
pub(in crate::infrastructure::persistence) struct PersistedWalk {
    summaries: BTreeMap<PathBuf, (Vec<u64>, SessionSummary)>,
    unlisted: BTreeMap<PathBuf, Vec<u64>>,
}

impl PersistedWalk {
    pub(in crate::infrastructure::persistence) fn into_entries(self) -> WalkEntries {
        let unlisted = self
            .unlisted
            .into_iter()
            .map(|(p, stamp)| (p, (stamp, None)));
        let listed = self.summaries.into_iter();
        unlisted
            .chain(listed.map(|(p, (stamp, summary))| (p, (stamp, Some(summary)))))
            .collect()
    }
}

pub(in crate::infrastructure::persistence) fn persisted_walk(
    layout: &FlatSessionLayout,
) -> PersistedWalk {
    let mut walk = PersistedWalk::default();
    let Ok(bytes) = std::fs::read(layout.home_catalogue_file()) else {
        return walk;
    };
    let Ok(index) = Catalogue::decode(&bytes) else {
        return walk;
    };
    let summary = |key: String, stamp: &[u64], title: String, message_count: usize| {
        let identity = super::persisted_identity(key.as_str());
        let valid = identity.persisted_key().is_some() && stamp.len() == STAMP_LEN;
        valid.then(|| {
            let summary = SessionSummary {
                key,
                title,
                message_count,
                updated_unix_secs: Some(stamp[4]),
                identity: identity.clone(),
            };
            (layout.session_file(&identity), (stamp.to_vec(), summary))
        })
    };
    for (key, entry) in index.records {
        if let Some(listed) = entry.summary {
            let seeded = summary(key, &entry.stamp, listed.title, listed.message_count);
            walk.summaries.extend(seeded);
        }
    }
    for (name, entry) in index.rejected {
        let Some(path) = record_path(layout, &name) else {
            continue;
        };
        match entry.listed {
            // The summary seeds only the file it was recorded beside.
            Some((key, title, count)) => walk.summaries.extend(
                summary(key, &entry.stamp, title, count).filter(|(listed, _)| *listed == path),
            ),
            None if entry.unlisted => {
                walk.unlisted.insert(path, entry.stamp);
            }
            None => {}
        }
    }
    walk
}
