//! The store's two writes and their durability (#2218). A compaction is
//! written to a temporary file, fsynced, renamed over the transcript and the
//! directory fsynced; an append is fsynced after it is written. A turn's
//! messages carry durable ordinals a supervisor may deliver, so a record
//! lost to a power cut would hand the same ordinals out again: an fsync
//! (milliseconds) per save is cheap beside a model turn (seconds).
//!
//! The store appends only onto a file whose bytes it can vouch for: the
//! very file it last wrote or loaded whole — same device, inode, length and
//! modification time. The record is dropped before every write starts and
//! restored only once the write succeeds, so after a failed, cancelled or
//! foreign write (another process's rename or in-place rewrite) the next
//! save compacts without trusting what it reads back — the page cache may
//! hold pages that never reached the disk (after an fsync error, say). A
//! record torn by a crash is therefore never appended after either.
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::*;

/// Which file a path names, and its state: a rename changes the inode, an
/// in-place rewrite the modification time, an append the length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
}

impl FileIdentity {
    pub(super) async fn of(path: &Path) -> Option<Self> {
        let metadata = tokio::fs::metadata(path).await.ok()?;
        Some(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime_ns: i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()),
        })
    }
}

/// The identity of each transcript as this store last wrote or loaded it
/// whole (#2218).
#[derive(Debug, Default)]
pub(super) struct IntactFiles(std::sync::Mutex<HashMap<PathBuf, FileIdentity>>);

impl IntactFiles {
    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, FileIdentity>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `path` is still exactly the file this store left.
    pub(super) async fn appendable(&self, path: &Path) -> bool {
        let Some(known) = self.known().get(path).copied() else {
            return false;
        };
        FileIdentity::of(path).await == Some(known)
    }

    /// Read the transcript, with the identity of the file it was read from.
    pub(super) async fn read(
        &self,
        path: &Path,
    ) -> Result<(String, Option<FileIdentity>), DomainError> {
        let before = FileIdentity::of(path).await;
        let data = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| DomainError::Session(format!("failed to read session: {}", e)))?;
        Ok((data, before))
    }

    /// `path` was read whole as `len` bytes (`intact` when every record
    /// parsed) from the file `before` the read: remember it only if it is
    /// intact and still that file, unchanged.
    pub(super) async fn observe_read(
        &self,
        path: &Path,
        before: Option<FileIdentity>,
        len: usize,
        intact: bool,
    ) {
        let after = FileIdentity::of(path).await;
        match (intact, before, after) {
            (true, Some(before), Some(after)) if before == after && after.len == len as u64 => {
                self.known().insert(path.to_path_buf(), after);
            }
            _ => self.forget(path),
        }
    }

    /// Nothing about `path` can be vouched for until a write succeeds.
    pub(super) fn forget(&self, path: &Path) {
        self.known().remove(path);
    }

    /// The write of `path` succeeded: remember the file it left.
    pub(super) async fn record_written(&self, path: &Path) {
        match FileIdentity::of(path).await {
            Some(identity) => {
                self.known().insert(path.to_path_buf(), identity);
            }
            None => self.forget(path),
        }
    }
}

impl FileSessionStore {
    /// One write of `path` under the intactness record (#2218): forgotten
    /// before the write starts, remembered with its new length only once it
    /// succeeds; `write` learns whether it may append.
    pub(super) async fn tracked<F: Future<Output = Result<(), DomainError>>>(
        &self,
        path: &Path,
        write: impl FnOnce(bool) -> F,
    ) -> Result<(), DomainError> {
        let appendable = self.intact.appendable(path).await;
        self.intact.forget(path);
        let written = write(appendable).await;
        if written.is_ok() {
            self.intact.record_written(path).await;
        }
        written
    }
}

