use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::{Message, Role, StopReason, ToolCall};
use crate::domain::session::{Session, SessionSummary};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::{error::DomainError, workflow::WorkflowRunPersisted};
use crate::infrastructure::turn_origin_names::{self as names, origin_from_name, origin_name};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use super::session_layout::FlatSessionLayout;

/// The file-backed session store: JSON/JSONL records under the flat layout
/// (`FlatSessionLayout` owns every path; this adapter owns the I/O).
#[derive(Debug)]
pub struct FileSessionStore {
    layout: FlatSessionLayout,
    ownership: super::session_ownership::SessionOwnershipRegistry,
    summaries: std::sync::Arc<std::sync::Mutex<session_store_list::SummaryCache>>,
    intact: session_store_write::IntactFiles,
}

#[path = "session_store_catalogue.rs"]
pub(super) mod session_store_catalogue;
#[path = "session_store_home.rs"]
pub(super) mod session_store_home;
#[path = "session_store_list.rs"]
pub(in crate::infrastructure::persistence) mod session_store_list;
#[path = "session_store_ordinals.rs"]
mod session_store_ordinals;
#[path = "session_store_records.rs"]
pub(super) mod session_store_records;
#[path = "session_store_write.rs"]
mod session_store_write;
use session_store_ordinals::{assign_missing_ordinals, messages_with_assigned_ordinals};
use session_store_records::*;
use session_store_write::{append_known_delta, append_or_compact, compact_or_append_delta};
#[cfg(test)]
use session_store_write::{append_record, persisted_prefix_changed, write_compacted};

impl FileSessionStore {
    /// A store over `layout`'s flat directory.
    pub fn new(layout: FlatSessionLayout) -> Self {
        Self {
            layout,
            summaries: Default::default(),
            ownership: super::session_ownership::SessionOwnershipRegistry::default(),
            intact: Default::default(),
        }
    }

    #[cfg(feature = "test-support")]
    pub fn summary_transcript_reads(&self) -> usize {
        self.summaries.lock().expect("summary cache lock").reads
    }

    fn claim_key(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.ownership.claim(&self.layout, identity)
    }

    fn session_path(&self, identity: &SessionIdentity) -> PathBuf {
        self.layout.session_file(identity)
    }

    pub async fn save_clean_delta(
        &self,
        identity: &SessionIdentity,
        messages: &[Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Result<(), DomainError> {
        self.claim_key(identity)?;
        if messages.is_empty() && workflow_run.is_none() {
            return self.delete_session_file_if_present(identity).await;
        }
        self.ensure_dir().await?;
        let path = self.session_path(identity);
        let target = &path;
        self.tracked(&path, |appendable| async move {
            let must_compact = previously_persisted == 0
                || !appendable
                || previously_persisted > messages.len()
                || !target.exists()
                || !is_jsonl_session_file(target).await?;
            compact_or_append_delta(
                target,
                identity,
                messages,
                previously_persisted,
                workflow_run.as_ref(),
                must_compact,
            )
            .await
        })
        .await
    }

    async fn ensure_dir(&self) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(self.layout.sessions_dir())
            .await
            .map_err(|e| DomainError::Session(format!("failed to create sessions dir: {}", e)))
    }
}

impl SessionStore for FileSessionStore {
    fn claim(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.claim_key(identity)
    }

    fn release(&self, identity: &SessionIdentity) {
        self.intact.forget(&self.session_path(identity));
        self.ownership.release(identity);
    }

    fn load(
        &self,
        identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        let path = self.session_path(identity);
        Box::pin(async move {
            if !path.exists() {
                return Ok(None);
            }
            let (data, before) = self.intact.read(&path).await?;
            let (session, intact) = parse_session_records(&data)
                .map_err(|e| DomainError::Session(format!("failed to parse session: {}", e)))?;
            self.intact
                .observe_read(&path, before, data.len(), intact)
                .await;
            Ok(Some(session))
        })
    }

    fn save<'a>(
        &'a self,
        session: &'a Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>> {
        let path = self.session_path(&session.key);
        Box::pin(async move {
            self.claim_key(&session.key)?;
            if session.messages.is_empty()
                && session.workflow_run.is_none()
                && session.subagent_roster.is_empty()
            {
                return self.delete_session_file_if_present(&session.key).await;
            }
            self.ensure_dir().await?;
            self.tracked(&path, |appendable| {
                append_or_compact(&path, session, appendable)
            })
            .await
        })
    }

