//! A finished child's report through the supervisor's own tool (#2218):
//! the parent's `AgentCmdTool` reads the page the child's real state serves
//! after each turn, delivers it, and reads again. And while a later queued
//! turn runs, the earlier turns are already saved and numbered, the task
//! included.
use std::sync::{Arc, Mutex};

use super::super::fixture_tests::Fixture;
use super::persisted;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::tool::ToolResult;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;
use crate::interface::cli::uds_session::{HISTORY_PAGE_SIZE, messages_page_json};

pub(super) const READ: &str = r#"{"agent_id":"w1","command":"get_messages"}"#;

/// A child socket answering every `get_messages` with `page`: what the
/// child's dispatch loop serves from its state when the read arrives.
pub(super) fn child_socket(
    page: Arc<Mutex<serde_json::Value>>,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("child.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let page = page.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                while let Some(line) =
                    crate::infrastructure::test_support::read_framed_command(&stream)
                {
                    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["type"], "get_messages", "only reads are sent");
                    let response = serde_json::json!({"type": "response", "id": request["id"],
                        "command": "get_messages", "success": true,
                        "data": page.lock().unwrap().clone()});
                    let _ = writeln!(stream, "{response}");
                }
            });
        }
    });
    (tmp, path)
}

pub(super) async fn read(tool: &AgentCmdTool) -> ToolResult {
    let result = tool.execute(READ).await.expect("agent_cmd answers");
    assert!(!result.is_error, "{}", result.content);
    result
}

pub(super) fn serve(fx: &Fixture, page: &Mutex<serde_json::Value>) {
    *page.lock().unwrap() = messages_page_json(&fx.messages, HISTORY_PAGE_SIZE, None);
}

async fn follow_up(fx: &mut Fixture, task: &str) {
    let mut ctx = fx.ctx();
    super::super::handle_follow_up(&mut ctx, Some("task"), "follow_up", task.into()).await;
}

#[tokio::test]
async fn a_delivered_report_is_unchanged_and_a_later_turn_reports_only_its_messages() {
    let mut fx = Fixture::new();
    let page = Arc::new(Mutex::new(serde_json::Value::Null));
    let (_tmp, socket) = child_socket(page.clone());
    let registry = AgentCmdTool::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("w1".into(), SubagentEntry::new(socket, 0));
    let tool = AgentCmdTool::new(registry);

    follow_up(&mut fx, "first task").await;
    serve(&fx, &page);
    let first = read(&tool).await;
    assert!(first.content.contains("stub response"), "{}", first.content);
    assert!(
        !first.content.contains("reportIncomplete"),
        "{}",
        first.content
    );
    tool.result_delivered(READ, &first);

    serve(&fx, &page);
    let second = read(&tool).await;
    assert!(
        second.content.contains(r#""unchanged":true"#),
        "{}",
        second.content
    );
    tool.result_delivered(READ, &second);

    follow_up(&mut fx, "second task").await;
    serve(&fx, &page);
    let third = read(&tool).await;
    assert!(
        !third.content.contains("reportIncomplete"),
        "{}",
        third.content
    );
    assert!(third.content.contains("second task"), "{}", third.content);
    assert!(
        !third.content.contains("first task"),
        "only the new turn is unread: {}",
        third.content
    );
}

/// What the loop had published and saved when each model call began.
#[derive(Debug, Clone)]
struct Observed {
    published_ordinals: Vec<Option<u64>>,
    stored: Vec<String>,
}

/// A provider that records, at every call, the published transcript and
/// the store's content.
struct Probe {
    /// Filled once the agent is installed (installing recomposes the graph).
    state: Arc<Mutex<Option<ActiveSessionHandle>>>,
    store: Arc<crate::infrastructure::persistence::session_store::FileSessionStore>,
    observed: Arc<Mutex<Vec<Observed>>>,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Probe")
    }
}

impl LlmProvider for Probe {
    fn name(&self) -> &str {
        "probe"
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
        Box::pin(async move {
            use crate::application::sessions::ports::SessionStore;
            let state = self.state.lock().unwrap().clone().expect("installed");
            let published_ordinals = state
                .read()
                .await
                .conversation()
                .live_messages()
                .iter()
                .map(|m| m.ordinal)
                .collect();
            let identity =
                crate::domain::session_identity::SessionIdentity::from_persisted_key("cli:test");
            let stored = self
                .store
                .load(&identity)
                .await?
                .map(|s| s.messages.into_iter().map(|m| m.content).collect())
                .unwrap_or_default();
            self.observed.lock().unwrap().push(Observed {
                published_ordinals,
                stored,
            });
            Ok(LlmResponse {
                content: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[tokio::test]
async fn each_queued_turn_is_saved_before_the_next_and_its_task_before_itself() {
    let mut fx = Fixture::new();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(Mutex::new(None));
    let probe = Probe {
        state: state.clone(),
        store: fx.store.clone(),
        observed: observed.clone(),
    };
    fx.set_agent(super::super::super::dispatch_test_env::make_dispatch_test_agent(Arc::new(probe)));
    *state.lock().unwrap() = Some(fx.sessions.active_session.clone());
    fx.session.enqueue_pending("first task".into());
    fx.session.enqueue_pending("second task".into());
    {
        let mut ctx = fx.ctx();
        super::super::super::drain_pending_and_nudge(&mut ctx).await;
    }
    let observed = observed.lock().unwrap().clone();
    assert_eq!(observed.len(), 2, "two turns ran: {observed:?}");
    assert_eq!(
        observed[0].stored,
        ["first task"],
        "a drained task is saved before its own turn"
    );
    assert_eq!(
        observed[1].stored,
        ["first task", "done", "second task"],
        "the first turn is saved before the second runs"
    );
    let turn_one = &observed[1].published_ordinals;
    assert!(
        turn_one.len() >= 2 && turn_one[..2].iter().all(Option::is_some),
        "a read during the second turn sees the first one numbered: {turn_one:?}"
    );
    let stored = persisted(&fx).await.expect("saved");
    assert_eq!(stored.len(), 4);
}