pub(super) async fn write_compacted(path: &Path, session: &Session) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;
    #[cfg(test)]
    injected(&FAIL_NEXT_WRITE_OF, path)?;

    let record = SessionRecordRef::Snapshot(SessionFileRef {
        key: session.key.runtime_key(),
        messages: session.messages.iter().map(message_to_record_ref).collect(),
        workflow_run: session.workflow_run.as_ref(),
        subagent_roster: &session.subagent_roster,
    });
    let mut line = serde_json::to_string(&record)
        .map_err(|e| DomainError::Session(format!("failed to serialize session: {e}")))?;
    line.push('\n');
    let tmp_path = path.with_extension("tmp");
    let write_error =
        |e: std::io::Error| DomainError::Session(format!("failed to write session: {e}"));
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(write_error)?;
    file.write_all(line.as_bytes()).await.map_err(write_error)?;
    file.sync_all().await.map_err(write_error)?;
    drop(file);
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to rename session: {e}")))?;
    sync_parent_dir(path).await?;
    #[cfg(test)]
    injected(&FAIL_NEXT_SYNC_OF, path)?;
    Ok(())
}

/// Make the rename itself durable: fsync the directory holding `path`.
async fn sync_parent_dir(path: &Path) -> Result<(), DomainError> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::File::open(&dir)?.sync_all())
        .await
        .map_err(|e| DomainError::Session(format!("failed to sync sessions dir: {e}")))?
        .map_err(|e| DomainError::Session(format!("failed to sync sessions dir: {e}")))
}

pub(super) async fn append_record(
    path: &Path,
    record: &SessionRecordRef<'_>,
) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;
    #[cfg(test)]
    injected(&FAIL_NEXT_WRITE_OF, path)?;

    reject_symlink(path).await?;
    let mut line = serde_json::to_string(record)
        .map_err(|e| DomainError::Session(format!("failed to serialize session: {e}")))?;
    line.push('\n');
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to open session for append: {e}")))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| DomainError::Session(format!("failed to append session: {e}")))?;
    file.sync_data()
        .await
        .map_err(|e| DomainError::Session(format!("failed to sync session: {e}")))?;
    #[cfg(test)]
    injected(&FAIL_NEXT_SYNC_OF, path)?;
    Ok(())
}

async fn reject_symlink(path: &Path) -> Result<(), DomainError> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to inspect session: {e}")))?;
    if metadata.file_type().is_symlink() {
        return Err(DomainError::Session(
            "refusing to append to symlinked session file".to_string(),
        ));
    }
    Ok(())
}

pub(super) async fn persisted_prefix_changed(
    path: &Path,
    messages: &[Message],
    previously_persisted: usize,
) -> Result<bool, DomainError> {
    if previously_persisted > messages.len() {
        return Ok(true);
    }
    let data = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let (persisted, intact) = parse_session_records(&data)
        .map_err(|e| DomainError::Session(format!("failed to parse session: {e}")))?;
    if !intact {
        return Ok(true);
    }
    // #2218: a file of another length than the watermark (a write that
    // landed but reported failure, or a replaced file) is rewritten from the
    // live transcript, which is authoritative; never appended to.
    if persisted.messages.len() != previously_persisted {
        return Ok(true);
    }
    Ok(persisted.messages[..previously_persisted]
        .iter()
        .zip(messages_with_assigned_ordinals(&messages[..previously_persisted]).iter())
        .any(|(left, right)| message_to_record(left) != message_to_record(right)))
}

pub(super) async fn append_known_delta(
    path: &Path,
    key: &SessionIdentity,
    messages: &[Message],
    previously_persisted: usize,
    workflow_run: Option<&WorkflowRunPersisted>,
    appendable: bool,
) -> Result<(), DomainError> {
    let must_compact = previously_persisted == 0
        || !appendable
        || !path.exists()
        || !is_jsonl_session_file(path).await?
        || persisted_prefix_changed(path, messages, previously_persisted).await?;
    compact_or_append_delta(
        path,
        key,
        messages,
        previously_persisted,
        workflow_run,
        must_compact,
    )
    .await
}

