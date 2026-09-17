//! The on-disk shape of the derived `home.catalogue` (version 2). Per record:
//! the authority's file stamp (device, inode, length, mode, mtime, ctime), its
//! persisted key, the `.home` sidecar observation with the sidecar's own
//! stamp, and the listing summary the store's walk validated at that stamp.
//! No transcript bytes and no content digests: a record whose stamp is
//! unchanged is reused without a read; anything else is validated again.
//!
//! Trust: an entry is only ever reused when the file it names still carries
//! the recorded stamp, and the row it yields is the identity whose layout path
//! it was keyed under. A doctored entry can at most misreport a home or a
//! title in the listing; resume admission reads the `.home` sidecar exactly,
//! never this index, so no entry can make a session resume-eligible.
use crate::domain::session_home::{
    AssociationProvenance, SessionHome, SessionHomeScope, WorkspaceGroup,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Bumped whenever the entry shape changes: an older index is
/// version-incompatible and rebuilt once, with a diagnostic.
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
    /// A readable index: the current version and well-formed. Anything else
    /// names why.
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let index: Self = serde_json::from_slice(bytes).map_err(|_| "invalid".to_string())?;
        if index.version == VERSION {
            Ok(index)
        } else {
            Err(format!("version {} unsupported", index.version))
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

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(super) struct HomeEntry {
    /// The sidecar's stamp; `None` when no sidecar existed.
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
                let group = match group_kind.as_str() {
                    "git" => WorkspaceGroup::Git {
                        common_dir: group_path,
                    },
                    "folder" if execution_dir == group_path => WorkspaceGroup::Folder {
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
