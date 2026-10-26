//! #2708: the trusted root parent's guard on the real UDS dispatch path —
//! a scripted provider, the real dispatcher and a broadcast sink.
use super::super::dispatch_test_env::DispatchTestEnv;
use super::super::*;
use crate::application::agent_turn::use_cases::parent_outcome::ParentOutcomeRuntime;
use crate::domain::agents::value_objects::turn_outcome::{
    CORRECTION_NOTE, EPISODE_ID_REFUSAL, LIVE_REASON, STALLED_REASON,
};
use crate::domain::conversation::value_objects::message::{LlmResponse, Role, ToolCall};
use crate::interface::cli::uds_session::PendingMessage;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// The two original promises of #2708: each ends the turn with nothing run.
pub(super) const PROMISE_2648: &str = "#2648 merged … Now working on #2650";
pub(super) const PROMISE_SPLIT_READER: &str =
    "B2 needs two PRs … I'm splitting out the bounded-reader work first";

/// One scripted reply: text, or a single tool call.
#[derive(Clone)]
pub(super) enum Reply {
    Text(&'static str),
    Call(&'static str, Value),
    Fail,
}

/// A provider that answers each request with the next scripted reply and
/// records the last user message of every request.
pub(super) struct Scripted {
    replies: Mutex<std::collections::VecDeque<Reply>>,
    pub(super) seen: Mutex<Vec<String>>,
    /// When set, each request records whether the correction note was
    /// already saved under this directory.
    pub(super) probe: Mutex<Option<std::path::PathBuf>>,
    pub(super) persisted: Mutex<Vec<bool>>,
}

/// Whether any file under `dir` holds `needle`.
fn saved_under(dir: &std::path::Path, needle: &str) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            match path.is_dir() {
                true => saved_under(&path, needle),
                false => std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)),
            }
        })
}

impl Scripted {
    pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            seen: Mutex::new(Vec::new()),
            probe: Mutex::new(None),
            persisted: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl std::fmt::Debug for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scripted")
    }
}

