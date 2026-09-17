//! Versioned optional authority. Transcript saves never rewrite these bytes.
use super::FileSessionStore;
use crate::domain::{
    error::DomainError,
    session_home::{AssociationProvenance, SessionHome, SessionHomeScope, WorkspaceGroup},
    session_identity::SessionIdentity,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct HomeRecord {
    version: u32,
    execution_dir: Vec<u8>,
    group_kind: String,
    group_path: Vec<u8>,
    provenance: String,
}

fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    if bytes.len() <= 32768 && bytes.iter().all(|byte| *byte >= 32 && *byte != 127) {
        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
        if path.is_absolute()
            && path.components().all(|part| {
                matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            return Ok(path);
        }
    }
    Err("home path is not an admissible absolute path".into())
}

pub(super) fn encode(home: &SessionHome) -> Result<Vec<u8>, DomainError> {
    let (kind, path) = match &home.group {
        WorkspaceGroup::Git { common_dir } => ("git", common_dir),
        WorkspaceGroup::Folder { directory } => ("folder", directory),
    };
    let record = HomeRecord {
        version: 1,
        execution_dir: path_bytes(&home.execution_dir),
        group_kind: kind.into(),
        group_path: path_bytes(path),
        provenance: "saved_here".into(),
    };
    let bytes = serde_json::to_vec(&record).map_err(error)?;
    match decode(&bytes) {
        SessionHomeScope::Scoped(_) => Ok(bytes),
        _ => Err(error("home facts failed validation")),
    }
}
pub(in crate::infrastructure::persistence) fn decode(bytes: &[u8]) -> SessionHomeScope {
    let parsed = (|| -> Result<SessionHome, String> {
        let record: HomeRecord = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if record.version == 1 && record.provenance == "saved_here" {
            let execution_dir = decode_path(record.execution_dir)?;
            let group_path = decode_path(record.group_path)?;
            let group = match record.group_kind.as_str() {
                "git" => WorkspaceGroup::Git {
                    common_dir: group_path,
                },
                "folder" if execution_dir == group_path => WorkspaceGroup::Folder {
                    directory: group_path,
                },
                _ => return Err("unsupported home group".into()),
            };
            Ok(SessionHome {
                execution_dir,
                group,
                provenance: AssociationProvenance::SavedHere,
            })
        } else {
            Err("unsupported home version or provenance".into())
        }
    })();
    match parsed {
        Ok(home) => SessionHomeScope::Scoped(home),
        Err(reason) => SessionHomeScope::Unavailable(reason),
    }
}
pub(in crate::infrastructure::persistence) fn error(error: impl std::fmt::Display) -> DomainError {
    DomainError::Session(format!("session home: {error}"))
}

/// Atomic publication via a same-directory temporary file and durable rename.
pub(in crate::infrastructure::persistence) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    new_only: bool,
) -> Result<(), DomainError> {
    let parent = path.parent().ok_or_else(|| error("missing parent"))?;
    std::fs::create_dir_all(parent).map_err(error)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    temp.write_all(bytes).map_err(error)?;
    temp.as_file().sync_all().map_err(error)?;
    if new_only {
        match temp.persist_noclobber(path) {
            Ok(_) => (),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) => return Err(error(e)),
        }
    } else {
        temp.persist(path).map_err(error)?;
    }
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(error)
}

impl FileSessionStore {
    /// The flat layout this store's records, homes and catalogue live in.
    pub fn layout(&self) -> &super::super::session_layout::FlatSessionLayout {
        &self.layout
    }
    /// The listing summary this store's walk validated for `path` at exactly
    /// `stamp`, for the derived index to carry; `None` when the walk has not
    /// seen that version.
    pub(in crate::infrastructure::persistence) fn summary_at(
        &self,
        path: &Path,
        stamp: &[u64],
    ) -> Option<crate::domain::session::SessionSummary> {
        self.summaries.lock().ok()?.summary_at(path, stamp).cloned()
    }
    pub fn read_home(&self, identity: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        match std::fs::read(self.layout.home_file(identity)) {
            Ok(bytes) => Ok(decode(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(self.layout.home_file(identity)) {
                    Err(absent) if absent.kind() == std::io::ErrorKind::NotFound => {
                        Ok(SessionHomeScope::LegacyUnscoped)
                    }
                    _ => Ok(SessionHomeScope::Unavailable(
                        "home authority is inaccessible".into(),
                    )),
                }
            }
            Err(e) => Ok(SessionHomeScope::Unavailable(format!(
                "home authority unreadable: {e}"
            ))),
        }
    }
    pub fn record_new_home(
        &self,
        identity: &SessionIdentity,
        home: &SessionHome,
    ) -> Result<(), DomainError> {
        if identity.persisted_key().is_some() {
            self.claim_key(identity)?;
            match std::fs::metadata(self.layout.session_file(identity)) {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(error(e)),
            }
            atomic_write(&self.layout.home_file(identity), &encode(home)?, true)?;
        }
        Ok(())
    }

    /// An empty save is no session: the transcript goes, and with it the
    /// home sidecar (#2009) — a home without a transcript is never authority.
    pub(super) async fn delete_session_file_if_present(
        &self,
        identity: &SessionIdentity,
    ) -> Result<(), DomainError> {
        match tokio::fs::remove_file(self.session_path(identity)).await {
            Ok(()) => self.discard_orphan_home(identity),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.discard_orphan_home(identity)
            }
            Err(e) => Err(DomainError::Session(format!(
                "failed to delete empty session: {e}"
            ))),
        }
    }

    /// A home sidecar without a transcript is not authority — a save that
    /// never committed a message, or a transcript removed by hand — and
    /// must not lock the key to a directory with no history. Removed under
    /// the key's claim, and only while the transcript is absent; a home
    /// beside a transcript is never touched here.
    pub fn discard_orphan_home(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        if identity.persisted_key().is_none() {
            return Ok(());
        }
        self.claim_key(identity)?;
        match std::fs::metadata(self.layout.session_file(identity)) {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
        match std::fs::remove_file(self.layout.home_file(identity)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(error(format!("orphan home not removable: {e}"))),
        }
    }
}
