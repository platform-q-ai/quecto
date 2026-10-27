//! Retention's sweep of a board's orphaned sidecars and partials (#2767, #2769).
use super::*;

impl SqliteBoardBackup {
    /// Removes this board's canonical sidecars whose snapshot is gone: those of
    /// the snapshots just pruned and orphans left by earlier runs (#2767).
    /// Also removes this board's canonical partials and their sidecars, left
    /// by a run killed mid-backup (#2769), except the current run's own.
    /// Runs under the board lock, so no publisher of this board races it.
    /// Only single-link regular files are removed; any other entry under a
    /// canonical name (symlink, hard link, directory, FIFO...) is left alone.
    pub(super) fn sweep_sidecars(
        &self,
        scope: &SweepScope<'_>,
        budget: &Budget<'_>,
        swept: &mut usize,
    ) -> Result<(), BackupError> {
        // Bounded like every listing here. Another board's concurrent run may
        // grow the shared directory past the bound; the sweep then works on the
        // names it listed and the rest wait for a later run.
        let (names, _) = bounded_entries(scope.directory, budget.limits.entries.saturating_add(1))
            .map_err(|_| BackupError::RetentionFailed)?;
        self.sweep_listed(scope, budget, &names, swept)
    }
    /// Decides per listed name; `names` may be any subset of the directory.
    pub(super) fn sweep_listed(
        &self,
        scope: &SweepScope<'_>,
        budget: &Budget<'_>,
        names: &[String],
        swept: &mut usize,
    ) -> Result<(), BackupError> {
        let SweepScope {
            directory,
            board,
            own_partial,
            ..
        } = *scope;
        assert!(
            partial_of(board, own_partial) == Some(own_partial),
            "the current run's partial is a canonical partial of this board"
        );
        for name in names {
            if let Some(snapshot) = sidecar_of(board, name) {
                // Presence is asked of the directory, not inferred from a listing
                // that may be truncated.
                if exists(directory, snapshot)? {
                    // A retained snapshot keeps its sidecars.
                } else {
                    assert!(
                        SIDECARS.iter().any(|suffix| name.ends_with(suffix))
                            && snapshot_at(board, snapshot).is_some(),
                        "only canonical snapshot sidecars are swept"
                    );
                    self.sweep_entry(scope, budget, name, swept)?;
                }
            } else if let Some(partial) = partial_of(board, name) {
                // Under this board's lock no other run of this board is in
                // flight, so every partial but the current run's is orphaned.
                if partial == own_partial {
                    // The current run's partial (or a sidecar of it) is left
                    // to its own drop guard.
                } else {
                    assert!(
                        partial != own_partial && partial_of(board, partial) == Some(partial),
                        "only another run's canonical partial of this board is swept"
                    );
                    self.sweep_entry(scope, budget, name, swept)?;
                }
            }
        }
        Ok(())
    }
    /// Removes one admitted sweep candidate if it is a single-link regular
    /// file; an entry-level refusal leaves it alone and the sweep goes on.
    fn sweep_entry(
        &self,
        scope: &SweepScope<'_>,
        budget: &Budget<'_>,
        name: &str,
        swept: &mut usize,
    ) -> Result<(), BackupError> {
        budget.remaining()?;
        fs::stable_directory(scope.directory, scope.canonical)
            .map_err(|_| BackupError::RetentionFailed)?;
        if let Some(entry) = regular_entry(scope.directory, name)? {
            match self.files.prune_admitted(scope.directory, &entry) {
                Ok(()) => *swept += 1,
                Err(error) => {
                    if entry_refusal(&error) {
                        // Left alone; the sweep continues.
                    } else {
                        return Err(BackupError::RetentionFailed);
                    }
                }
            }
        }
        Ok(())
    }
}
/// The held destination a sweep works in, and the board whose lock it holds.
#[derive(Clone, Copy)]
pub(super) struct SweepScope<'a> {
    pub(super) directory: &'a File,
    pub(super) canonical: &'a Path,
    pub(super) board: &'a str,
    /// The current run's partial; never swept, though it should be gone by now.
    pub(super) own_partial: &'a str,
}
/// Allowlist of refusals that concern only the entry: unreadable, gone, or
/// replaced (identity/path refusal, now a symlink) since the probe. Anything
/// else (EIO, EROFS...) is a directory or device fault and fails retention.
fn entry_refusal(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied
    ) || error.raw_os_error() == Some(libc::ELOOP)
}
/// Whether `name` exists, without following it or opening its target.
fn exists(directory: &File, name: &str) -> Result<bool, BackupError> {
    match fs::open(directory, name, libc::O_PATH, 0) {
        Ok(_) => Ok(true),
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(BackupError::RetentionFailed)
            }
        }
    }
}
/// Identifies a sweep candidate without following or opening its target: `O_PATH`
/// never reads a FIFO or device. Admits only a single-link regular file.
fn regular_entry(directory: &File, name: &str) -> Result<Option<AdmittedBackupFile>, BackupError> {
    match fs::open(directory, name, libc::O_PATH, 0) {
        Ok(handle) => {
            let meta = handle
                .metadata()
                .map_err(|_| BackupError::RetentionFailed)?;
            if meta.is_file() {
                // `identity` admits exactly one link: a hard-linked alias is skipped.
                Ok(fs::identity(&meta).ok().map(|identity| AdmittedBackupFile {
                    name: name.to_owned(),
                    identity,
                }))
            } else {
                Ok(None)
            }
        }
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(BackupError::RetentionFailed)
            }
        }
    }
}
/// Allowlist: the snapshot name a canonical `-wal`/`-shm`/`-journal` belongs to.
fn sidecar_of<'a>(board: &str, name: &'a str) -> Option<&'a str> {
    SIDECARS
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .filter(|snapshot| snapshot_at(board, snapshot).is_some())
}
/// The name this adapter gives a board's in-flight copy.
pub(super) fn partial_name(board: &str, id: uuid::Uuid) -> String {
    format!(".{board}.{}.partial", id.hyphenated())
}
/// Allowlist: the canonical partial a name is, or is a `-wal`/`-shm`/`-journal`
/// of: `.{board}.{uuid}.partial` exactly as `partial_name` spells it, with a
/// lowercase hyphenated version-4 UUID.
fn partial_of<'a>(board: &str, name: &'a str) -> Option<&'a str> {
    let partial = SIDECARS
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name);
    let text = partial
        .strip_prefix('.')?
        .strip_prefix(board)?
        .strip_prefix('.')?
        .strip_suffix(".partial")?;
    uuid::Uuid::try_parse(text)
        .ok()
        .filter(|id| id.get_version_num() == 4 && partial_name(board, *id) == partial)
        .map(|_| partial)
}
