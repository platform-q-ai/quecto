//! The on-disk shape of the derived `home.catalogue` (version 2). Per record:
//! the authority's file stamp (device, inode, length, mode, mtime, ctime), the
//! `.home` observation with the sidecar's stamp, and the listing summary the
//! store's walk validated at that stamp. No transcript content beyond the
//! title, no digests: an unchanged stamp is reused without a read.
//!
//! Trust: an entry is reused only while its file carries the recorded stamp,
//! yields only the identity it was keyed under, and its paths pass the sidecar
//! decoder's rule. A doctored entry can at most misreport a home or title in
//! the listing; admission reads the sidecar exactly, never this index.
use super::super::session_store::session_store_home::admissible_home_path;
use crate::domain::session_home::{
    AssociationProvenance, SessionHome, SessionHomeScope, WorkspaceGroup,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Bumped when the entry shape changes; older indexes rebuild once, with a diagnostic.
pub(super) const VERSION: u32 = 2;

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(super) struct Catalogue {
    pub(super) version: u32,
    /// Keyed by persisted session key.
    pub(super) records: BTreeMap<String, IndexEntry>,
}

impl Catalogue {
    pub(super) fn new(records: BTreeMap<String, IndexEntry>) -> Self {
        Self {
            version: VERSION,
            records,
        }
    }
    /// A readable index: current version and well-formed; anything else names why.
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let index: Self = serde_json::from_slice(bytes).map_err(|_| "invalid".to_string())?;
        match index.version {
            VERSION => Ok(index),
            other => Err(format!("version {other} unsupported")),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(super) struct IndexEntry {
    /// The validated transcript's stamp.
    pub(super) stamp: Vec<u64>,
    /// The sidecar observation this entry may reuse; `None` when the last
    /// observation was not stable enough to cache (it is re-observed).
    pub(super) home: Option<HomeEntry>,
    /// The summary the store's walk validated at `stamp`; `None` until the
    /// walk has run in a process that then published the index.
    pub(super) summary: Option<SummaryEntry>,
}

/// The listing data of one record beyond its identity and stamp: the store
/// walk derives everything else (key, mtime) from those.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(in crate::infrastructure::persistence) struct SummaryEntry {
    pub(in crate::infrastructure::persistence) title: String,
    pub(in crate::infrastructure::persistence) message_count: usize,
}

/// A sidecar observation with the sidecar's stamp (`None`: no sidecar existed).
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(super) struct HomeEntry {
    pub(super) stamp: Option<Vec<u64>>,
    pub(super) scope: IndexHome,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum IndexHome {
    Scoped {
        execution_dir: String,
        group_kind: String,
        group_path: String,
    },
    LegacyUnscoped,
    Unavailable {
        reason: String,
    },
}

impl IndexHome {
    /// The cacheable form of an observation; `None` when a path is not
    /// representable in the index (it is re-observed on every query).
    pub(super) fn from_scope(scope: &SessionHomeScope) -> Option<Self> {
        Some(match scope {
            SessionHomeScope::Scoped(home) => {
                let (group_kind, group_path) = match &home.group {
                    WorkspaceGroup::Git { common_dir } => ("git", common_dir),
                    WorkspaceGroup::Folder { directory } => ("folder", directory),
                };
                Self::Scoped {
                    execution_dir: home.execution_dir.to_str()?.to_string(),
                    group_kind: group_kind.to_string(),
                    group_path: group_path.to_str()?.to_string(),
                }
            }
            SessionHomeScope::LegacyUnscoped => Self::LegacyUnscoped,
            SessionHomeScope::Unavailable(reason) => Self::Unavailable {
                reason: reason.clone(),
            },
        })
    }
    /// The observation this entry stands for. Only the shapes authority can
    /// decode are scoped; an inconsistent (edited) entry is unavailable.
    pub(super) fn to_scope(&self) -> SessionHomeScope {
        match self {
            Self::Scoped {
                execution_dir,
                group_kind,
                group_path,
            } => {
                let execution_dir = PathBuf::from(execution_dir);
                let group_path = PathBuf::from(group_path);
                // Same admissibility as the sidecar decoder.
                if !admissible_home_path(&execution_dir) || !admissible_home_path(&group_path) {
                    return SessionHomeScope::Unavailable("home facts failed validation".into());
                }
                let group = match (group_kind.as_str(), execution_dir == group_path) {
                    ("git", _) => WorkspaceGroup::Git {
                        common_dir: group_path,
                    },
                    ("folder", true) => WorkspaceGroup::Folder {
                        directory: group_path,
                    },
                    _ => return SessionHomeScope::Unavailable("unsupported home group".into()),
                };
                SessionHomeScope::Scoped(SessionHome {
                    execution_dir,
                    group,
                    provenance: AssociationProvenance::SavedHere,
                })
            }
            Self::LegacyUnscoped => SessionHomeScope::LegacyUnscoped,
            Self::Unavailable { reason } => SessionHomeScope::Unavailable(reason.clone()),
        }
    }
}
