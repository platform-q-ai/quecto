//! #2342: a coordinator's context. A scripted coordinator conversation,
//! shaped like the owner's run (one large bash output early, a full swarm
//! summary every few turns, small results between), measured by the tokens
//! of every request the provider sees: before (every summary kept, the
//! 200k default ceiling) and after (superseded summaries collapsed, a swarm
//! member's 48k ceiling), at the shipped tool dial of 50 and at the 100 the
//! owner's runs pruned at.

use super::ctx_mgmt_tests::{CapturingAuditSink, MemSpillStore};
use crate::application::agent_loop::tests::MockRegistry;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::audit::AuditEvent;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::tool::{ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::application::context_pruning::large_results::LargeResultCollapse;

const SUMMARY_KEY: &str = "swarm.summary";
const TURNS: u32 = 100;
/// How the large bash output starts.
pub(super) const LARGE_HEAD: &str = "commit 0 touched";
/// A secret-shaped string inside a summary: no record may carry it.
pub(super) const SECRET: &str = "sk-live-2342SECRETSHAPEDVALUEabcdef0123456789";

/// The coordinator's script: a large bash output at turn 1, a summary
/// every 6th turn, a small bash result otherwise, then a closing reply.
fn scripted_call(turn: u32) -> (&'static str, String) {
    match turn {
        1 => ("bash", r#"{"command":"git log --stat"}"#.to_string()),
        t if t % 6 == 0 => ("swarm", r#"{"op":"summary"}"#.to_string()),
        t => ("bash", format!(r#"{{"command":"git status # {t}"}}"#)),
    }
}

/// The provider: answers the script and records what each request carries.
#[derive(Debug, Default)]
pub(super) struct ScriptedProvider {
    turn: Mutex<u32>,
    /// Estimated tokens of each request's messages.
    request_tokens: Mutex<Vec<usize>>,
    /// Per request: whether the latest delivered summary was there in full.
    pub(super) latest_summary_visible: Mutex<Vec<bool>>,
    /// Per request: how many summaries it carried in full.
    full_summaries: Mutex<Vec<usize>>,
    /// Per request: whether it carried the large bash output in full.
    pub(super) large_in_full: Mutex<Vec<bool>>,
    latest_summary: Arc<Mutex<Option<String>>>,
}

impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-coordinator"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let tokens = request.messages.iter().map(Message::estimated_tokens).sum();
        self.request_tokens.lock().unwrap().push(tokens);
        let latest = self.latest_summary.lock().unwrap().clone();
        let visible =
            latest.is_none_or(|latest| request.messages.iter().any(|m| m.content == latest));
        self.latest_summary_visible.lock().unwrap().push(visible);
        let full = request
            .messages
            .iter()
            .filter(|m| m.content.starts_with("{\"members\""))
            .count();
        self.full_summaries.lock().unwrap().push(full);
        let large = request
            .messages
            .iter()
            .any(|m| m.content.starts_with(LARGE_HEAD));
        self.large_in_full.lock().unwrap().push(large);
        let mut turn = self.turn.lock().unwrap();
        *turn += 1;
        let response = if *turn > TURNS {
            LlmResponse {
                content: Some("the run is complete".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }
        } else {
            let (name, arguments) = scripted_call(*turn);
            LlmResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: format!("call-{turn}"),
                    name: name.to_string(),
                    arguments,
                }],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }
        };
        Box::pin(async move { Ok(response) })
    }
}

/// `bash`: one large output (about 17k tokens) first, small ones after.
struct ScriptedBash {
    calls: Mutex<u32>,
}

impl Tool for ScriptedBash {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: "scripted bash".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let content = match *calls {
            1 => (0..1_700)
                .map(|i| format!("commit {i} touched src/file_{i}.rs lines\n"))
                .collect(),
            n => format!(
                "On branch work {n}\n{}",
                "modified: src/lib.rs\n".repeat(80)
            ),
        };
        Box::pin(async move { Ok(text(content)) })
    }
}

/// `swarm`: a full summary that grows as the run does. It names the
/// summary a snapshot only when `snapshots` is on (the "after" run).
struct ScriptedSwarm {
    reads: Mutex<u32>,
    snapshots: bool,
    latest_summary: Arc<Mutex<Option<String>>>,
}