impl crate::application::providers::ports::LlmProvider for Scripted {
    fn route_order(&self) -> Vec<String> {
        vec!["scripted".into()]
    }
    fn name(&self) -> &str {
        "scripted"
    }
    fn chat(
        &self,
        request: crate::application::providers::ports::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, crate::domain::shared::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        self.seen.lock().unwrap().push(last_user);
        if let Some(dir) = self.probe.lock().unwrap().as_ref() {
            let saved = saved_under(dir, CORRECTION_NOTE);
            self.persisted.lock().unwrap().push(saved);
        }
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::Text("(unscripted)"));
        let (content, tool_calls) = match reply {
            Reply::Fail => {
                return Box::pin(async {
                    Err(crate::domain::shared::error::DomainError::Other(
                        "provider down".into(),
                    ))
                });
            }
            Reply::Text(text) => (Some(text.to_string()), vec![]),
            Reply::Call(name, arguments) => (
                None,
                vec![ToolCall {
                    id: format!("call-{}", self.seen.lock().unwrap().len()),
                    name: name.into(),
                    arguments: arguments.to_string(),
                }],
            ),
        };
        Box::pin(async move {
            Ok(LlmResponse {
                content,
                tool_calls,
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// A guarded root: the scripted provider, the declaration tool over the
/// runtime, a real registry and a broadcast sink whose frames are kept.
pub(super) struct Root {
    pub(super) env: DispatchTestEnv,
    pub(super) provider: Arc<Scripted>,
    pub(super) runtime: Arc<ParentOutcomeRuntime>,
    pub(super) tx: tokio::sync::broadcast::Sender<String>,
    pub(super) rx: tokio::sync::broadcast::Receiver<String>,
}

/// A test tool that registers a running child in the real registry, as a
/// successful spawn does, so the root owns live work.
struct LaunchTool(crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry);

impl crate::application::tools::ports::Tool for LaunchTool {
    fn definition(&self) -> crate::domain::tool_policy::value_objects::tool::ToolDefinition {
        crate::domain::tool_policy::value_objects::tool::ToolDefinition {
            name: "launch".into(),
            description: "Launch a child.".into(),
            parameters_schema: r#"{"type":"object","properties":{}}"#.into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::tool_policy::value_objects::tool::ToolResult,
                        crate::domain::shared::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        let mut entry = crate::infrastructure::tools::agents::subagent_registry::SubagentEntry::new(
            "/s".into(),
            0,
        );
        entry.status =
            crate::infrastructure::tools::agents::subagent_registry::SubagentStatus::Running;
        self.0.lock().unwrap().insert("child-1".into(), entry);
        Box::pin(async {
            Ok(
                crate::domain::tool_policy::value_objects::tool::ToolResult {
                    content: "launched child-1".into(),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                },
            )
        })
    }
}

impl Root {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        let provider = Scripted::new(replies);
        let mut env = DispatchTestEnv::with_unselected_workflow();
        let mut agent = super::super::dispatch_test_env::make_dispatch_test_agent(provider.clone());
        let runtime = crate::composition::bootstrap::agent_turn::build_parent_outcome();
        let registry = crate::infrastructure::tools::agents::subagent_registry::new_registry();
        assert!(agent.register_runtime_tool(Arc::new(
            crate::infrastructure::tools::native::declare_turn_outcome::DeclareTurnOutcomeTool::new(
                runtime.clone(),
                crate::domain::turn_declaration::LaunchAuthority::TopLevelParent,
            )
        )));
        assert!(agent.register_runtime_tool(Arc::new(LaunchTool(registry.clone()))));
        env.set_agent(agent);
        env.subagent_registry = Some(registry);
        env.session.bind_parent_outcome(Some(runtime.clone()));
        let (tx, rx) = tokio::sync::broadcast::channel(1024);
        Self {
            env,
            provider,
            runtime,
            tx,
            rx,
        }
    }

    /// An unguarded (legacy) harness over the same rig.
    pub(super) fn legacy(replies: Vec<Reply>) -> Self {
        let root = Self::new(replies);
        let Root {
            mut env,
            provider,
            runtime,
            tx,
            rx,
        } = root;
        env.session.bind_parent_outcome(None);
        Self {
            env,
            provider,
            runtime,
            tx,
            rx,
        }
    }

    /// Dispatch one command line through the real dispatcher.
    pub(super) async fn send(&mut self, line: &str) {
        let command = protocol::parse_command_line(line).expect("valid command");
        let tx = self.tx.clone();
        let mut ctx = self.env.ctx();
        ctx.broadcast_tx = Some(tx);
        ctx.stdout = None;
        dispatch_command(command, &mut ctx).await;
    }

    /// Dispatch one command line on the per-connection (writer) path and
    /// return the frames written to that connection.
    pub(super) async fn send_on_connection(&mut self, line: &str) -> Vec<Value> {
        let command = protocol::parse_command_line(line).expect("valid command");
        let mut written: Vec<u8> = Vec::new();
        {
            let mut ctx = self.env.ctx();
            ctx.broadcast_tx = None;
            ctx.stdout = Some(&mut written);
            dispatch_command(command, &mut ctx).await;
        }
        String::from_utf8(written)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("JSON frame"))
            .collect()
    }

    /// Every frame emitted so far, in order.
    pub(super) fn frames(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        while let Ok(line) = self.rx.try_recv() {
            frames.push(serde_json::from_str(line.trim_end()).expect("JSON frame"));
        }
        frames
    }
}

fn of_type<'a>(frames: &'a [Value], kind: &str) -> Vec<&'a Value> {
    frames
        .iter()
        .filter(|frame| frame["type"] == kind)
        .collect()
}

fn position(frames: &[Value], predicate: impl Fn(&Value) -> bool) -> usize {
    frames
        .iter()
        .position(predicate)
        .expect("expected frame present")
}

async fn assert_promise_is_corrected_once_then_stalls(promise: &'static str) {
    let mut root = Root::new(vec![Reply::Text(promise), Reply::Text(promise)]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on with the layout epic"}"#)
        .await;
    let requests = root.provider.requests();
    assert_eq!(
        requests.len(),
        2,
        "exactly two provider requests: {requests:?}"
    );
    assert_eq!(requests[1], CORRECTION_NOTE, "the literal correction, once");
    let frames = root.frames();
    let admitted = of_type(&frames, "turn_admitted");
    assert_eq!(
        admitted.len(),
        2,
        "the opener and the correction: {frames:?}"
    );
    assert!(
        admitted
            .iter()
            .all(|frame| frame["episodeId"] == "ep-1" && frame.as_object().unwrap().len() == 5)
    );
    assert_ne!(admitted[0]["turnId"], admitted[1]["turnId"]);
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "one outcome, never done: {frames:?}");
    assert_eq!(outcomes[0]["outcome"], "stalled");
    assert_eq!(outcomes[0]["reason"], STALLED_REASON);
    assert_eq!(outcomes[0]["turnId"], admitted[1]["turnId"]);
    assert_eq!(outcomes[0]["sequence"], 1);
    assert_eq!(outcomes[0].as_object().unwrap().len(), 8);
    // Order: ack, admitted, start, end, admitted, start, end, outcome.
    let ack = position(&frames, |f| f["type"] == "response" && f["id"] == "ep-1");
    let first = position(&frames, |f| f["type"] == "turn_admitted");
    let start = position(&frames, |f| f["type"] == "agent_start");
    let outcome = position(&frames, |f| f["type"] == "turn_outcome");
    let last_end = frames
        .iter()
        .rposition(|f| f["type"] == "agent_end")
        .unwrap();
    assert!(
        ack < first && first < start && last_end < outcome,
        "{frames:?}"
    );
    assert_eq!(frames[ack]["data"]["episodeId"], "ep-1");
    assert_eq!(
        of_type(&frames, "response")
            .iter()
            .filter(|f| f["id"] == "ep-1")
            .count(),
        1,
        "one acknowledgement, at admission"
    );
}

#[tokio::test]
async fn issue_2708_uds_parent_original_promise_2648_is_corrected_once_then_stalls() {
    assert_promise_is_corrected_once_then_stalls(PROMISE_2648).await;
}

#[tokio::test]
async fn issue_2708_uds_parent_original_split_reader_promise_is_corrected_once_then_stalls() {
    assert_promise_is_corrected_once_then_stalls(PROMISE_SPLIT_READER).await;
}

#[tokio::test]
async fn issue_2708_uds_parent_the_per_connection_path_is_guarded_the_same_way() {
    let mut root = Root::new(vec![Reply::Text(PROMISE_2648), Reply::Text(PROMISE_2648)]);
    let frames = root
        .send_on_connection(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    assert_eq!(root.provider.requests().len(), 2, "one correction");
    let admitted = of_type(&frames, "turn_admitted");
    assert_eq!(admitted.len(), 2, "{frames:?}");
    let first = position(&frames, |f| f["type"] == "turn_admitted");
    let start = position(&frames, |f| f["type"] == "agent_start");
    assert!(first < start, "admitted before the turn starts: {frames:?}");
    let outcome = of_type(&frames, "turn_outcome");
    assert_eq!(outcome.len(), 1);
    assert_eq!(outcome[0]["outcome"], "stalled");
}

#[tokio::test]
async fn issue_2708_uds_parent_declared_done_settles_done_without_a_correction() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Merged #2650"}),
        ),
        Reply::Text("All merged."),
    ]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Merge it"}"#)
        .await;
    assert_eq!(
        root.provider.requests().len(),
        2,
        "tool call, then the reply"
    );
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        (&outcomes[0]["outcome"], &outcomes[0]["reason"]),
        (&json!("done"), &json!("Merged #2650"))
    );
    assert_eq!(of_type(&frames, "turn_admitted").len(), 1);
}

