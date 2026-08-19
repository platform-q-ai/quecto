use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, StopReason, ThinkingBlock, ToolCall};
use crate::domain::session::{Session, SessionStore, SessionSummary};
use crate::domain::workflow::WorkflowRunPersisted;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

#[derive(Debug)]
pub struct FileSessionStore {
    sessions_dir: PathBuf,
    ownership: super::session_ownership::SessionOwnershipRegistry,
}

#[path = "session_store_ordinals.rs"]
pub(crate) mod session_store_ordinals;
#[path = "session_store_records.rs"]
mod session_store_records;
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
        if messages.is_empty() && workflow_run.is_none() {
            return self.delete_session_file_if_present(key).await;
        }
        self.ensure_dir().await?;
        let path = self.session_path(key);
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
        Box::pin(async move {
            if !path.exists() {
                return Ok(None);
            }
            let data = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to read session: {}", e)))?;
            let session = parse_session_data(&data)
                .map_err(|e| DomainError::Session(format!("failed to parse session: {}", e)))?;
            Ok(Some(session))
        })
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
            {
                return self.delete_session_file_if_present(&session.key).await;
            }
            self.ensure_dir().await?;
            append_or_compact(&path, &session).await
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
            self.claim_key(key)?;
            if messages.is_empty() && workflow_run.is_none() {
                return self.delete_session_file_if_present(key).await;
            }
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
        }
    }
    Ok(SessionHeader { key, messages })
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
            SessionRecord::Snapshot(file) => session = Some(session_from_file(file)),
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
        }
    }
    Ok(session
        .map(session_store_ordinals::with_assigned_ordinals)
        .unwrap_or_else(|| Session::new("")))
}

fn session_from_file(file: SessionFile) -> Session {
    let messages =
        assign_missing_ordinals(file.messages.into_iter().map(record_to_message).collect());
    Session {
        key: file.key,
        messages,
        workflow_run: file.workflow_run,
        subagent_roster: file.subagent_roster,
    }
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
        let subagent_roster = tokio::fs::read_to_string(path)
            .await
            .ok()
            .and_then(|data| parse_session_data(&data).ok())
            .map(|s| s.subagent_roster)
            .unwrap_or_default();
        let session = Session {
            key: key.to_string(),
            messages: messages_with_assigned_ordinals(messages),
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
    if previous.key != session.key
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

async fn write_compacted(path: &Path, session: &Session) -> Result<(), DomainError> {
    let record = SessionRecordRef::Snapshot(SessionFileRef {
        key: &session.key,
        messages: session.messages.iter().map(message_to_record_ref).collect(),
        workflow_run: session.workflow_run.as_ref(),
        subagent_roster: &session.subagent_roster,
    });
    let mut line = serde_json::to_string(&record)
        .map_err(|e| DomainError::Session(format!("failed to serialize session: {e}")))?;
    line.push('\n');
    let tmp_path = path.with_extension("tmp");
    tokio::fs::write(&tmp_path, line.as_bytes())
        .await
        .map_err(|e| DomainError::Session(format!("failed to write session: {e}")))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to rename session: {e}")))?;
    Ok(())
}

async fn append_record(path: &Path, record: &SessionRecordRef<'_>) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;

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
    file.flush()
        .await
        .map_err(|e| DomainError::Session(format!("failed to flush session: {e}")))?;
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

fn role_to_str(role: &Role) -> &str {
    role.as_str()
}

fn str_to_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Extract the raw title datum: the session's first user message, trimmed.
/// Returns an empty string when there is none, bounded to a transport-safe
/// length (no ellipsis). Display truncation and the "(untitled)" placeholder
/// are applied by the interface/display layer, not by persistence.
fn first_user_message(messages: &[MessageHeader<'_>]) -> String {
    const TRANSPORT_CHAR_CAP: usize = 200;
    messages
        .iter()
        .find(|m| matches!(str_to_role(&m.role), Role::User))
        .map(|m| m.content.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(TRANSPORT_CHAR_CAP).collect())
        .unwrap_or_default()
}

fn message_to_record_ref(msg: &Message) -> MessageRecordRef<'_> {
    MessageRecordRef {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role),
        content: &msg.content,
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecordRef {
                id: &tc.id,
                name: &tc.name,
                arguments: &tc.arguments,
            })
            .collect(),
        tool_call_id: msg.tool_call_id.as_deref(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        tool_name: msg.tool_name.as_deref(),
        input_preview: msg.input_preview.as_deref(),
        spill_id: msg.spill_id.as_deref(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(|tb| match tb {
                ThinkingBlock::Normal {
                    thinking,
                    signature,
                } => ThinkingBlockRecordRef::Normal {
                    thinking,
                    signature,
                },
                ThinkingBlock::Redacted { data } => ThinkingBlockRecordRef::Redacted { data },
            })
            .collect(),
    }
}

fn message_to_record(msg: &Message) -> MessageRecord {
    MessageRecord {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role).to_string(),
        content: msg.content.clone(),
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect(),
        tool_call_id: msg.tool_call_id.clone(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        tool_name: msg.tool_name.clone(),
        input_preview: msg.input_preview.clone(),
        spill_id: msg.spill_id.clone(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(|tb| match tb {
                ThinkingBlock::Normal {
                    thinking,
                    signature,
                } => ThinkingBlockRecord::Normal {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                },
                ThinkingBlock::Redacted { data } => {
                    ThinkingBlockRecord::Redacted { data: data.clone() }
                }
            })
            .collect(),
    }
}

fn record_to_message(rec: MessageRecord) -> Message {
    let role = str_to_role(&rec.role);
    let tool_calls = rec
        .tool_calls
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
        })
        .collect();
    let mut msg = match role {
        Role::System => Message::system(rec.content),
        Role::User => Message::user(rec.content),
        Role::Assistant => Message::assistant(rec.content, tool_calls),
        Role::Tool => Message::tool(rec.tool_call_id.unwrap_or_default(), rec.content),
    };
    msg.ordinal = rec.ordinal;
    msg.turn = rec.turn;
    msg.is_manifest = rec.is_manifest;
    msg.is_collapsed = rec.is_collapsed;
    msg.tool_name = rec.tool_name;
    msg.input_preview = rec.input_preview;
    msg.spill_id = rec.spill_id;
    msg.is_error = rec.is_error;
    msg.stop_reason = rec.stop_reason.as_deref().map(StopReason::parse);
    if let Some(pinned) = rec.is_pinned {
        msg.is_pinned = pinned;
    }
    msg.thinking_blocks = rec
        .thinking_blocks
        .into_iter()
        .map(|tb| match tb {
            ThinkingBlockRecord::Normal {
                thinking,
                signature,
            } => ThinkingBlock::Normal {
                thinking,
                signature,
            },
            ThinkingBlockRecord::Redacted { data } => ThinkingBlock::Redacted { data },
        })
        .collect();
    msg
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