impl Tool for ScriptedSwarm {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "swarm".into(),
            description: "scripted swarm".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn snapshot_key(&self, arguments: &str, content: &str) -> Option<&'static str> {
        let summary = arguments.contains("summary") && content.starts_with("{\"members\"");
        (self.snapshots && summary).then_some(SUMMARY_KEY)
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let mut reads = self.reads.lock().unwrap();
        *reads += 1;
        let tasks: Vec<String> = (0..30 + *reads * 6)
            .map(|i| format!(r#"{{"id":{i},"status":"claimed","owner":"W{}"}}"#, i % 3))
            .collect();
        let content = format!(
            r#"{{"members":["C1","W1","W2"],"note":"{SECRET}","event_cursor":{},"tasks":[{}]}}"#,
            *reads * 10,
            tasks.join(",")
        );
        *self.latest_summary.lock().unwrap() = Some(content.clone());
        Box::pin(async move { Ok(text(content)) })
    }
}

fn text(content: String) -> ToolResult {
    ToolResult {
        content,
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

pub(super) struct Run {
    pub(super) provider: Arc<ScriptedProvider>,
    pub(super) sink: Arc<CapturingAuditSink>,
}

impl Run {
    pub(super) fn total_request_tokens(&self) -> usize {
        self.provider.request_tokens.lock().unwrap().iter().sum()
    }
}

/// What a scripted run has on: summaries named snapshots, a swarm
/// member's ceiling, and the tool-result count dial.
#[derive(Clone, Copy, Debug)]
pub(super) struct Setup {
    pub(super) snapshots: bool,
    pub(super) cap: Option<usize>,
    pub(super) dial: u32,
    /// The size-aware collapse (#2348); off in the #2342 measurements.
    pub(super) large: LargeResultCollapse,
}

/// The shipped defaults before #2342: no snapshots, no cap, dial 50.
const BEFORE: Setup = Setup {
    snapshots: false,
    cap: None,
    dial: 50,
    large: LargeResultCollapse::DISABLED,
};
/// After #2342, as a joined swarm member runs: snapshots and the 48k cap.
pub(super) const AFTER: Setup = Setup {
    snapshots: true,
    cap: Some(48_000),
    dial: 50,
    large: LargeResultCollapse::DISABLED,
};

/// Run the scripted coordinator, `after` or before #2342.
async fn run_coordinator(after: bool) -> Run {
    run_scripted(if after { AFTER } else { BEFORE }).await
}

pub(super) async fn run_scripted(setup: Setup) -> Run {
    let provider = Arc::new(ScriptedProvider::default());
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(ScriptedBash {
        calls: Mutex::new(0),
    }));
    registry.register(Arc::new(ScriptedSwarm {
        reads: Mutex::new(0),
        snapshots: setup.snapshots,
        latest_summary: provider.latest_summary.clone(),
    }));
    let sink = Arc::new(CapturingAuditSink::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: "coordinator".to_string(),
        context_collapse_after_tool_calls: setup.dial,
        max_context_tokens: 200_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: Some(sink.clone() as Arc<dyn AuditSink>),
        pin_recent_turns: 2,
        context_collapse_after_messages: 50,
        large_result_collapse: setup.large,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Child,
    });
    if let Some(cap) = setup.cap {
        agent.context_ceiling_cap().lower_to(cap);
    }
    let mut messages = vec![
        Message::system("You coordinate a swarm."),
        Message::user("Run the swarm to completion."),
    ];
    agent.run_loop(&mut messages).await.unwrap();
    Run { provider, sink }
}

#[tokio::test]
async fn a_scripted_coordinator_sends_fewer_tokens_with_the_same_latest_state() {
    let before = run_coordinator(false).await;
    let after = run_coordinator(true).await;

    let (before_tokens, after_tokens) =
        (before.total_request_tokens(), after.total_request_tokens());
    let fewer = |before: usize, after: usize| 100.0 * (1.0 - after as f64 / before as f64);
    eprintln!(
        "#2342 scripted coordinator ({} requests): before {before_tokens}, after {after_tokens} tokens ({:.1}% fewer)",
        after.provider.request_tokens.lock().unwrap().len(),
        fewer(before_tokens, after_tokens),
    );
    for dial in [50, 100] {
        let base = Setup { dial, ..BEFORE };
        let base_tokens = run_scripted(base).await.total_request_tokens();
        for setup in [
            Setup {
                snapshots: true,
                ..base
            },
            Setup {
                cap: Some(48_000),
                ..base
            },
            Setup {
                snapshots: true,
                cap: Some(48_000),
                ..base
            },
        ] {
            let tokens = run_scripted(setup).await.total_request_tokens();
            eprintln!(
                "#2342   {setup:?}: {base_tokens} -> {tokens} ({:.1}% fewer)",
                fewer(base_tokens, tokens)
            );
        }
    }
    assert_eq!(
        before.provider.request_tokens.lock().unwrap().len(),
        after.provider.request_tokens.lock().unwrap().len(),
        "the same conversation: the model's calls do not change"
    );
    assert!(
        after_tokens < before_tokens,
        "with the shipped dials, fewer request tokens: before {before_tokens}, after {after_tokens}"
    );
    // The owner's runs pruned as a tool dial of 100 does: there the count
    // dial left a large result in place, and the swarm ceiling bounds it.
    let owners = Setup {
        dial: 100,
        ..BEFORE
    };
    let owners_before = run_scripted(owners).await.total_request_tokens();
    let owners_after = run_scripted(Setup { dial: 100, ..AFTER })
        .await
        .total_request_tokens();
    assert!(
        owners_after * 10 < owners_before * 8,
        "at least 20% fewer at the owner's dial: before {owners_before}, after {owners_after}"
    );
    assert!(
        after
            .provider
            .latest_summary_visible
            .lock()
            .unwrap()
            .iter()
            .all(|&visible| visible),
        "every request carries the latest board state in full"
    );
    assert!(
        after
            .provider
            .full_summaries
            .lock()
            .unwrap()
            .iter()
            .all(|&full| full <= 1),
        "no request carries a superseded summary in full"
    );
    assert!(
        before
            .provider
            .full_summaries
            .lock()
            .unwrap()
            .iter()
            .any(|&full| full > 1),
        "the baseline does carry superseded summaries"
    );
}

#[tokio::test]
async fn the_prune_record_counts_superseded_snapshots_and_the_ceiling_in_force() {
    let after = run_coordinator(true).await;

    let events = after.sink.events.lock().unwrap();
    let pruned: Vec<(usize, usize)> = events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                snapshots_superseded,
                ceiling_tokens,
                ..
            } => Some((*snapshots_superseded, *ceiling_tokens)),
            _ => None,
        })
        .collect();
    let superseded: usize = pruned.iter().map(|(superseded, _)| superseded).sum();
    // 16 summary reads: every one but the last is superseded, once.
    assert_eq!(superseded, (TURNS / 6) as usize - 1, "records: {pruned:?}");
    assert!(
        pruned.iter().all(|&(_, ceiling)| ceiling == 48_000),
        "each record names the swarm member's ceiling: {pruned:?}"
    );
    for event in events.iter() {
        let json = serde_json::to_string(event).unwrap();
        if json.contains("context_pruned") {
            assert!(!json.contains(SECRET), "a prune record carries counts only");
            assert!(!json.contains("members"), "no summary content: {json}");
        }
    }
}