#[tokio::test]
async fn issue_2708_uds_parent_corrected_blocked_needs_the_user() {
    let mut root = Root::new(vec![
        Reply::Text(PROMISE_2648),
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"blocked","reason":"Need the deploy key"}),
        ),
        Reply::Text("Blocked on the key."),
    ]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Deploy"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0]["outcome"], "needs_user");
    assert_eq!(outcomes[0]["reason"], "Need the deploy key");
}

#[tokio::test]
async fn issue_2708_uds_parent_launched_work_waits_without_a_correction() {
    let mut root = Root::new(vec![
        Reply::Call("launch", json!({})),
        Reply::Text("Launched the reviewer."),
    ]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Review"}"#)
        .await;
    assert_eq!(root.provider.requests().len(), 2, "no correction request");
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        (&outcomes[0]["outcome"], &outcomes[0]["reason"]),
        (&json!("waiting"), &json!(LIVE_REASON))
    );
}

#[tokio::test]
async fn issue_2708_terminal_autowake_terminal_fences_notes_until_a_fresh_instruction() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"asking","reason":"Which repository?"}),
        ),
        Reply::Text("Which repository?"),
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Used quecto"}),
        ),
        Reply::Text("Done."),
    ]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Start"}"#)
        .await;
    assert_eq!(root.provider.requests().len(), 2);
    root.env
        .session
        .restore_pending(std::iter::once(PendingMessage::Automatic(
            "child callback retained".into(),
        )));
    {
        let mut ctx = root.env.ctx();
        ctx.broadcast_tx = Some(root.tx.clone());
        ctx.stdout = None;
        drain_pending_and_nudge(&mut ctx).await;
    }
    assert_eq!(
        root.provider.requests().len(),
        2,
        "no automatic model turn after a terminal outcome"
    );
    assert!(
        root.env.session.holds_pending_work(),
        "the note is retained"
    );
    let _ = root.frames();
    root.send(r#"{"type":"prompt","id":"ep-2","message":"Use quecto"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "{frames:?}");
    assert_eq!(
        (&outcomes[0]["outcome"], &outcomes[0]["episodeId"]),
        (&json!("done"), &json!("ep-2"))
    );
    assert_eq!(outcomes[0]["sequence"], 2);
}