    fn save_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.session_path(identity);
        Box::pin(async move {
            self.claim_key(identity)?;
            if messages.is_empty() && workflow_run.is_none() {
                return self.delete_session_file_if_present(identity).await;
            }
            self.ensure_dir().await?;
            self.tracked(&path, |appendable| {
                append_known_delta(
                    &path,
                    identity,
                    messages,
                    previously_persisted,
                    workflow_run.as_ref(),
                    appendable,
                )
            })
            .await
        })
    }

    fn save_clean_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.claim_key(identity)?;
            if messages.is_empty() && workflow_run.is_none() {
                return self.delete_session_file_if_present(identity).await;
            }
            FileSessionStore::save_clean_delta(
                self,
                identity,
                messages,
                previously_persisted,
                workflow_run,
            )
            .await
        })
    }

    fn exists(
        &self,
        identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        let path = self.session_path(identity);
        Box::pin(async move { Ok(path.exists()) })
    }

    fn list(
        &self,
        query: &SessionListQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        let query = query.clone();
        Box::pin(async move { Ok(self.walk(&query).await?.0) })
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
    parse_session_records(data).map(|(session, _)| session)
}

/// The session `data` holds, and whether every record of it was read and
/// the last one is terminated: an append may follow only an intact file.
fn parse_session_records(data: &str) -> Result<(Session, bool), serde_json::Error> {
    if let Ok(file) = serde_json::from_str::<SessionFile>(data) {
        // One snapshot line (a legacy plain-JSON file is compacted anyway).
        return Ok((session_from_file(file), data.ends_with('\n')));
    }
    let mut intact = data.ends_with('\n');

    let mut session: Option<Session> = None;
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record");
                intact = false;
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
                            intact = false;
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
    let session = session
        .map(session_store_ordinals::with_assigned_ordinals)
        .unwrap_or_else(|| Session::new(SessionIdentity::ephemeral()));
    Ok((session, intact))
}

fn session_from_file(file: SessionFile) -> Session {
    let messages =
        assign_missing_ordinals(file.messages.into_iter().map(record_to_message).collect());
    Session {
        key: SessionIdentity::from_persisted_key(file.key),
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
        turn_origin: origin_name(msg.turn_origin),
        user_kind: names::user_kind_name(msg.user_kind),
        tool_name: msg.tool_name.as_deref(),
        input_preview: msg.input_preview.as_deref(),
        spill_id: msg.spill_id.as_deref(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(ThinkingBlockRecordRef::from)
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
        turn_origin: origin_name(msg.turn_origin).map(str::to_string),
        user_kind: names::user_kind_name(msg.user_kind).map(str::to_string),
        tool_name: msg.tool_name.clone(),
        input_preview: msg.input_preview.clone(),
        spill_id: msg.spill_id.clone(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(ThinkingBlockRecord::from)
            .collect(),
    }
}

fn record_to_message(rec: MessageRecord) -> Message {
    let tool_calls = rec
        .tool_calls
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
        })
        .collect();
    let mut msg = match str_to_role(&rec.role) {
        Role::System => Message::system(rec.content),
        Role::User => Message::user(rec.content),
        Role::Assistant => Message::assistant(rec.content, tool_calls),
        Role::Tool => Message::tool(rec.tool_call_id.unwrap_or_default(), rec.content),
    };
    msg.ordinal = rec.ordinal;
    msg.turn = rec.turn;
    msg.is_manifest = rec.is_manifest;
    msg.is_collapsed = rec.is_collapsed;
    msg.turn_origin = origin_from_name(rec.turn_origin.as_deref());
    msg.user_kind = names::user_kind_from_name(rec.user_kind.as_deref());
    msg.tool_name = rec.tool_name;
    msg.input_preview = rec.input_preview;
    msg.spill_id = rec.spill_id;
    msg.is_error = rec.is_error;
    msg.stop_reason = rec.stop_reason.as_deref().map(StopReason::parse);
    msg.is_pinned = rec.is_pinned.unwrap_or(msg.is_pinned);
    msg.thinking_blocks = rec.thinking_blocks.into_iter().map(Into::into).collect();
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
#[cfg(test)]
#[path = "session_store_torn_tests.rs"]
mod torn_tests;
#[cfg(test)]
#[path = "session_store_workflow_tests.rs"]
mod workflow_tests;
