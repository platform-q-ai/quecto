use super::cov_tests::{RecordingSpillStore, SessionAwareTool, make_agent_with};
use crate::domain::message::Message;
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::{Tool, ToolRegistry};
use std::sync::Arc;

#[tokio::test]
async fn recording_spill_store_covers_append_recall_list_and_clear() {
    let store = RecordingSpillStore::default();
    let entry = SpillEntry {
        id: "spill-1".into(),
        tool: "message".into(),
        input_preview: "preview".into(),
        tokens: 1,
        content: "body".into(),
    };

    store.append("cli:surface", &entry).await.unwrap();
    assert!(
        store
            .recall("cli:surface", "spill-1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.list_entries("cli:surface").await.unwrap().is_empty());
    store.clear("cli:surface").await.unwrap();
    assert_eq!(store.cleared.lock().unwrap().as_slice(), ["cli:surface"]);
}

#[tokio::test]
async fn session_aware_tool_covers_set_session_key_and_execute() {
    let tool = SessionAwareTool::default();
    tool.set_session_key("cli:surface".into());
    let result = tool.execute("{}").await.unwrap();

    assert!(!result.is_error);
    assert_eq!(tool.seen.lock().unwrap().as_slice(), ["cli:surface"]);
}

#[tokio::test]
async fn registry_with_session_aware_tool_exposes_trait_surface() {
    let tool = Arc::new(SessionAwareTool::default());
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register_extension(tool.clone());
    assert_eq!(registry.tool_count(), 1);
    assert_eq!(
        registry.extension_names(),
        vec!["session_aware".to_string()]
    );

    let mut agent = make_agent_with(
        Box::new(registry),
        Some(Arc::new(RecordingSpillStore::default())),
    );
    agent.set_session_key("cli:surface".into());
    assert_eq!(tool.seen.lock().unwrap().as_slice(), ["cli:surface"]);

    // Keep the helper linked into this sibling module without running a turn.
    let _ = Message::user("surface");
}
