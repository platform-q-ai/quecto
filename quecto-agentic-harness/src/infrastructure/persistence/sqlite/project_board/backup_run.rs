//! Linux backup execution. FDs and transactions remain owned until cleanup/join.
use super::super::backup_files::{AdmittedBackupFile, anchored as fs};
use super::super::backup_sqlite::{ValidationLease, classify};
use super::*;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use rusqlite::{Connection, OpenFlags};
use std::{collections::BTreeSet, fs::File, io, path::Path, time::Duration};

fn unsafe_io(_: io::Error) -> BackupError {
    BackupError::UnsafePath
}
fn canonical_board(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn entries(directory: &File, limit: usize) -> Result<Vec<String>, BackupError> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(fs::path(directory, "")).map_err(unsafe_io)? {
        if result.len() < limit {
        } else {
            return Err(BackupError::ResourceLimit);
        }
        result.push(
            entry
                .map_err(unsafe_io)?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    result.sort();
    Ok(result)
}
fn record(directory: &File, name: &str) -> Result<(File, AdmittedBackupFile), BackupError> {
    let file =
        fs::open(directory, name, libc::O_RDONLY | libc::O_NONBLOCK, 0).map_err(unsafe_io)?;
    let identity = fs::identity(&file.metadata().map_err(unsafe_io)?).map_err(unsafe_io)?;
    Ok((
        file,
        AdmittedBackupFile {
            name: name.to_owned(),
            identity,
        },
    ))
}
fn open_read(path: &Path, busy: Duration) -> Result<Connection, BackupError> {
    let c =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(classify)?;
    c.busy_timeout(busy).map_err(classify)?;
    c.execute_batch("BEGIN").map_err(classify)?;
    Ok(c)
}
struct Guard(Option<Box<dyn ValidationLease>>);
impl Guard {
    fn finish(&mut self) -> Result<(), BackupError> {
        match self.0.take() {
            Some(mut lease) => lease.cancel_and_join(),
            None => Ok(()),
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
struct Partial<'a> {
    directory: &'a File,
    owned: AdmittedBackupFile,
}
impl Drop for Partial<'_> {
    fn drop(&mut self) {
        // An unpredictable exclusive name makes these sidecars invocation-owned.
        for suffix in ["-wal", "-shm", "-journal"] {
            let name = format!("{}{suffix}", self.owned.name);
            if let Ok((_, file)) = record(self.directory, &name) {
                let _ = fs::remove(self.directory, &file);
            }
        }
        let _ = fs::remove(self.directory, &self.owned);
    }
}
struct Budget<'a> {
    owner: &'a SqliteBoardBackup,
    invocation: Duration,
    board: Duration,
    limits: &'a BackupLimits,
}
impl Budget<'_> {
    fn remaining(&self) -> Result<Duration, BackupError> {
        let now = self.owner.time.elapsed();
        let invocation = self
            .limits
            .invocation
            .saturating_sub(now.saturating_sub(self.invocation));
        let board = self
            .limits
            .board
            .saturating_sub(now.saturating_sub(self.board));
        if invocation > Duration::ZERO && board > Duration::ZERO {
            Ok(invocation.min(board))
        } else if invocation == Duration::ZERO {
            Err(BackupError::InvocationDeadline)
        } else {
            Err(BackupError::ValidationDeadline)
        }
    }
}
impl SqliteBoardBackup {
    pub(super) fn run(&self, request: &BackupRequest) -> Result<BackupReport, BackupError> {
        let invocation = self.time.elapsed();
        let root = match fs::directory(&self.source_root) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(BackupError::NoTargets);
            }
            Err(error) => return Err(unsafe_io(error)),
        };
        let names = entries(&root, request.limits.entries)?;
        let mut boards: Vec<_> = names
            .iter()
            .filter_map(|name| {
                match &self.resolved {
                    Some((Some(board), resolve)) => resolve(board)
                        .file_name()
                        .and_then(|file| (file == name.as_str()).then_some(board.as_str())),
                    Some((None, _)) => Some(name.as_str()),
                    None => name.strip_suffix(".sqlite"),
                }
                .filter(|id| canonical_board(id))
                .map(str::to_owned)
            })
            .collect();
        if let Some(board) = &request.board {
            if canonical_board(board) && boards.contains(board) {
                boards = vec![board.clone()];
            } else {
                return Err(BackupError::UnknownBoard);
            }
        }
        if boards.len() <= request.limits.boards {
        } else {
            return Err(BackupError::ResourceLimit);
        }
        match boards.first() {
            Some(_) => {}
            None => return Err(BackupError::NoTargets),
        }
        let mut outcomes = Vec::new();
        for board in boards {
            let mut outcome = BackupOutcome {
                board,
                published: None,
                error: None,
                taken_at: request.at.clone(),
                retained: 0,
                deleted: 0,
                stage: BackupStage::Unattempted,
            };
            let budget = Budget {
                owner: self,
                invocation,
                board: self.time.elapsed(),
                limits: &request.limits,
            };
            let result = budget
                .remaining()
                .and_then(|_| self.board(&root, request, &budget, &mut outcome));
            if let Err(error) = result {
                outcome.error = Some(error);
            }
            outcomes.push(outcome);
        }
        Ok(BackupReport { outcomes })
    }
    fn guarded<T>(
        &self,
        c: &Connection,
        budget: &Budget<'_>,
        operation: impl FnOnce() -> Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        let mut guard = Guard(Some(
            self.validation
                .arm(c.get_interrupt_handle(), budget.remaining()?)?,
        ));
        let result = operation();
        let joined = guard.finish();
        joined?;
        result
    }
    fn board(
        &self,
        root: &File,
        request: &BackupRequest,
        budget: &Budget<'_>,
        outcome: &mut BackupOutcome,
    ) -> Result<(), BackupError> {
        outcome.stage = BackupStage::Admission;
        let resolved;
        let held_parent;
        let source_name = match &self.resolved {
            Some((single, resolve)) => {
                resolved = resolve(&outcome.board);
                let parent = resolved.parent().ok_or(BackupError::UnsafePath)?;
                let relative = parent
                    .strip_prefix(&self.source_root)
                    .map_err(|_| BackupError::UnsafePath)?;
                held_parent = if single.is_some() {
                    if relative.as_os_str().is_empty() {
                    } else {
                        return Err(BackupError::UnsafePath);
                    }
                    None
                } else {
                    if relative == Path::new(&outcome.board) {
                    } else {
                        return Err(BackupError::UnsafePath);
                    }
                    Some(
                        fs::open(root, &outcome.board, libc::O_RDONLY | libc::O_DIRECTORY, 0)
                            .map_err(unsafe_io)?,
                    )
                };
                resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or(BackupError::UnsafePath)?
                    .to_owned()
            }
            None => {
                held_parent = None;
                format!("{}.sqlite", outcome.board)
            }
        };
        let discovery_root = root;
        let root = held_parent.as_ref().unwrap_or(root);
        let source_parent = match &self.resolved {
            Some((_, resolve)) => resolve(&outcome.board)
                .parent()
                .ok_or(BackupError::UnsafePath)?
                .to_owned(),
            None => self.source_root.clone(),
        };
        let (_source_file, source) = record(root, &source_name)?;
        let source_path = fs::path(root, &source_name);
        let connection = open_read(&source_path, request.limits.busy.min(budget.remaining()?))?;
        self.guarded(&connection, budget, || {
            connection
                .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(classify)?;
            admission::admit(&connection, &outcome.board)
        })?;
        fs::stable_directory(root, &source_parent).map_err(unsafe_io)?;
        fs::check(root, &source).map_err(unsafe_io)?;
        let (directory, canonical) =
            fs::private_destination(&request.to, &self.source_root).map_err(unsafe_io)?;
        // Read-only preflight preserves inventories for immediately refused requests.
        let preflight = self.candidates(&directory, &outcome.board, request)?;
        check_time(&preflight, request, &outcome.board)?;
        let _lock = fs::lock(&directory, &outcome.board).map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock {
                BackupError::BusyDeadline
            } else {
                BackupError::UnsafePath
            }
        })?;
        let candidates = self.candidates(&directory, &outcome.board, request)?;
        let final_name = snapshot_name(&outcome.board, &request.at);
        check_time(&candidates, request, &outcome.board)?;
        let name = format!(".{}.{}.partial", outcome.board, uuid::Uuid::new_v4());
        let file = self
            .files
            .create_partial(&directory, &name)
            .map_err(|_| BackupError::PublishFailed)?;
        let mut partial = Partial {
            directory: &directory,
            owned: AdmittedBackupFile {
                name,
                identity: fs::identity(&file.metadata().map_err(unsafe_io)?).map_err(unsafe_io)?,
            },
        };
        fs::stable_directory(&directory, &canonical).map_err(unsafe_io)?;
        fs::stable_directory(root, &source_parent).map_err(unsafe_io)?;
        fs::check(root, &source).map_err(unsafe_io)?;
        let partial_path = fs::path(&directory, &partial.owned.name);
        let mut destination =
            Connection::open_with_flags(&partial_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|_| BackupError::PublishFailed)?;
        outcome.stage = BackupStage::Copy;
        let mut copy_remaining = request.limits.copy;
        self.copy_with_budget(
            self.sqlite.as_ref(),
            &connection,
            &mut destination,
            budget,
            &mut copy_remaining,
        )?;
        drop(connection);
        outcome.stage = BackupStage::Validation;
        self.guarded(&destination, budget, || {
            admission::admit(&destination, &outcome.board)?;
            let rows = self.sqlite.integrity(&destination).map_err(classify)?;
            if rows == ["ok"] {
                Ok(())
            } else {
                Err(BackupError::IntegrityFailed)
            }
        })?;
        drop(destination);
        // Admit all prior contents and identities before publication or any pruning.
        for (candidate, _) in &candidates {
            fs::check(&directory, candidate).map_err(unsafe_io)?;
            let prior = open_read(
                &fs::path(&directory, &candidate.name),
                request.limits.busy.min(budget.remaining()?),
            )?;
            self.guarded(&prior, budget, || {
                admission::admit(&prior, &outcome.board)?;
                exact_integrity(&prior)
            })?;
            // Pinned SQLite omits CHECK expressions on READ_ONLY schema loads.
            // An engine-owned temporary copy validates those same constraints
            // without opening any prior/source database writable or reinterpreting schema.
            let mut validation = Connection::open("").map_err(classify)?;
            self.copy_with_budget(
                &SystemBackupSqliteOps,
                &prior,
                &mut validation,
                budget,
                &mut copy_remaining,
            )?;
            self.guarded(&validation, budget, || exact_integrity(&validation))?;
            fs::check(&directory, candidate).map_err(unsafe_io)?;
        }
        fs::stable_directory(root, &source_parent).map_err(unsafe_io)?;
        fs::check(root, &source).map_err(unsafe_io)?;
        fs::stable_directory(discovery_root, &self.source_root).map_err(unsafe_io)?;
        fs::stable_directory(&directory, &canonical).map_err(unsafe_io)?;
        budget.remaining()?;
        outcome.stage = BackupStage::Publication;
        self.files
            .sync_file(&file)
            .map_err(|_| BackupError::PublishFailed)?;
        match self
            .files
            .publish_noreplace(&directory, &partial.owned, &final_name)
        {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(BackupError::Collision);
            }
            Err(_) => {
                if publication_seen(&directory, &final_name, &mut partial.owned) {
                    outcome.published = Some(request.to.join(&final_name));
                    return Err(BackupError::PublicationUncertain);
                }
                return Err(BackupError::PublishFailed);
            }
        }
        outcome.published = Some(request.to.join(&final_name));
        self.files
            .sync_directory(&directory)
            .map_err(|_| BackupError::PublicationUncertain)?;
        outcome.stage = BackupStage::Retention;
        self.retain(&directory, &canonical, candidates, budget, outcome)?;
        outcome.stage = BackupStage::Complete;
        Ok(())
    }
    fn copy_with_budget(
        &self,
        operations: &dyn BackupSqliteOps,
        source: &Connection,
        destination: &mut Connection,
        budget: &Budget<'_>,
        remaining: &mut Duration,
    ) -> Result<(), BackupError> {
        let permitted = (*remaining).min(budget.remaining()?);
        if permitted > Duration::ZERO {
        } else {
            return Err(BackupError::BusyDeadline);
        }
        let began = self.time.elapsed();
        let result = operations.copy(
            source,
            destination,
            permitted,
            budget.limits.busy.min(permitted),
        );
        *remaining = remaining.saturating_sub(self.time.elapsed().saturating_sub(began));
        result
    }
    fn candidates(
        &self,
        directory: &File,
        board: &str,
        request: &BackupRequest,
    ) -> Result<Vec<(AdmittedBackupFile, Timestamp)>, BackupError> {
        let prefix = format!("{board}-");
        let mut result = Vec::new();
        for name in entries(directory, request.limits.entries)? {
            if let Some(text) = name
                .strip_prefix(&prefix)
                .and_then(|n| n.strip_suffix(".sqlite"))
            {
                if text.len() == 16 && text.is_ascii() {
                    let spelled = format!(
                        "{}-{}-{}T{}:{}:{}Z",
                        &text[..4],
                        &text[4..6],
                        &text[6..8],
                        &text[9..11],
                        &text[11..13],
                        &text[13..15]
                    );
                    if text.as_bytes()[8] == b'T' && text.as_bytes()[15] == b'Z' {
                        if let Ok(at) = Timestamp::parse(&spelled) {
                            if result.len() < request.limits.candidates {
                            } else {
                                return Err(BackupError::ResourceLimit);
                            }
                            let (_, candidate) = record(directory, &name)?;
                            result.push((candidate, at));
                        }
                    }
                }
            }
        }
        result.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(result)
    }
    fn retain(
        &self,
        directory: &File,
        canonical: &Path,
        candidates: Vec<(AdmittedBackupFile, Timestamp)>,
        budget: &Budget<'_>,
        outcome: &mut BackupOutcome,
    ) -> Result<(), BackupError> {
        let mut dates = BTreeSet::new();
        dates.insert(outcome.taken_at.as_str()[..10].to_owned());
        outcome.retained = 1;
        for (candidate, at) in candidates {
            let day = at.as_str()[..10].to_owned();
            if dates.len() < 14 && dates.insert(day) {
                outcome.retained += 1;
            } else {
                budget.remaining()?;
                fs::stable_directory(directory, canonical)
                    .map_err(|_| BackupError::RetentionFailed)?;
                assert_ne!(
                    candidate.name,
                    snapshot_name(&outcome.board, &outcome.taken_at),
                    "retention never removes the new publication"
                );
                self.files
                    .prune_admitted(directory, &candidate)
                    .map_err(|_| BackupError::RetentionFailed)?;
                outcome.deleted += 1;
            }
        }
        if outcome.deleted > 0 {
            self.files
                .sync_directory(directory)
                .map_err(|_| BackupError::RetentionFailed)?;
        }
        Ok(())
    }
}
fn check_time(
    candidates: &[(AdmittedBackupFile, Timestamp)],
    request: &BackupRequest,
    board: &str,
) -> Result<(), BackupError> {
    let final_name = snapshot_name(board, &request.at);
    for (candidate, at) in candidates {
        if at < &request.at {
        } else if candidate.name == final_name {
            return Err(BackupError::Collision);
        } else {
            return Err(BackupError::ClockRegression);
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "backup_run_tests.rs"]
mod tests;
fn exact_integrity(c: &Connection) -> Result<(), BackupError> {
    if SystemBackupSqliteOps.integrity(c).map_err(classify)? == ["ok"] {
        Ok(())
    } else {
        Err(BackupError::IntegrityFailed)
    }
}

fn snapshot_name(board: &str, at: &Timestamp) -> String {
    format!("{board}-{}.sqlite", at.as_str().replace(['-', ':'], ""))
}
fn publication_seen(directory: &File, name: &str, partial: &mut AdmittedBackupFile) -> bool {
    if let Ok(file) = fs::open(directory, name, libc::O_RDONLY | libc::O_NONBLOCK, 0) {
        if let Ok(metadata) = file.metadata() {
            for links in [1, 2] {
                let mut expected = partial.identity.clone();
                expected.links = links;
                if fs::same(&metadata, &expected) {
                    partial.identity = expected;
                    return true;
                }
            }
        }
    }
    false
}
