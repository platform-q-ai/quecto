use super::*;
use crate::domain::session::{SpillEntry, SpillIndex};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Default)]
struct FailingSpillStore {
    entries: Vec<SpillEntry>,
    fail_recall: bool,
    fail_list: bool,
}

impl ContextSpillStore for FailingSpillStore {
    fn append(
        &self,
        _session_key: &str,
        _entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        if self.fail_recall {
            return Box::pin(async { Err(DomainError::Session("spill read failed".into())) });
        }
        let found = self.entries.iter().find(|entry| entry.id == id).cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>> {
        if self.fail_list {
            return Box::pin(async { Err(DomainError::Session("spill index failed".into())) });
        }
        let index = self
            .entries
            .iter()
            .map(|entry| SpillIndex {
                id: entry.id.clone(),
                tool: entry.tool.clone(),
                input_preview: entry.input_preview.clone(),
                tokens: entry.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

fn entry(id: &str, preview: &str, tokens: usize, content: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: preview.to_string(),
        tokens,
        content: content.to_string(),
    }
}

#[tokio::test]
async fn missing_id_from_empty_arguments_reports_error_for_empty_id() {
    let tool = RecallTool::new(
        Arc::new(FailingSpillStore::default()),
        "session".to_string(),
    );

    let result = tool.execute(r#"{}"#).await.unwrap();

    assert!(result.is_error);
    assert_eq!(result.content, "No spilled output found for id: ");
}

#[tokio::test]
async fn list_index_includes_preview_and_token_counts_but_not_content() {
    let store = FailingSpillStore {
        entries: vec![
            entry("turn1:bash:0", "cargo test", 123, "full cargo output"),
            entry("turn2:grep:0", "rg TODO", 7, "secret grep output"),
        ],
        fail_recall: false,
        fail_list: false,
    };
    let tool = RecallTool::new(Arc::new(store), "session".to_string());

    let result = tool.execute(r#"{"id":"list"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Spilled outputs (2 entries):"));
    assert!(
        result
            .content
            .contains("turn1:bash:0 — cargo test (123 tokens)")
    );
    assert!(result.content.contains("turn2:grep:0 — rg TODO (7 tokens)"));
    assert!(!result.content.contains("full cargo output"));
    assert!(!result.content.contains("secret grep output"));
}

#[tokio::test]
async fn recall_propagates_spill_read_failure() {
    let tool = RecallTool::new(
        Arc::new(FailingSpillStore {
            fail_recall: true,
            ..FailingSpillStore::default()
        }),
        "session".to_string(),
    );

    let err = tool.execute(r#"{"id":"turn1:bash:0"}"#).await.unwrap_err();

    assert!(err.to_string().contains("spill read failed"), "got: {err}");
}

#[tokio::test]
async fn list_propagates_spill_index_failure() {
    let tool = RecallTool::new(
        Arc::new(FailingSpillStore {
            fail_list: true,
            ..FailingSpillStore::default()
        }),
        "session".to_string(),
    );

    let err = tool.execute(r#"{"id":"list"}"#).await.unwrap_err();

    assert!(err.to_string().contains("spill index failed"), "got: {err}");
}

#[tokio::test]
async fn recall_count_tracking_caps_new_ids_after_256_entries() {
    let tool = RecallTool::new(
        Arc::new(FailingSpillStore::default()),
        "session".to_string(),
    );

    for i in 0..300 {
        let _ = tool.execute(&format!(r#"{{"id":"missing-{i}"}}"#)).await;
    }

    let counts = tool.recall_counts.lock().unwrap();
    assert_eq!(counts.len(), 256);
    assert!(counts.contains_key("missing-0"));
    assert!(!counts.contains_key("missing-299"));
}

#[test]
fn debug_includes_session_key_when_lock_available() {
    let tool = RecallTool::new(
        Arc::new(FailingSpillStore::default()),
        "session-a".to_string(),
    );

    let rendered = format!("{tool:?}");

    assert!(rendered.contains("RecallTool"));
    assert!(rendered.contains("session-a"));
}