#[tokio::test]
async fn without_snapshot_keys_nothing_is_superseded() {
    let before = run_coordinator(false).await;

    let events = before.sink.events.lock().unwrap();
    assert!(events.iter().all(|event| !matches!(
        event,
        AuditEvent::ContextPruned {
            snapshots_superseded: 1..,
            ..
        }
    )));
}

/// A tool that names every result a snapshot, yet fails every call.
struct FailingSnapshotTool;

impl Tool for FailingSnapshotTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "swarm".into(),
            description: "failing".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn snapshot_key(&self, _arguments: &str, _content: &str) -> Option<&'static str> {
        Some(SUMMARY_KEY)
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(ToolResult::from_error(&"the run is missing")) })
    }
}

#[tokio::test]
async fn a_failed_call_is_never_a_snapshot() {
    use crate::application::agent_loop::tests::{MockProvider, text_response};
    let call = |id: &str| LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: "swarm".to_string(),
            arguments: r#"{"op":"summary"}"#.to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    };
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(FailingSnapshotTool));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![
            call("a"),
            call("b"),
            text_response("done"),
        ])),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: "coordinator".to_string(),
        context_collapse_after_tool_calls: 50,
        max_context_tokens: 200_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: 50,
        large_result_collapse:
            crate::application::context_pruning::large_results::LargeResultCollapse::DISABLED,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Child,
    });
    let mut messages = vec![Message::user("go")];
    agent.run_loop(&mut messages).await.unwrap();

    let results: Vec<&Message> = messages
        .iter()
        .filter(|m| m.tool_call_id.is_some())
        .collect();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|m| m.snapshot_key.is_none() && !m.is_collapsed),
        "an error supersedes nothing and is superseded by nothing"
    );
}
