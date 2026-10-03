//! A child whose transcript carries the recall notice (#2218 review 2):
//! the notice is inserted at the head during a turn, after the task was
//! saved, so it is numbered after the task (`[2, 1, 3]`). A failed first
//! turn, a notice re-inserted after a transient spill-store error, and a
//! turn's own `agent_end` must all leave a complete report behind.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::super::fixture_tests::Fixture;
use super::delivery_tests::{READ, child_socket, read, serve};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role};
use crate::domain::session::SpillEntry;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;

/// Replies in order; `None` is a provider failure.
#[derive(Debug)]
struct Scripted(Mutex<VecDeque<Option<&'static str>>>);

impl LlmProvider for Scripted {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "scripted"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        let next = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted reply");
        Box::pin(async move {
            let content = next.ok_or_else(|| DomainError::Provider("scripted failure".into()))?;
            Ok(LlmResponse {
                content: Some(content.into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// A fixture whose agent retains context in a spill store that already
/// holds an entry, so every turn carries the recall notice.
async fn fixture_with_recall(replies: &[Option<&'static str>]) -> Fixture {
    let mut fx = Fixture::new();
    let spill: Arc<dyn ContextSpillStore> = Arc::new(FileContextSpillStore::new(
        FlatSessionLayout::new(fx._tmp.path()),
    ));
    let entry = SpillEntry {
        id: "spill-1".into(),
        tool: "read".into(),
        input_preview: "a big file".into(),
        tokens: 10,
        content: "retained".into(),
        images: Vec::new(),
    };
    spill
        .append(&SessionIdentity::from_persisted_key("cli:test"), &entry)
        .await
        .unwrap();
    let agent = crate::application::agent_loop::AgentLoopImpl::new(
        crate::application::agent_loop::AgentLoopConfig {
            provider: Arc::new(Scripted(Mutex::new(replies.iter().copied().collect()))),
            tool_registry: Box::new(
                crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
            ),
            model: "stub".into(),
            max_tokens: 100,
            temperature: 0.0,
            retention: Some(crate::composition::retention::context_retention_over(
                spill.clone(),
            )),
            session_key: "cli:test".into(),
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: 2,
            context_marks: Default::default(),
            model_context_window: None,
            tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
        },
    );
    fx.set_agent(agent);
    fx.set_retention(Some(spill));
    fx
}

async fn follow_up(fx: &mut Fixture, task: &str) {
    let mut ctx = fx.ctx();
    super::super::handle_follow_up(&mut ctx, Some("task"), "follow_up", task.into()).await;
}

fn tool_over(socket: std::path::PathBuf) -> AgentCmdTool {
    let registry = AgentCmdTool::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("w1".into(), SubagentEntry::new(socket, 0));
    AgentCmdTool::new(registry)
}

fn shape(fx: &Fixture) -> Vec<(Role, Option<u64>)> {
    fx.messages
        .iter()
        .map(|m| (m.role.clone(), m.ordinal))
        .collect()
}

#[tokio::test]
async fn a_failed_first_turn_leaves_a_complete_report_with_nothing_to_report() {
    let mut fx = fixture_with_recall(&[None]).await;
    follow_up(&mut fx, "task").await;
    assert_eq!(
        shape(&fx),
        [(Role::System, Some(2)), (Role::User, Some(1))],
        "the notice is numbered after the task saved before the turn"
    );
    let page = Arc::new(Mutex::new(serde_json::Value::Null));
    let (_tmp, socket) = child_socket(page.clone());
    serve(&fx, &page);
    let result = read(&tool_over(socket)).await;
    assert!(
        !result.content.contains("reportIncomplete"),
        "{}",
        result.content
    );
    // A page naming no report delivers its unread window and says no answer
    // was found, so the read completes and moves on (#2226).
    assert!(
        result.content.contains(r#""reportFound":false"#),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn a_notice_reinserted_after_the_task_still_reports_complete() {
    let mut fx = fixture_with_recall(&[Some("first report"), Some("second report")]).await;
    follow_up(&mut fx, "first task").await;
    assert_eq!(
        shape(&fx),
        [
            (Role::System, Some(2)),
            (Role::User, Some(1)),
            (Role::Assistant, Some(3))
        ]
    );
    let page = Arc::new(Mutex::new(serde_json::Value::Null));
    let (_tmp, socket) = child_socket(page.clone());
    let tool = tool_over(socket);
    serve(&fx, &page);
    let first = read(&tool).await;
    assert!(
        !first.content.contains("reportIncomplete"),
        "{}",
        first.content
    );
    assert!(first.content.contains("first report"), "{}", first.content);
    tool.result_delivered(READ, &first);

    // A transient spill-store error dropped the notice; the next turn
    // re-inserts it at the head, numbered after that turn's saved task.
    fx.messages.retain(|m| !m.is_manifest);
    follow_up(&mut fx, "second task").await;
    assert_eq!(fx.messages[0].ordinal, Some(5), "{:?}", shape(&fx));
    serve(&fx, &page);
    let second = read(&tool).await;
    assert!(
        !second.content.contains("reportIncomplete"),
        "{}",
        second.content
    );
    assert!(
        second.content.contains("second report"),
        "{}",
        second.content
    );
    assert!(
        !second.content.contains("first report"),
        "{}",
        second.content
    );
}

/// A writer that, when a turn's `agent_end` is written, records whether the
/// turn's report was already in the store.
struct EndProbe {
    session_file: std::path::PathBuf,
    saved_at_end: Arc<Mutex<Vec<bool>>>,
}

impl tokio::io::AsyncWrite for EndProbe {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if String::from_utf8_lossy(buf).contains(r#""type":"agent_end""#) {
            let stored = std::fs::read_to_string(&self.session_file).unwrap_or_default();
            self.saved_at_end
                .lock()
                .unwrap()
                .push(stored.contains("first report"));
        }
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn a_turn_is_saved_before_it_reports_its_end() {
    let mut fx = fixture_with_recall(&[Some("first report")]).await;
    let saved_at_end = Arc::new(Mutex::new(Vec::new()));
    let mut probe = EndProbe {
        session_file: fx._tmp.path().join("sessions").join("cli_test.json"),
        saved_at_end: saved_at_end.clone(),
    };
    {
        let mut ctx = fx.ctx();
        ctx.stdout = Some(&mut probe);
        super::super::handle_follow_up(&mut ctx, Some("task"), "follow_up", "task".into()).await;
    }
    assert_eq!(
        *saved_at_end.lock().unwrap(),
        [true],
        "agent_end is written only once the turn's report is durable"
    );
}
