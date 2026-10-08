//! #2414 review M2 (restoring #2348 review M1's coverage): when a cut
//! archives a result, or the emergency ladder drops one with its call, the
//! loop tells its tool, exactly once, so the `read` tool's cache forgets
//! that delivery and a re-read of the unchanged file answers its content,
//! not "unchanged".

use super::ctx_mgmt_tests::{CapturingAuditSink, MemSpillStore};
use crate::application::agent_loop::tests::{MockProvider, MockRegistry, test_config};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::application::tools::ports::Tool;
use crate::domain::audit::AuditEvent;
use crate::domain::conversation::services::turn_origin::prompt;
use crate::domain::conversation::services::watermark::Watermark;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, ToolCall};
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A `read` tool: a large file first, small ones after; it records the
/// results it is told left the context.
#[derive(Default)]
struct RecordingRead {
    calls: Mutex<u32>,
    told: Mutex<Vec<String>>,
}

impl Tool for RecordingRead {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "read".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn result_collapsed(&self, arguments: &str) {
        self.told.lock().unwrap().push(arguments.to_string());
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let content = match *calls {
            1 => "fn alpha() { let value = compute(); }\n".repeat(600),
            _ => "fn small() {}\n".to_string(),
        };
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

fn read_call(id: &str, path: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: "read".to_string(),
            arguments: format!(r#"{{"path":"{path}"}}"#),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn kinds(sink: &CapturingAuditSink) -> (usize, Vec<bool>) {
    let events = sink.events.lock().unwrap();
    let cuts = events
        .iter()
        .filter(|event| matches!(event, AuditEvent::ContextCut(_)))
        .count();
    let fallbacks = events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                watermark_fallback, ..
            } => Some(*watermark_fallback),
            _ => None,
        })
        .collect();
    (cuts, fallbacks)
}

/// A cut that archives a large read tells the read tool once; the reads
/// the cut keeps are never reported.
#[tokio::test]
async fn a_cut_that_archives_a_read_tells_its_tool_once() {
    let read = Arc::new(RecordingRead::default());
    let mut registry = MockRegistry::new();
    registry.register(read.clone());
    let provider = Arc::new(MockProvider::new(vec![
        read_call("r1", "big.rs"),
        read_call("r2", "a.rs"),
        read_call("r3", "b.rs"),
        read_call("r4", "c.rs"),
        crate::application::agent_loop::tests::text_response("done"),
    ]));
    let sink = Arc::new(CapturingAuditSink::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        context_marks: Watermark::new(4_000, 1_200).unwrap(),
        audit_log: Some(sink.clone() as Arc<dyn AuditSink>),
        ..test_config(provider, Box::new(registry))
    });
    let mut messages = vec![prompt("go".into())];
    agent.run_loop(&mut messages).await.unwrap();

    let (cuts, fallbacks) = kinds(&sink);
    assert_eq!((cuts, fallbacks), (1, vec![]), "one cut, no ladder");
    assert!(
        messages
            .iter()
            .all(|m| m.tool_call_id.as_deref() != Some("r1")),
        "the cut archived the large read"
    );
    assert_eq!(
        *read.told.lock().unwrap(),
        vec![r#"{"path":"big.rs"}"#.to_string()],
        "only the archived read is reported, once"
    );
}

/// The emergency ladder drops an unspilled read together with its call
/// when no cut can be made; its tool is still told, once.
#[tokio::test]
async fn a_read_the_ladder_drops_with_its_call_tells_its_tool_once() {
    let read = Arc::new(RecordingRead::default());
    let mut registry = MockRegistry::new();
    registry.register(read.clone());
    let provider = Arc::new(MockProvider::new(vec![
        crate::application::agent_loop::tests::text_response("done"),
    ]));
    let sink = Arc::new(CapturingAuditSink::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        max_context_tokens: 1_500,
        pin_recent_turns: 0,
        audit_log: Some(sink.clone() as Arc<dyn AuditSink>),
        ..test_config(provider, Box::new(registry))
    });
    // No retention: the read is never spilled, so the ladder can only
    // drop it, with its call.
    let mut call = Message::assistant(
        "",
        vec![ToolCall {
            id: "r1".to_string(),
            name: "read".to_string(),
            arguments: r#"{"path":"a.rs"}"#.to_string(),
        }],
    );
    call.turn = Some(1);
    let mut result = Message::tool("r1", "fn alpha() { let value = compute(); }\n".repeat(400));
    result.tool_name = Some("read".to_string());
    result.turn = Some(1);
    let mut reply = Message::assistant("read it", vec![]);
    reply.turn = Some(1);
    // A transcript with no user message (a cut keeps the brief and the
    // latest prompt, so it has no head to keep and is not made): only the
    // ladder can bring it under the ceiling.
    let mut messages = vec![Message::system("system"), call, result, reply];
    agent.run_loop(&mut messages).await.unwrap();

    let (cuts, fallbacks) = kinds(&sink);
    assert_eq!(cuts, 0, "no cut fits");
    assert_eq!(fallbacks.first(), Some(&true), "the ladder ran");
    assert!(
        messages
            .iter()
            .all(|m| m.tool_call_id.as_deref() != Some("r1")),
        "the read was dropped"
    );
    assert_eq!(
        *read.told.lock().unwrap(),
        vec![r#"{"path":"a.rs"}"#.to_string()],
        "the dropped read is reported, once"
    );
}
