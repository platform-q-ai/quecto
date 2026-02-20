// File-based SessionStore: persists sessions as JSON files in a directory.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role, ToolCall};
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
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ToolCallRecord {
    id: String,
    name: String,
    arguments: String,
}

impl FileSessionStore {
    /// Create a new file-based session store rooted at the given directory.
    /// The `sessions/` subdirectory will be created if needed.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            sessions_dir: base_dir.as_ref().join("sessions"),
        }
    }

    /// Convert a session key to a safe filename.
    /// Replaces `:` with `_` for cross-platform compatibility.
    fn key_to_filename(key: &str) -> String {
        format!("{}.json", key.replace(':', "_"))
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
    }
}

fn record_to_message(rec: MessageRecord) -> Message {
    Message {
        role: str_to_role(&rec.role),
        content: rec.content,
        tool_calls: rec
            .tool_calls
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
            })
            .collect(),
        tool_call_id: rec.tool_call_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
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

    #[tokio::test]
    async fn test_session_with_tool_calls() {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());

        let session = Session {
            key: "test:tools".to_string(),
            messages: vec![
                make_message(Role::User, "run a command"),
                Message {
                    role: Role::Assistant,
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "exec".to_string(),
                        arguments: r#"{"command":"ls"}"#.to_string(),
                    }],
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: "file1.txt\nfile2.txt".to_string(),
                    tool_calls: vec![],
                    tool_call_id: Some("call_1".to_string()),
                },
            ],
        };

        store.save(&session).await.unwrap();
        let loaded = store.load("test:tools").await.unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[1].tool_calls.len(), 1);
        assert_eq!(loaded.messages[1].tool_calls[0].name, "exec");
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
}