/// Review #2762 (4): the correction runs as its own turn; the prompt's
/// control receipt keeps the prompt's result, not the correction's.
#[tokio::test]
async fn review_2762_a_failed_correction_leaves_the_prompts_receipt_completed() {
    let mut root = Root::new(vec![Reply::Text(PROMISE_2648), Reply::Fail]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    let _ = root.frames();
    root.send(r#"{"type":"get_state","id":"s1"}"#).await;
    let state = root.frames().remove(0);
    let receipts = state["data"]["controlReceipts"].as_array().unwrap().clone();
    let receipt = receipts
        .iter()
        .find(|receipt| receipt["id"] == "ep-1")
        .expect("the prompt's receipt");
    assert_eq!(receipt["status"], "completed", "{receipts:?}");
}

/// Review #2762 (4): the correction is saved before it runs, as any other
/// drained turn's opener is.
#[tokio::test]
async fn review_2762_the_correction_note_is_saved_before_its_turn_runs() {
    let mut root = Root::new(vec![Reply::Text(PROMISE_2648), Reply::Text(PROMISE_2648)]);
    *root.provider.probe.lock().unwrap() = Some(root.env.tmp.path().to_path_buf());
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    assert_eq!(
        *root.provider.persisted.lock().unwrap(),
        vec![false, true],
        "the correction is on disk when its request is made"
    );
}

/// Review #2762 (9): a queued instruction alone is work: an undeclared turn
/// waits rather than being corrected.
#[tokio::test]
async fn review_2762_a_queued_instruction_alone_makes_an_undeclared_turn_wait() {
    let mut root = Root::new(vec![Reply::Text("Noted."), Reply::Text("Next.")]);
    let body = crate::interface::cli::uds_session::PromptBody {
        text: "Then this".into(),
        images: vec![],
    };
    let follow_up = crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP;
    assert!(
        root.env
            .session
            .enqueue_control(Some("ep-2"), "follow_up", body, follow_up)
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"First"}"#)
        .await;
    let frames = root.frames();
    let first = of_type(&frames, "turn_outcome")[0].clone();
    assert_eq!(
        (&first["episodeId"], &first["outcome"], &first["reason"]),
        (&json!("ep-1"), &json!("waiting"), &json!(LIVE_REASON))
    );
    let requests = root.provider.requests();
    assert_eq!(
        requests[1], "Then this",
        "no correction before it: {requests:?}"
    );
}

/// Review #2762 (A): a swarm the parent launched is live while its
/// coordinator is, idle or not: the parent's wake turn waits.
#[tokio::test]
async fn review_2762_an_open_swarm_with_an_idle_coordinator_keeps_a_wake_turn_waiting() {
    let mut root = Root::new(vec![
        Reply::Text("Started the swarm."),
        Reply::Text("Noted."),
    ]);
    {
        let registry = root.env.subagent_registry.clone().unwrap();
        let mut coordinator =
            crate::infrastructure::tools::agents::subagent_registry::SubagentEntry::new(
                "/s".into(),
                0,
            );
        coordinator.status =
            crate::infrastructure::tools::agents::subagent_registry::SubagentStatus::Idle;
        coordinator.coordinator_wake.launched = true;
        registry
            .lock()
            .unwrap()
            .insert("coordinator".into(), coordinator);
    }
    root.env
        .session
        .restore_pending(std::iter::once(PendingMessage::Automatic(
            "swarm report".into(),
        )));
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Run the swarm"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 2, "{frames:?}");
    assert_eq!(outcomes[1]["outcome"], "waiting", "{frames:?}");
    assert_eq!(root.provider.requests().len(), 2, "no correction");
}

/// Review #2762 (1): a note turn that ends terminal fences the next note,
/// but never the human's instruction queued behind it.
#[tokio::test]
async fn review_2762_instruction_queued_behind_two_notes_is_admitted_after_a_terminal_note_turn() {
    let mut root = Root::new(vec![
        Reply::Text("Working on it."),
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Note handled"}),
        ),
        Reply::Text("Done."),
        Reply::Text("Following up."),
    ]);
    root.env.session.restore_pending(
        [
            PendingMessage::Automatic("note A".into()),
            PendingMessage::Automatic("note B".into()),
        ]
        .into_iter(),
    );
    let body = crate::interface::cli::uds_session::PromptBody {
        text: "Then this".into(),
        images: vec![],
    };
    let follow_up = crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP;
    assert!(
        root.env
            .session
            .enqueue_control(Some("ep-2"), "follow_up", body, follow_up)
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Start"}"#)
        .await;
    let frames = root.frames();
    let done = position(&frames, |f| {
        f["type"] == "turn_outcome" && f["outcome"] == "done"
    });
    let second = position(&frames, |f| {
        f["type"] == "turn_admitted" && f["episodeId"] == "ep-2"
    });
    assert!(
        done < second,
        "the queued instruction is admitted: {frames:?}"
    );
    assert!(
        frames[done + 1..second]
            .iter()
            .all(|f| f["type"] != "agent_start"),
        "fenced note B does not run before it: {frames:?}"
    );
    let requests = root.provider.requests();
    assert_eq!(requests[3], "Then this", "{requests:?}");
}

#[tokio::test]
async fn issue_2708_protocol_a_queued_follow_up_is_admitted_right_after_the_terminal_outcome() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"First done"}),
        ),
        Reply::Text("First."),
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Second done"}),
        ),
        Reply::Text("Second."),
    ]);
    // A follow-up the client queued while the first turn ran.
    let body = crate::interface::cli::uds_session::PromptBody {
        text: "Then this".into(),
        images: vec![],
    };
    let follow_up = crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP;
    assert!(
        root.env
            .session
            .enqueue_control(Some("ep-2"), "follow_up", body, follow_up)
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"First"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 2, "{frames:?}");
    assert_eq!(
        (&outcomes[0]["episodeId"], &outcomes[0]["outcome"]),
        (&json!("ep-1"), &json!("done")),
        "queued work does not turn a declared done into waiting"
    );
    let first_outcome = position(&frames, |f| f["type"] == "turn_outcome");
    let first_end = position(&frames, |f| f["type"] == "agent_end");
    let second = position(&frames, |f| {
        f["type"] == "turn_admitted" && f["episodeId"] == "ep-2"
    });
    assert!(
        first_end < first_outcome && first_outcome < second,
        "{frames:?}"
    );
    assert!(
        frames[first_outcome + 1..second]
            .iter()
            .all(|f| f["type"] != "agent_start"),
        "no other turn runs between the outcome and the follow-up's admission"
    );
    assert_eq!(outcomes[1]["sequence"], 2);
}

