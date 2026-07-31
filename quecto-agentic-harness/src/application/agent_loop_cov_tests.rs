use super::*;
use crate::application::agent_loop::tests::{MockProvider, MockRegistry, test_config};
use crate::domain::audit::{AuditEvent, AuditSink};
use crate::domain::message::{LlmResponse, Message, Role};
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingAudit {
    events: Mutex<Vec<(u32, String)>>,
}

impl AuditSink for RecordingAudit {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push((turn, format!("{event:?}")));
            Ok(())
        })
    }
}

#[test]
fn setters_swap_provider_streaming_and_audit_log() {
    let provider1 = Arc::new(MockProvider::new(vec![]));
    let provider2 = Arc::new(MockProvider::new(vec![]));
    let mut agent = AgentLoopImpl::new(test_config(provider1, Box::new(MockRegistry::new())));
    assert!(agent.audit_log().is_none());

    agent.swap_provider(provider2.clone());
    assert!(format!("{agent:?}").contains("mock"));
    agent.set_streaming(true);
    assert!(agent.streaming);

    let audit: Arc<dyn AuditSink> = Arc::new(RecordingAudit::default());
    agent.set_audit_log(Some(audit.clone()));
    assert!(Arc::ptr_eq(agent.audit_log().unwrap(), &audit));
    agent.set_audit_log(None);
    assert!(agent.audit_log().is_none());
}

#[test]
fn uds_extension_lifecycle_methods_delegate_to_registry_contracts() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut agent = AgentLoopImpl::new(test_config(provider, Box::new(ToolRegistryImpl::new())));
    assert!(
        agent.register_uds_extension_tool(Arc::new(super::tests::MockTool::new(
            "legacy_uds_tool",
            "ok"
        )))
    );
    assert!(
        agent
            .tool_registry_extension_names()
            .contains(&"legacy_uds_tool".to_string())
    );
    agent.unregister_extension_tool("legacy_uds_tool");
    assert!(
        !agent
            .tool_registry_extension_names()
            .contains(&"legacy_uds_tool".to_string())
    );
    agent.register_uds_extension_tool_for_owner(
        Arc::new(super::tests::MockTool::new("owned_tool", "ok")),
        "uds:client:55".into(),
    );
    agent.register_uds_extension_tool_for_owner(
        Arc::new(super::tests::MockTool::new("other_tool", "ok")),
        "uds:client:77".into(),
    );

    let removed = agent.unregister_uds_extension_tools_for_client(55);

    assert_eq!(removed, vec!["owned_tool".to_string()]);
    assert_eq!(
        agent.tool_registry_extension_names(),
        vec!["other_tool".to_string()]
    );
}

#[tokio::test]
async fn run_loop_emits_audit_for_non_streaming_text_response() {
    let provider = Arc::new(MockProvider::new(vec![LlmResponse {
        content: Some("hello".into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }]));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(MockRegistry::new()))
    });
    agent.set_streaming(false);
    let mut messages = vec![Message::user("hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(result.response, "hello");
    let events = audit.events.lock().unwrap();
    assert!(
        events.iter().any(|(_, e)| e.contains("LlmTurnStart")),
        "{events:?}"
    );
    assert!(
        events.iter().any(|(_, e)| e.contains("LlmTurnEnd")),
        "{events:?}"
    );
    assert!(messages.iter().any(|m| m.role == Role::Assistant));
}
