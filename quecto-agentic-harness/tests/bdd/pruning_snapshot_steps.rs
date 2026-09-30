//! #2342 / #2349 review L1: a swarm member's context through the agent loop.
//! A newer board summary supersedes the older ones, which collapse to recall
//! stubs while the newest stays in full; a member's swarm ceiling prunes its
//! context at that ceiling.

use super::*;
use quecto::application::agent_loop::AgentLoopConfig;
use quecto::application::audit::ports::AuditSink;
use quecto::application::sessions::ports::{ContextSpillStore, SpillIndexList};
use quecto::domain::audit::AuditEvent;
use quecto::domain::session::{SpillEntry, SpillIndex};
use quecto::domain::session_identity::{SessionIdentity, SpillId};

const SUMMARY: &str = "swarm.summary";

/// A board whose every `summary` answer is a whole snapshot of it (the
/// swarm tool's rule, #2342), each read newer than the last.
#[derive(Debug, Default)]
struct BoardSummaries {
    reads: Mutex<u32>,
}

impl Tool for BoardSummaries {
    fn snapshot_key(&self, arguments: &str, content: &str) -> Option<&'static str> {
        (arguments.contains("summary") && content.starts_with("{\"members\"")).then_some(SUMMARY)
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "board".into(),
            description: "the swarm board".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let mut reads = self.reads.lock().unwrap();
        *reads += 1;
        let content = format!(
            r#"{{"members":["C1","W1"],"event_cursor":{reads},"tasks":[{}]}}"#,
            (0..40)
                .map(|i| format!(r#"{{"id":{i},"status":"claimed","read":{reads}}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
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

/// The spill store the member retains its context in.
#[derive(Debug, Default)]
pub struct MemberSpill(Mutex<Vec<SpillEntry>>);

impl ContextSpillStore for MemberSpill {
    fn append(
        &self,
        _session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.0.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &SessionIdentity,
        id: &SpillId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id.as_str())
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(&self, _session_key: &SessionIdentity) -> SpillIndexList<'_> {
        let index: Vec<SpillIndex> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
    }

    fn clear(
        &self,
        _session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.0.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

/// The event log's records of the member's run.
#[derive(Debug, Default)]
pub struct MemberAudit(Mutex<Vec<AuditEvent>>);

impl AuditSink for MemberAudit {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.0.lock().unwrap().push(event);
        Box::pin(async { Ok(()) })
    }
}

/// What a member's run left behind.
#[derive(Debug)]
pub struct MemberRun {
    spill: Arc<MemberSpill>,
    audit: Arc<MemberAudit>,
    messages: Vec<Message>,
}

impl MemberRun {
    fn summaries(&self) -> Vec<&Message> {
        self.messages
            .iter()
            .filter(|m| m.tool_name.as_deref() == Some("board"))
            .collect()
    }

    fn pruned(&self) -> Vec<(usize, usize)> {
        self.audit
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AuditEvent::ContextPruned {
                    snapshots_superseded,
                    ceiling_tokens,
                    ..
                } => Some((*snapshots_superseded, *ceiling_tokens)),
                _ => None,
            })
            .collect()
    }
}

fn run(world: &QuectoWorld) -> &MemberRun {
    world.member_run.as_ref().expect("the member ran")
}

#[given(expr = "the member reads the board summary {int} times, then replies {string}")]
fn given_member_reads_summaries(world: &mut QuectoWorld, reads: u32, reply: String) {
    let mock = super::agent_loop_steps::ensure_mock_llm(world);
    for read in 1..=reads {
        mock.push_response(LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: format!("summary-{read}"),
                name: "board".to_string(),
                arguments: r#"{"op":"summary"}"#.to_string(),
            }],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        });
    }
    mock.push_response(LlmResponse {
        content: Some(reply),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
}

#[given(expr = "the member's swarm ceiling is {int} tokens")]
fn given_member_ceiling(world: &mut QuectoWorld, tokens: usize) {
    world.member_ceiling = Some(tokens);
}

#[when(expr = "the user sends {string} through the swarm member agent")]
fn when_user_sends_through_member(world: &mut QuectoWorld, text: String) {
    let provider = world.mock_llm.clone().expect("mock LLM not configured") as Arc<dyn LlmProvider>;
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(BoardSummaries::default()));
    let spill = Arc::new(MemberSpill::default());
    let audit = Arc::new(MemberAudit::default());
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: Some(quecto::composition::retention::context_retention_over(
            spill.clone(),
        )),
        session_key: "bdd-2342".to_string(),
        context_collapse_after_tool_calls: 50,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: Some(audit.clone() as Arc<dyn AuditSink>),
        pin_recent_turns: 2,
        context_collapse_after_messages: 50,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Child,
    });
    // What the composition does when the member's process joins a swarm.
    if let Some(ceiling) = world.member_ceiling {
        agent.context_ceiling_cap().lower_to(ceiling);
    }
    let mut agent = agent;
    let mut messages = world.watermark_history.clone();
    world.watermark_pre_run = messages.clone();
    messages.push(Message::user(text));
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages))
        .expect("agent process failed");
    world.watermark_post_run = messages.clone();
    world.member_run = Some(MemberRun {
        spill,
        audit,
        messages,
    });
}

#[then("only the newest board summary is in full context")]
fn then_only_newest_in_full(world: &mut QuectoWorld) {
    let summaries = run(world).summaries();
    let (newest, older) = summaries.split_last().expect("the member read the board");
    assert!(!newest.is_collapsed, "the latest board state stays in full");
    assert!(newest.content.starts_with("{\"members\""));
    assert!(
        older.iter().all(|m| m.is_collapsed),
        "every superseded summary is a stub"
    );
}

#[then("every older board summary is a recall stub of its full answer")]
fn then_older_recallable(world: &mut QuectoWorld) {
    let run = run(world);
    let summaries = run.summaries();
    let (_, older) = summaries.split_last().expect("the member read the board");
    assert!(
        !older.is_empty(),
        "the member read the board more than once"
    );
    let retained = run.spill.0.lock().unwrap();
    for stub in older {
        let id = stub
            .spill_id
            .as_deref()
            .expect("a superseded summary was spilled");
        assert!(
            stub.content.contains(&format!("recall(\"{id}\")")),
            "{}",
            stub.content
        );
        let entry = retained
            .iter()
            .find(|e| e.id == id)
            .expect("its full answer is retained");
        assert!(
            entry.content.starts_with("{\"members\""),
            "{}",
            entry.content
        );
    }
}

#[then(expr = "the prune records count {int} superseded summaries")]
fn then_prune_records_count(world: &mut QuectoWorld, expected: usize) {
    let superseded: usize = run(world).pruned().iter().map(|(s, _)| s).sum();
    assert_eq!(superseded, expected, "{:?}", run(world).pruned());
}

#[then(expr = "the prune records name a ceiling of {int} tokens")]
fn then_prune_records_ceiling(world: &mut QuectoWorld, expected: usize) {
    let pruned = run(world).pruned();
    assert!(!pruned.is_empty(), "the member's context was pruned");
    assert!(
        pruned.iter().all(|&(_, ceiling)| ceiling == expected),
        "{pruned:?}"
    );
}