#[tokio::test]
async fn issue_2708_protocol_episode_identity_is_bounded_echoed_and_minted() {
    let exact = "e".repeat(512);
    let mut root = Root::new(vec![Reply::Text("ok"); 8]);
    root.send(&json!({"type":"prompt","id": exact,"message":"go"}).to_string())
        .await;
    let frames = root.frames();
    let ack = &of_type(&frames, "response")[0];
    assert_eq!(ack["data"]["episodeId"], json!(exact));
    assert_eq!(
        of_type(&frames, "turn_admitted")[0]["episodeId"],
        json!(exact)
    );

    let calls = root.provider.requests().len();
    let over = "e".repeat(513);
    root.send(&json!({"type":"prompt","id": over,"message":"go"}).to_string())
        .await;
    let frames = root.frames();
    assert_eq!(
        root.provider.requests().len(),
        calls,
        "refused before commit"
    );
    assert_eq!(frames.len(), 1, "only the refusal: {frames:?}");
    assert_eq!(frames[0]["success"], false);
    assert_eq!(frames[0]["error"], EPISODE_ID_REFUSAL);

    root.send(r#"{"type":"prompt","message":"go"}"#).await;
    let frames = root.frames();
    let minted = of_type(&frames, "response")[0]["data"]["episodeId"].clone();
    assert!(minted.as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(of_type(&frames, "turn_admitted")[0]["episodeId"], minted);
}

#[tokio::test]
async fn issue_2708_protocol_get_state_carries_version_epoch_and_latest() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Finished"}),
        ),
        Reply::Text("Finished."),
    ]);
    root.send(r#"{"type":"get_state","id":"s1"}"#).await;
    let before = root.frames().remove(0);
    let epoch = root.runtime.view().epoch;
    assert_eq!(before["data"]["turnOutcomeVersion"], 1);
    assert_eq!(before["data"]["outcomeEpoch"], json!(epoch));
    assert_eq!(
        before["data"]["latestTurnOutcome"],
        Value::Null,
        "explicit null, never omitted"
    );
    assert!(
        before["data"]
            .as_object()
            .unwrap()
            .contains_key("latestTurnOutcome")
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Finish"}"#)
        .await;
    let event = of_type(&root.frames(), "turn_outcome")[0].clone();
    root.send(r#"{"type":"get_state","id":"s2"}"#).await;
    let after = root.frames().remove(0);
    assert_eq!(after["data"]["latestTurnOutcome"], event, "the same event");
    let generation = after["data"]["generation"].clone();
    root.send(&json!({"type":"get_state","id":"s3","since":generation}).to_string())
        .await;
    let unchanged = root.frames().remove(0);
    assert_eq!(unchanged["data"]["unchanged"], true);
    assert_eq!(unchanged["data"]["latestTurnOutcome"], event);
    assert_eq!(unchanged["data"]["outcomeEpoch"], json!(epoch));
    assert_eq!(unchanged["data"]["turnOutcomeVersion"], 1);
    assert_eq!(unchanged["data"]["sessionKey"], after["data"]["sessionKey"]);
}

#[tokio::test]
async fn issue_2708_protocol_clear_mints_a_new_epoch_and_a_null_latest() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Finished"}),
        ),
        Reply::Text("Finished."),
    ]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Finish"}"#)
        .await;
    let before = root.runtime.view();
    assert!(before.latest.is_some());
    root.send(r#"{"type":"clear_history","id":"c1"}"#).await;
    let after = root.runtime.view();
    assert_ne!(after.epoch, before.epoch);
    assert_eq!(after.latest, None);
    let _ = root.frames();
    root.send(r#"{"type":"set_effort","id":"e1","effort":"low"}"#)
        .await;
    assert_eq!(
        root.runtime.view().epoch,
        after.epoch,
        "an effort change keeps it"
    );
}

#[tokio::test]
async fn issue_2708_uds_parent_an_unguarded_harness_is_unchanged() {
    let mut root = Root::legacy(vec![Reply::Text(PROMISE_2648)]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    assert_eq!(root.provider.requests().len(), 1, "no correction");
    let frames = root.frames();
    assert!(of_type(&frames, "turn_admitted").is_empty());
    assert!(of_type(&frames, "turn_outcome").is_empty());
    let ack = position(&frames, |f| f["type"] == "response" && f["id"] == "ep-1");
    let end = position(&frames, |f| f["type"] == "agent_end");
    assert!(end < ack, "the legacy acknowledgement follows the turn");
    root.send(r#"{"type":"get_state","id":"s1"}"#).await;
    let state = root.frames().remove(0);
    for field in ["turnOutcomeVersion", "outcomeEpoch", "latestTurnOutcome"] {
        assert!(
            !state["data"].as_object().unwrap().contains_key(field),
            "{field}"
        );
    }
}