pub(super) async fn compact_or_append_delta(
    path: &Path,
    key: &SessionIdentity,
    messages: &[Message],
    previously_persisted: usize,
    workflow_run: Option<&WorkflowRunPersisted>,
    must_compact: bool,
) -> Result<(), DomainError> {
    if must_compact {
        let subagent_roster = tokio::fs::read_to_string(path)
            .await
            .ok()
            .and_then(|data| parse_session_data(&data).ok())
            .map(|s| s.subagent_roster)
            .unwrap_or_default();
        let session = Session {
            key: key.clone(),
            messages: messages_with_assigned_ordinals(messages).into_owned(),
            workflow_run: workflow_run.cloned(),
            subagent_roster,
        };
        return write_compacted(path, &session).await;
    }
    let assigned = messages_with_assigned_ordinals(messages);
    let record = SessionRecordRef::Append {
        start_index: Some(previously_persisted),
        messages: assigned[previously_persisted..]
            .iter()
            .map(message_to_record_ref)
            .collect(),
        workflow_run,
        workflow_run_cleared: workflow_run.is_none(),
        subagent_roster: None,
    };
    append_record(path, &record).await
}

pub(super) async fn append_or_compact(
    path: &Path,
    session: &Session,
    appendable: bool,
) -> Result<(), DomainError> {
    let mut assigned_session;
    let session = if session.messages.iter().any(|m| m.ordinal.is_none()) {
        assigned_session = session.clone();
        assigned_session.messages =
            messages_with_assigned_ordinals(&assigned_session.messages).into_owned();
        &assigned_session
    } else {
        session
    };
    if !appendable || !path.exists() || !is_jsonl_session_file(path).await? {
        return write_compacted(path, session).await;
    }

    let data = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let (previous, intact) = parse_session_records(&data)
        .map_err(|e| DomainError::Session(format!("failed to parse session: {e}")))?;
    if !intact
        || previous.key != session.key
        || session.messages.len() < previous.messages.len()
        || session.messages[..previous.messages.len()]
            .iter()
            .map(message_to_record)
            .zip(previous.messages.iter().map(message_to_record))
            .any(|(current, saved)| current != saved)
    {
        return write_compacted(path, session).await;
    }

    let added = &session.messages[previous.messages.len()..];
    let roster_changed = session.subagent_roster != previous.subagent_roster;
    if added.is_empty() && session.workflow_run == previous.workflow_run && !roster_changed {
        return Ok(());
    }

    let record = SessionRecordRef::Append {
        start_index: Some(previous.messages.len()),
        messages: added.iter().map(message_to_record_ref).collect(),
        workflow_run: session.workflow_run.as_ref(),
        workflow_run_cleared: session.workflow_run.is_none(),
        subagent_roster: roster_changed.then_some(session.subagent_roster.as_slice()),
    };
    append_record(path, &record).await
}

/// Test seams: the next write of a path lands and then reports an fsync
/// failure (as `EIO` from `fsync` does), or fails before it writes a byte.
#[cfg(test)]
pub(super) static FAIL_NEXT_SYNC_OF: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(test)]
pub(super) static FAIL_NEXT_WRITE_OF: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn injected(seam: &std::sync::Mutex<Vec<PathBuf>>, path: &Path) -> Result<(), DomainError> {
    let mut failing = seam.lock().unwrap_or_else(|e| e.into_inner());
    match failing.iter().position(|p| p == path) {
        Some(index) => {
            failing.remove(index);
            Err(DomainError::Session(
                "failed to write session: EIO (injected)".into(),
            ))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
impl FileSessionStore {
    /// The next write of `identity`'s transcript lands, then its fsync fails.
    pub(crate) fn fail_next_sync(&self, identity: &SessionIdentity) {
        let mut failing = FAIL_NEXT_SYNC_OF.lock().unwrap_or_else(|e| e.into_inner());
        failing.push(self.session_path(identity));
    }

    /// The next write of `identity`'s transcript fails before writing.
    pub(crate) fn fail_next_write(&self, identity: &SessionIdentity) {
        let mut failing = FAIL_NEXT_WRITE_OF.lock().unwrap_or_else(|e| e.into_inner());
        failing.push(self.session_path(identity));
    }
}
