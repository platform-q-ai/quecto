// File-based SessionStore: persists sessions as JSON files in a directory.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, StopReason, ThinkingBlock, ToolCall};
use crate::domain::session::{Session, SessionStore};

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

/// Serialise `StopReason` to a stable canonical string for persistence.
///
/// Uses the same strings that `StopReason::parse` accepts so that
/// round-trips are lossless regardless of which provider produced the value.
fn stop_reason_to_str(sr: &StopReason) -> String {
    match sr {
        StopReason::EndTurn => "end_turn".into(),
        StopReason::MaxTokens => "max_tokens".into(),
        StopReason::ToolUse => "tool_use".into(),
        StopReason::Refusal => "refusal".into(),
        StopReason::Error => "error".into(),
        StopReason::Aborted => "aborted".into(),
        StopReason::Unknown(s) => s.clone(),
    }
}

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
        stop_reason: msg.stop_reason.as_ref().map(stop_reason_to_str),
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_message(role: Role, content: &str) -> Message {
        match role {
            Role::System => Message::system(content),
            Role::User => Message::user(content),
            Role::Assistant => Message::assistant(content, vec![]),
            Role::Tool => Message::tool("call", content),
        }
    }

    #[tokio::test]
    async fn test_save_and_load_session() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let session = Session {
            key: "telegram:12345".to_string(),
            messages: vec![
                make_message(Role::User, "Hello"),
                make_message(Role::Assistant, "Hi there!"),
            ],
        };

        store.save(&session).await.unwrap();
        let loaded = store.load("telegram:12345").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.key, "telegram:12345");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "Hello");
        assert_eq!(loaded.messages[0].role, Role::User);
        assert_eq!(loaded.messages[1].content, "Hi there!");
        assert_eq!(loaded.messages[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_load_nonexistent_session() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let loaded = store.load("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_exists() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        assert!(!store.exists("telegram:12345").await.unwrap());

        let session = Session::new("telegram:12345");
        store.save(&session).await.unwrap();

        assert!(store.exists("telegram:12345").await.unwrap());
    }

    #[tokio::test]
    async fn test_key_to_filename() {
        assert_eq!(
            FileSessionStore::key_to_filename("telegram:12345"),
            "telegram_12345.json"
        );
        assert_eq!(
            FileSessionStore::key_to_filename("cli:default"),
            "cli_default.json"
        );
    }

    #[test]
    fn test_key_to_filename_sanitizes_path_traversal_chars() {
        let filename = FileSessionStore::key_to_filename("../../tmp/escape");
        assert!(!filename.contains(".."));
        assert!(!filename.contains('/'));
        assert!(!filename.contains('\\'));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn test_key_to_filename_avoids_collision_for_unsafe_keys() {
        let a = FileSessionStore::key_to_filename("a/b");
        let b = FileSessionStore::key_to_filename("a?b");
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn test_session_with_tool_calls() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let session = Session {
            key: "test:tools".to_string(),
            messages: vec![
                make_message(Role::User, "run a command"),
                Message::assistant(
                    String::new(),
                    vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "bash".to_string(),
                        arguments: r#"{"command":"ls"}"#.to_string(),
                    }],
                ),
                Message::tool("call_1", "file1.txt\nfile2.txt"),
            ],
        };

        store.save(&session).await.unwrap();
        let loaded = store.load("test:tools").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[1].tool_calls.len(), 1);
        assert_eq!(loaded.messages[1].tool_calls[0].name, "bash");
        assert_eq!(loaded.messages[2].tool_call_id.as_deref(), Some("call_1"));
    }

    #[tokio::test]
    async fn test_overwrite_existing_session() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut session = Session::new("test:overwrite");
        session.messages.push(make_message(Role::User, "first"));
        store.save(&session).await.unwrap();

        session
            .messages
            .push(make_message(Role::Assistant, "response"));
        store.save(&session).await.unwrap();

        let loaded = store.load("test:overwrite").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[test]
    fn test_session_build_key() {
        assert_eq!(Session::build_key("telegram", "12345"), "telegram:12345");
        assert_eq!(Session::build_key("cli", "default"), "cli:default");
    }

    #[tokio::test]
    async fn test_persistence_across_store_instances() {
        let tmp = TempDir::new().unwrap();

        // Save with one store instance
        let store1 = FileSessionStore::new(tmp.path());
        let session = Session {
            key: "telegram:persist".to_string(),
            messages: vec![make_message(Role::User, "persisted message")],
        };
        store1.save(&session).await.unwrap();

        // Load with a new store instance pointing to the same directory
        let store2 = FileSessionStore::new(tmp.path());
        let loaded = store2.load("telegram:persist").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().messages[0].content, "persisted message");
    }

    // --- Pruning metadata round-trip tests ---

    #[tokio::test]
    async fn test_turn_field_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut tool_msg = Message::tool("call_1", "tool output");
        tool_msg.turn = Some(3);

        let session = Session {
            key: "test:turn".to_string(),
            messages: vec![tool_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:turn").await.unwrap().unwrap();
        assert_eq!(
            loaded.messages[0].turn,
            Some(3),
            "turn field should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_is_collapsed_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut tool_msg = Message::tool("call_1", "[bash: echo hello (100 tokens)]");
        tool_msg.is_collapsed = true;

        let session = Session {
            key: "test:collapsed".to_string(),
            messages: vec![tool_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:collapsed").await.unwrap().unwrap();
        assert!(
            loaded.messages[0].is_collapsed,
            "is_collapsed should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_is_manifest_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut manifest = Message::system("[Session memory: 5 spilled entries]");
        manifest.is_manifest = true;
        manifest.is_pinned = true;

        let session = Session {
            key: "test:manifest".to_string(),
            messages: vec![manifest],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:manifest").await.unwrap().unwrap();
        assert!(
            loaded.messages[0].is_manifest,
            "is_manifest should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_is_pinned_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut user_msg = Message::user("first message");
        user_msg.is_pinned = true;

        let session = Session {
            key: "test:pinned".to_string(),
            messages: vec![user_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:pinned").await.unwrap().unwrap();
        assert!(
            loaded.messages[0].is_pinned,
            "is_pinned should survive save/load for non-system messages"
        );
    }

    #[tokio::test]
    async fn test_tool_name_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut tool_msg = Message::tool("call_1", "output");
        tool_msg.tool_name = Some("bash".to_string());

        let session = Session {
            key: "test:toolname".to_string(),
            messages: vec![tool_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:toolname").await.unwrap().unwrap();
        assert_eq!(
            loaded.messages[0].tool_name.as_deref(),
            Some("bash"),
            "tool_name should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_input_preview_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut tool_msg = Message::tool("call_1", "output");
        tool_msg.input_preview = Some("echo hello".to_string());

        let session = Session {
            key: "test:preview".to_string(),
            messages: vec![tool_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:preview").await.unwrap().unwrap();
        assert_eq!(
            loaded.messages[0].input_preview.as_deref(),
            Some("echo hello"),
            "input_preview should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_spill_id_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let mut tool_msg = Message::tool("call_1", "output");
        tool_msg.spill_id = Some("turn1:bash:0".to_string());

        let session = Session {
            key: "test:spillid".to_string(),
            messages: vec![tool_msg],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:spillid").await.unwrap().unwrap();
        assert_eq!(
            loaded.messages[0].spill_id.as_deref(),
            Some("turn1:bash:0"),
            "spill_id should survive save/load"
        );
    }

    #[tokio::test]
    async fn test_system_is_pinned_default_survives_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let session = Session {
            key: "test:sys_pinned".to_string(),
            messages: vec![Message::system("system prompt")],
        };
        store.save(&session).await.unwrap();
        let loaded = store.load("test:sys_pinned").await.unwrap().unwrap();
        // System messages are pinned by default in constructor,
        // so this should pass even without explicit persistence —
        // but user messages marked as pinned would fail.
        assert!(
            loaded.messages[0].is_pinned,
            "system message should remain pinned after round-trip"
        );
    }
}
