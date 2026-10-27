//! Repair of a publication a crash interrupted between `linkat(partial,
//! snapshot)` and the partial's unlink: both names then share one inode with
//! two links, and every later admission of that snapshot refuses.
use super::sweep::partial_of;
use super::*;
use std::os::unix::fs::MetadataExt;

/// Whether an interrupted pair may be admitted or must still refuse.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Interrupted {
    /// The unlocked preflight admits the pair so the locked run can repair it.
    Tolerated,
    /// After repair every candidate must again be single-link.
    Refused,
}
/// A canonical snapshot and this board's canonical partial on one inode.
pub(super) struct InterruptedPair {
    pub(super) snapshot: AdmittedBackupFile,
    pub(super) partial: AdmittedBackupFile,
}
impl SqliteBoardBackup {
    /// Under the board lock no publisher of this board is in flight, so a
    /// 2-link pair is a crash's leftover: unlinking the partial name makes the
    /// snapshot single-link again. Unlinks made are synced before any error.
    pub(super) fn repair_interrupted(
        &self,
        directory: &File,
        canonical: &Path,
        board: &str,
        request: &BackupRequest,
    ) -> Result<(), BackupError> {
        let names = entries(directory, request.limits.entries)?;
        let mut repaired = 0;
        let mut repair = || -> Result<(), BackupError> {
            for name in &names {
                if snapshot_at(board, name).is_some() {
                    if let Some(pair) = interrupted_pair(directory, board, &names, name)? {
                        fs::stable_directory(directory, canonical).map_err(unsafe_io)?;
                        self.files
                            .prune_admitted(directory, &pair.partial)
                            .map_err(unsafe_io)?;
                        repaired += 1;
                    }
                }
            }
            Ok(())
        };
        let result = repair();
        let synced = if repaired > 0 {
            self.files.sync_directory(directory).map_err(unsafe_io)
        } else {
            Ok(())
        };
        result.and(synced)
    }
}
/// Allowlist: `snapshot` is this board's canonical snapshot, a regular file
/// with exactly two links, and its one peer is this board's canonical partial
/// (never a sidecar of one), probed no-follow on the same device and inode.
/// Anything else is `None` and stays refused by single-link admission.
pub(super) fn interrupted_pair(
    directory: &File,
    board: &str,
    names: &[String],
    snapshot: &str,
) -> Result<Option<InterruptedPair>, BackupError> {
    assert!(
        snapshot_at(board, snapshot).is_some(),
        "only this board's canonical snapshot is ever paired"
    );
    let Some(held) = probe(directory, snapshot)? else {
        return Ok(None);
    };
    if held.is_file() && held.nlink() == 2 {
    } else {
        return Ok(None);
    }
    let mut peers = Vec::new();
    for name in names {
        if partial_of(board, name) == Some(name.as_str()) {
            if let Some(meta) = probe(directory, name)? {
                if meta.is_file()
                    && meta.nlink() == 2
                    && meta.dev() == held.dev()
                    && meta.ino() == held.ino()
                {
                    peers.push(name.clone());
                }
            }
        }
    }
    if let [partial] = peers.as_slice() {
        let identity = BackupFileIdentity {
            device: held.dev(),
            inode: held.ino(),
            links: 2,
        };
        Ok(Some(InterruptedPair {
            snapshot: AdmittedBackupFile {
                name: snapshot.to_owned(),
                identity: identity.clone(),
            },
            partial: AdmittedBackupFile {
                name: partial.clone(),
                identity,
            },
        }))
    } else {
        Ok(None)
    }
}
/// No-follow `O_PATH` metadata; never opens a FIFO or device for reading.
fn probe(directory: &File, name: &str) -> Result<Option<std::fs::Metadata>, BackupError> {
    match fs::open(directory, name, libc::O_PATH, 0) {
        Ok(handle) => handle.metadata().map(Some).map_err(unsafe_io),
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(unsafe_io(error))
            }
        }
    }
}
