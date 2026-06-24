// File-based SessionStore: persists sessions as JSON files in a directory.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, StopReason, ThinkingBlock, ToolCall};
use crate::domain::session::{Session, SessionStore, SessionSummary};
use crate::domain::workflow::WorkflowRunPersisted;

/// File-based session store. Each session is stored as a JSON file
/// in `<base_dir>/sessions/`, with the filename derived from the session key.
#[derive(Debug)]
pub struct FileSessionStore {
    sessions_dir: PathBuf,
}

// -- Serializable structs for JSON persistence --

#[derive(serde::Serialize, serde::Deserialize)]
struct SessionFile {
    key: String,
    messages: Vec<MessageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_run: Option<WorkflowRunPersisted>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MessageRecord {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    // Context-pruning metadata (all optional for backward compat)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<u32>,
    /// `None` = absent in old files (use constructor default);
    /// `Some(true/false)` = explicitly persisted value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    is_pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    is_manifest: bool,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    is_collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spill_id: Option<String>,
    #[serde(default, skip_serializing_if = "skip_if_false")]
    is_error: bool,
    /// Stop reason for assistant messages (serialised as raw Anthropic string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    /// Extended thinking blocks from assistant messages (#437-5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    thinking_blocks: Vec<ThinkingBlockRecord>,
}

fn skip_if_false(v: &bool) -> bool {
    !v
}

/// Lightweight view of a session file used by `list()` to derive the title,
/// message count and key WITHOUT deserializing the full message bodies
/// (#765). Only the fields actually needed for a summary are described, so
/// heavy or even malformed per-message details (tool calls, thinking blocks,
/// pruning metadata) are skipped entirely by serde rather than parsed and
/// discarded — turning per-turn O(total_chars) list work into O(messages).
#[derive(serde::Deserialize)]
struct SessionHeader {
    key: String,
    #[serde(default)]
    messages: Vec<MessageHeader>,
}

/// Per-message header: just the role (for counting/title selection) and the
/// content (for the title). Every other field is ignored by serde.
#[derive(serde::Deserialize)]
struct MessageHeader {
    role: String,
    #[serde(default)]
    content: String,
}

/// Uses the same strings that `StopReason::parse` accepts so that
/// round-trips are lossless regardless of which provider produced the value.

#[derive(serde::Serialize, serde::Deserialize)]
struct ToolCallRecord {
    id: String,
    name: String,
    arguments: String,
}

/// Serializable representation of a thinking block (#437-5).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum ThinkingBlockRecord {
    /// Normal thinking block with visible reasoning text and signature.
    #[serde(rename = "normal")]
    Normal { thinking: String, signature: String },
    /// Redacted thinking block (reasoning hidden by safety filters).
    #[serde(rename = "redacted")]
    Redacted { data: String },
}

impl FileSessionStore {
    /// Create a new file-based session store rooted at the given directory.
    /// The `sessions/` subdirectory will be created if needed.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            sessions_dir: base_dir.as_ref().join("sessions"),
        }
    }

    /// Convert a session key to a safe filename with `.json` extension.
    fn key_to_filename(key: &str) -> String {
        format!("{}.json", super::filename::sanitize_session_key(key))
    }

    fn session_path(&self, key: &str) -> PathBuf {
        self.sessions_dir.join(Self::key_to_filename(key))
    }

    /// Ensure the sessions directory exists.
    async fn ensure_dir(&self) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(&self.sessions_dir)
            .await
            .map_err(|e| DomainError::Session(format!("failed to create sessions dir: {}", e)))
    }
}

impl SessionStore for FileSessionStore {
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
            let file: SessionFile = serde_json::from_str(&data)
                .map_err(|e| DomainError::Session(format!("failed to parse session: {}", e)))?;
            Ok(Some(Session {
                key: file.key,
                messages: file.messages.into_iter().map(record_to_message).collect(),
                workflow_run: file.workflow_run,
            }))
        })
    }

    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.session_path(&session.key);
        let file = SessionFile {
            key: session.key.clone(),
            messages: session.messages.iter().map(message_to_record).collect(),
            workflow_run: session.workflow_run.clone(),
        };
        Box::pin(async move {
            self.ensure_dir().await?;
            let json = serde_json::to_string_pretty(&file)
                .map_err(|e| DomainError::Session(format!("failed to serialize session: {}", e)))?;
            // Write atomically: temp file + rename
            let tmp_path = path.with_extension("tmp");
            tokio::fs::write(&tmp_path, json.as_bytes())
                .await
                .map_err(|e| DomainError::Session(format!("failed to write session: {}", e)))?;
            tokio::fs::rename(&tmp_path, &path)
                .await
                .map_err(|e| DomainError::Session(format!("failed to rename session: {}", e)))?;
            Ok(())
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
        // The caller owns the namespace policy. Pre-compute the sanitized
        // filename prefix so non-matching files can be skipped WITHOUT being
        // read or parsed (files on disk are named "<sanitized key>.json").
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
                // Cheap filename-level filter: skip files outside the requested
                // namespace before the costly read + parse.
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
                let header: SessionHeader = match serde_json::from_str(&content) {
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
                // Authoritative backstop on the real key.
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
                    key: header.key,
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

// -- Conversion helpers --

fn role_to_str(role: &Role) -> &str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
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
fn first_user_message(messages: &[MessageHeader]) -> String {
    const TRANSPORT_CHAR_CAP: usize = 200;
    messages
        .iter()
        .find(|m| matches!(str_to_role(&m.role), Role::User))
        .map(|m| m.content.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(TRANSPORT_CHAR_CAP).collect())
        .unwrap_or_default()
}

fn message_to_record(msg: &Message) -> MessageRecord {
    MessageRecord {
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
    msg.turn = rec.turn;
    msg.is_manifest = rec.is_manifest;
    msg.is_collapsed = rec.is_collapsed;
    msg.tool_name = rec.tool_name;
    msg.input_preview = rec.input_preview;
    msg.spill_id = rec.spill_id;
    // is_pinned: `Some(v)` = explicitly persisted, use it.
    // `None` = absent (old session file), keep constructor default
    // (true for System, false for others).
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
#[path = "session_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_store_chat_tests.rs"]
mod chat_tests;
