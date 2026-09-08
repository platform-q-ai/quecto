use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, StopReason, ThinkingBlock, ToolCall};
use crate::domain::session::{ExecutionMetadataWrite, Session, SessionStore, SessionSummary};
use crate::domain::workflow::WorkflowRunPersisted;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
#[derive(Debug)]
pub struct FileSessionStore {
    sessions_dir: PathBuf,
    ownership: super::session_ownership::SessionOwnershipRegistry,
}
#[cfg(test)]
#[path = "session_store_1612_red_tests.rs"]
mod issue_1612_red_tests;
#[path = "session_store_message_records.rs"]
mod session_store_message_records;
#[path = "session_store_ordinals.rs"]
pub(crate) mod session_store_ordinals;
#[path = "session_store_records.rs"]
mod session_store_records;
use session_store_message_records::*;
use session_store_ordinals::{assign_missing_ordinals, messages_with_assigned_ordinals};
use session_store_records::*;
impl FileSessionStore {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            sessions_dir: base_dir.as_ref().join("sessions"),
            ownership: super::session_ownership::SessionOwnershipRegistry::default(),
        }
    }
    fn claim_key(&self, key: &str) -> Result<(), DomainError> {
        self.ownership.claim(&self.sessions_dir, key)
    }
    fn key_to_filename(key: &str) -> String {
        format!("{}.json", super::filename::sanitize_session_key(key))
    }
    fn session_path(&self, key: &str) -> PathBuf {
        self.sessions_dir.join(Self::key_to_filename(key))
    }
    pub async fn save_clean_delta(
        &self,
        key: &str,
        messages: &[Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Result<(), DomainError> {
        self.claim_key(key)?;
        let path = self.session_path(key);
        if messages.is_empty() && workflow_run.is_none() {
            let retains_state = load_existing_session(&path)
                .await?
                .is_some_and(|session| session_has_non_transcript_state(&session));
            if retains_state {
                // Compaction below preserves non-transcript state while clearing messages.
            } else {
                return self.delete_session_file_if_present(key).await;
            }
        }
        self.ensure_dir().await?;
        let must_compact = previously_persisted == 0
            || previously_persisted > messages.len()
            || !path.exists()
            || !is_jsonl_session_file(&path).await?;
        compact_or_append_delta(
            &path,
            key,
            messages,
            previously_persisted,
            workflow_run.as_ref(),
            must_compact,
        )
        .await
    }
    async fn delete_session_file_if_present(&self, key: &str) -> Result<(), DomainError> {
        match tokio::fs::remove_file(self.session_path(key)).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(DomainError::Session(format!(
                "failed to delete empty session: {err}"
            ))),
        }
    }
    async fn ensure_dir(&self) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(&self.sessions_dir)
            .await
            .map_err(|e| DomainError::Session(format!("failed to create sessions dir: {}", e)))
    }
}
impl SessionStore for FileSessionStore {
    fn claim(&self, key: &str) -> Result<(), DomainError> {
        self.claim_key(key)
    }
    fn release(&self, key: &str) {
        self.ownership.release(key);
    }
    fn load(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        let path = self.session_path(key);
        Box::pin(async move { load_existing_session(&path).await })
    }
    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.session_path(&session.key);
        let session = session.clone();
        Box::pin(async move {
            self.claim_key(&session.key)?;
            if session.messages.is_empty()
                && session.workflow_run.is_none()
                && session.subagent_roster.is_empty()
                && session.origin_execution_metadata().is_none()
                && session.latest_execution_metadata().is_none()
            {
                return self.delete_session_file_if_present(&session.key).await;
            }
            self.ensure_dir().await?;
            append_or_compact(&path, &session).await
        })
    }
    fn save_execution_metadata<'a>(
        &'a self,
        key: &'a str,
        write: ExecutionMetadataWrite,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>> {
        let path = self.session_path(key);
        Box::pin(async move {
            self.claim_key(key)?;
            self.ensure_dir().await?;
            let (latest, force_initialize) = match write {
                ExecutionMetadataWrite::Initialize(metadata) => (metadata, true),
                ExecutionMetadataWrite::UpdateLatest(metadata) => (metadata, false),
            };
            if path.exists() {
                let jsonl = is_jsonl_session_file(&path).await?;
                let existing = load_existing_session(&path)
                    .await?
                    .expect("path existence was established before loading");
                if existing.latest_execution_metadata() == Some(&latest)
                    && (!force_initialize || existing.origin_execution_metadata().is_some())
                {
                    return Ok(());
                }
                if force_initialize
                    || (existing.origin_execution_metadata().is_none()
                        && existing.latest_execution_metadata().is_none())
                {
                    let mut initialized = existing;
                    initialized.initialize_execution_metadata(latest.clone());
                    return write_compacted(&path, &initialized).await;
                }
                if !jsonl {
                    write_compacted(&path, &existing).await?;
                }
            } else {
                let mut session = Session::new(key);
                session.initialize_execution_metadata(latest.clone());
                return write_compacted(&path, &session).await;
            }
            append_record(
                &path,
                &SessionRecordRef::Metadata {
                    latest_execution_metadata: &latest,
                },
            )
            .await
        })
    }
    fn save_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.session_path(key);
        Box::pin(async move {
            self.claim_key(key)?;
            if messages.is_empty() && workflow_run.is_none() {
                let retains_state = load_existing_session(&path)
                    .await?
                    .is_some_and(|session| session_has_non_transcript_state(&session));
                if retains_state {
                    return compact_or_append_delta(
                        &path,
                        key,
                        messages,
                        previously_persisted,
                        None,
                        true,
                    )
                    .await;
                }
                return self.delete_session_file_if_present(key).await;
            }
            self.ensure_dir().await?;
            append_known_delta(
                &path,
                key,
                messages,
                previously_persisted,
                workflow_run.as_ref(),
            )
            .await
        })
    }
    fn save_clean_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            FileSessionStore::save_clean_delta(
                self,
                key,
                messages,
                previously_persisted,
                workflow_run,
            )
            .await
        })
    }
    fn exists(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        let path = self.session_path(key);
        Box::pin(async move { Ok(path.exists()) })
    }
    fn list(
        &self,
        key_prefix: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        let key_prefix = key_prefix.map(|p| p.to_string());
        let file_prefix = key_prefix
            .as_deref()
            .map(super::filename::sanitize_session_key);
        Box::pin(async move {
            let mut summaries = Vec::new();
            if !self.sessions_dir.exists() {
                return Ok(summaries);
            }
            let mut entries = tokio::fs::read_dir(&self.sessions_dir)
                .await
                .map_err(|e| DomainError::Session(format!("failed to read sessions dir: {}", e)))?;
            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                DomainError::Session(format!("failed to read sessions dir entry: {}", e))
            })? {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                if let Some(ref fp) = file_prefix {
                    if !entry.file_name().to_string_lossy().starts_with(fp.as_str()) {
                        continue;
                    }
                }
                let metadata = entry.metadata().await.ok();
                let updated_unix_secs = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                let content = match tokio::fs::read_to_string(&path).await {
                    Ok(content) => content,
                    Err(err) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "skipping unreadable session file while listing sessions"
                        );
                        continue;
                    }
                };
                let header: SessionHeader = match parse_session_header(&content) {
                    Ok(header) => header,
                    Err(err) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "skipping invalid session file while listing sessions"
                        );
                        continue;
                    }
                };
                if let Some(ref prefix) = key_prefix {
                    if !header.key.starts_with(prefix.as_str()) {
                        continue;
                    }
                }
                let title = first_user_message(&header.messages);
                let message_count = header
                    .messages
                    .iter()
                    .filter(|m| matches!(str_to_role(&m.role), Role::User | Role::Assistant))
                    .count();
                summaries.push(SessionSummary {
                    title,
                    key: header.key.into_owned(),
                    message_count,
                    updated_unix_secs,
                    latest_execution_metadata: header.latest_execution_metadata,
                });
            }
            summaries.sort_by(|a, b| {
                b.updated_unix_secs
                    .cmp(&a.updated_unix_secs)
                    .then_with(|| a.title.cmp(&b.title))
            });
            Ok(summaries)
        })
    }
}
fn parse_session_header(data: &str) -> Result<SessionHeader<'_>, serde_json::Error> {
    if let Ok(header) = serde_json::from_str::<SessionHeader<'_>>(data) {
        return Ok(header);
    }
    let mut key = std::borrow::Cow::Borrowed("");
    let mut messages = Vec::new();
    let mut latest_execution_metadata = None;
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record");
                break;
            }
            Err(err) => return Err(err),
        };
        parsed_any = true;
        match record {
            SessionRecord::Snapshot(file) => {
                key = file.key.into();
                latest_execution_metadata = file.latest_execution_metadata;
                messages = file
                    .messages
                    .into_iter()
                    .map(|message| MessageHeader {
                        role: message.role.into(),
                        content: message.content.into(),
                    })
                    .collect();
            }
            SessionRecord::Append {
                messages: added, ..
            } => {
                messages.extend(added.into_iter().map(|message| MessageHeader {
                    role: message.role.into(),
                    content: message.content.into(),
                }));
            }
            SessionRecord::Metadata {
                latest_execution_metadata: latest,
            } => latest_execution_metadata = Some(latest),
        }
    }
    Ok(SessionHeader {
        key,
        messages,
        latest_execution_metadata,
    })
}
fn parse_session_data(data: &str) -> Result<Session, serde_json::Error> {
    if let Ok(file) = serde_json::from_str::<SessionFile>(data) {
        return Ok(session_from_file(file));
    }
    let mut session: Option<Session> = None;
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record");
                break;
            }
            Err(err) => return Err(err),
        };
        parsed_any = true;
        match record {
            SessionRecord::Snapshot(file) => session = Some(session_from_file(*file)),
            SessionRecord::Append {
                start_index,
                messages,
                workflow_run,
                workflow_run_cleared,
                subagent_roster,
            } => {
                if let Some(session) = &mut session {
                    if let Some(start_index) = start_index {
                        if start_index != session.messages.len() {
                            tracing::warn!(
                                start_index,
                                current_len = session.messages.len(),
                                "ignoring out-of-order session append record"
                            );
                            break;
                        }
                    }
                    session
                        .messages
                        .extend(messages.into_iter().map(record_to_message));
                    if workflow_run_cleared {
                        session.workflow_run = None;
                    } else if workflow_run.is_some() {
                        session.workflow_run = workflow_run;
                    }
                    if let Some(roster) = subagent_roster {
                        session.subagent_roster = roster;
                    }
                }
            }
            SessionRecord::Metadata {
                latest_execution_metadata,
            } => {
                if let Some(session) = &mut session {
                    session.update_latest_execution_metadata(latest_execution_metadata);
                }
            }
        }
    }
    Ok(session
        .map(session_store_ordinals::with_assigned_ordinals)
        .unwrap_or_else(|| Session::new("")))
}
async fn load_existing_session(path: &Path) -> Result<Option<Session>, DomainError> {
    match tokio::fs::read_to_string(path).await {
        Ok(data) => parse_session_data(&data)
            .map(Some)
            .map_err(|error| DomainError::Session(format!("failed to parse session: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DomainError::Session(format!(
            "failed to read session: {error}"
        ))),
    }
}
fn session_has_non_transcript_state(session: &Session) -> bool {
    session.workflow_run.is_some()
        || !session.subagent_roster.is_empty()
        || session.origin_execution_metadata().is_some()
        || session.latest_execution_metadata().is_some()
}
fn session_from_file(file: SessionFile) -> Session {
    let messages =
        assign_missing_ordinals(file.messages.into_iter().map(record_to_message).collect());
    Session::restore(
        file.key,
        messages,
        file.workflow_run,
        file.subagent_roster,
        file.origin_execution_metadata,
        file.latest_execution_metadata,
    )
}
async fn is_jsonl_session_file(path: &Path) -> Result<bool, DomainError> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let mut prefix = [0_u8; 64];
    let len = file
        .read(&mut prefix)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let prefix = std::str::from_utf8(&prefix[..len]).unwrap_or("");
    Ok(prefix.trim_start().starts_with(r#"{"type":"#))
}
async fn persisted_prefix_changed(
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
    let persisted = parse_session_data(&data)
        .map_err(|e| DomainError::Session(format!("failed to parse session: {e}")))?;
    if persisted.messages.len() < previously_persisted {
        return Ok(false);
    }
    Ok(persisted.messages[..previously_persisted]
        .iter()
        .zip(messages_with_assigned_ordinals(&messages[..previously_persisted]).iter())
        .any(|(left, right)| message_to_record(left) != message_to_record(right)))
}

async fn append_known_delta(
    path: &Path,
    key: &str,
    messages: &[Message],
    previously_persisted: usize,
    workflow_run: Option<&WorkflowRunPersisted>,
) -> Result<(), DomainError> {
    let must_compact = previously_persisted == 0
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

async fn compact_or_append_delta(
    path: &Path,
    key: &str,
    messages: &[Message],
    previously_persisted: usize,
    workflow_run: Option<&WorkflowRunPersisted>,
    must_compact: bool,
) -> Result<(), DomainError> {
    if must_compact {
        let persisted = load_existing_session(path).await?;
        let subagent_roster = persisted
            .as_ref()
            .map(|session| session.subagent_roster.clone())
            .unwrap_or_default();
        let origin_execution_metadata = persisted
            .as_ref()
            .and_then(|session| session.origin_execution_metadata().cloned());
        let latest_execution_metadata = persisted
            .as_ref()
            .and_then(|session| session.latest_execution_metadata().cloned());
        let session = Session::restore(
            key,
            messages_with_assigned_ordinals(messages),
            workflow_run.cloned(),
            subagent_roster,
            origin_execution_metadata,
            latest_execution_metadata,
        );
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

async fn append_or_compact(path: &Path, session: &Session) -> Result<(), DomainError> {
    let mut assigned_session;
    let session = if session.messages.iter().any(|m| m.ordinal.is_none()) {
        assigned_session = session.clone();
        assigned_session.messages = messages_with_assigned_ordinals(&assigned_session.messages);
        &assigned_session
    } else {
        session
    };
    if !path.exists() || !is_jsonl_session_file(path).await? {
        return write_compacted(path, session).await;
    }

    let data = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let previous = parse_session_data(&data)
        .map_err(|e| DomainError::Session(format!("failed to parse session: {e}")))?;
    let mut session = session.clone();
    if session.origin_execution_metadata().is_none()
        && session.latest_execution_metadata().is_none()
        && (previous.origin_execution_metadata().is_some()
            || previous.latest_execution_metadata().is_some())
    {
        session.inherit_execution_metadata_from(&previous);
    }
    if previous.key != session.key
        || session.origin_execution_metadata() != previous.origin_execution_metadata()
        || session.latest_execution_metadata() != previous.latest_execution_metadata()
        || session.messages.len() < previous.messages.len()
        || session.messages[..previous.messages.len()]
            .iter()
            .map(message_to_record)
            .zip(previous.messages.iter().map(message_to_record))
            .any(|(current, saved)| current != saved)
    {
        return write_compacted(path, &session).await;
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

async fn write_compacted(path: &Path, session: &Session) -> Result<(), DomainError> {
    let record = SessionRecordRef::Snapshot(SessionFileRef {
        key: &session.key,
        messages: session.messages.iter().map(message_to_record_ref).collect(),
        workflow_run: session.workflow_run.as_ref(),
        subagent_roster: &session.subagent_roster,
        origin_execution_metadata: session.origin_execution_metadata(),
        latest_execution_metadata: session.latest_execution_metadata(),
    });
    let mut line = serde_json::to_string(&record)
        .map_err(|e| DomainError::Session(format!("failed to serialize session: {e}")))?;
    line.push('\n');
    write_session_bytes_atomically(path, line.as_bytes(), "failed to rename session").await
}

async fn append_record(path: &Path, record: &SessionRecordRef<'_>) -> Result<(), DomainError> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

    reject_symlink(path).await?;
    let mut line = serde_json::to_string(record)
        .map_err(|e| DomainError::Session(format!("failed to serialize session: {e}")))?;
    line.push('\n');

    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to open session for append: {e}")))?;
    let len = file
        .metadata()
        .await
        .map_err(|e| DomainError::Session(format!("failed to inspect session: {e}")))?
        .len();
    if len > 0 {
        file.seek(std::io::SeekFrom::End(-1))
            .await
            .map_err(|e| DomainError::Session(format!("failed to seek session: {e}")))?;
        let mut tail = [0_u8; 1];
        file.read_exact(&mut tail)
            .await
            .map_err(|e| DomainError::Session(format!("failed to read session tail: {e}")))?;
        if tail[0] == b'\n' {
            file.seek(std::io::SeekFrom::End(0))
                .await
                .map_err(|e| DomainError::Session(format!("failed to seek session: {e}")))?;
        } else {
            let existing = tokio::fs::read(path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
            let complete_len = existing
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |index| index + 1);
            file.set_len(complete_len as u64)
                .await
                .map_err(|e| DomainError::Session(format!("failed to truncate session: {e}")))?;
            file.seek(std::io::SeekFrom::End(0))
                .await
                .map_err(|e| DomainError::Session(format!("failed to seek session: {e}")))?;
        }
    }
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| DomainError::Session(format!("failed to append session: {e}")))?;
    file.flush()
        .await
        .map_err(|e| DomainError::Session(format!("failed to flush session: {e}")))
}

async fn write_session_bytes_atomically(
    path: &Path,
    bytes: &[u8],
    rename_context: &str,
) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;

    let tmp_path = path.with_extension("tmp");
    match tokio::fs::remove_file(&tmp_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DomainError::Session(format!(
                "failed to write session: could not remove stale temp file: {error}"
            )));
        }
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp_path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to write session: {e}")))?;
    file.write_all(bytes)
        .await
        .map_err(|e| DomainError::Session(format!("failed to write session: {e}")))?;
    file.flush()
        .await
        .map_err(|e| DomainError::Session(format!("failed to flush session: {e}")))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| DomainError::Session(format!("{rename_context}: {e}")))
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

#[cfg(test)]
#[path = "session_store_chat_tests.rs"]
mod chat_tests;
#[cfg(test)]
#[path = "session_store_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "session_store_metadata_tests.rs"]
mod metadata_tests;
#[cfg(test)]
#[path = "session_store_subagent_roster_tests.rs"]
mod subagent_roster_tests;
#[cfg(test)]
#[path = "session_store_tests.rs"]
mod tests;
