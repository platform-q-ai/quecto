//! #2708: a trusted root's one-shot run through the real `run_agent_session`
//! — a scripted provider, the real Native tool and the run's real registry
//! and note channel.
use super::*;
use crate::domain::agents::value_objects::turn_outcome::STALLED_REASON;
use crate::domain::conversation::value_objects::message::{LlmResponse, Role, ToolCall};
use crate::infrastructure::tools::agents::subagent_registry::{
    SequencedSubagentNotification, SubagentEntry, SubagentNotification, SubagentStatus,
    new_registry,
};
use crate::interface::cli::controllers::agent::{AgentFlags, parse_agent_flags, run_agent_session};
use crate::interface::cli::controllers::run_end_fleet::RunHandles;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const PROMISE_2648: &str = "#2648 merged … Now working on #2650";
const PROMISE_SPLIT_READER: &str =
    "B2 needs two PRs … I'm splitting out the bounded-reader work first";

#[derive(Clone)]
enum Reply {
    Text(&'static str),
    Call(&'static str, Value),
}

struct Scripted {
    replies: Mutex<std::collections::VecDeque<Reply>>,
    seen: Mutex<Vec<String>>,
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
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let index = {
            let mut seen = self.seen.lock().unwrap();
            seen.push(last_user);
            seen.len()
        };
        let reply = self.replies.lock().unwrap().pop_front();
        let (content, tool_calls) = match reply.unwrap_or(Reply::Text("(unscripted)")) {
            Reply::Text(text) => (Some(text.to_string()), vec![]),
            Reply::Call(name, arguments) => (
                None,
                vec![ToolCall {
                    id: format!("call-{index}"),
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

/// Registers a running child in the run's real registry, as a spawn does.
struct LaunchTool(SubagentRegistry);

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
                        DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        let mut entry = SubagentEntry::new("/s".into(), 0);
        entry.status = SubagentStatus::Running;
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

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    requests: Vec<String>,
}

fn flags(max_time: Option<&str>) -> AgentFlags {
    let mut args: Vec<String> = ["-m", "Carry on with the layout epic", "--no-session"]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    if let Some(secs) = max_time {
        args.extend(["--max-time".to_string(), secs.to_string()]);
    }
    let mut stderr = String::new();
    parse_agent_flags(&args, &mut stderr).unwrap_or_else(|| panic!("flags: {stderr}"))
}

/// Run one one-shot session: guarded unless `legacy`, with `notes` queued
/// on the run's note channel before it starts.
fn run(replies: Vec<Reply>, legacy: bool, notes: Vec<&str>, max_time: Option<&str>) -> Run {
    let provider = Arc::new(Scripted {
        replies: Mutex::new(replies.into()),
        seen: Mutex::new(Vec::new()),
    });
    let mut agent = crate::application::agent_turn::agent_loop::AgentLoopImpl::new(
        crate::application::agent_turn::agent_loop::AgentLoopConfig {
            provider: provider.clone(),
            tool_registry: Box::new(
                crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
            ),
            model: "test-model".to_string(),
            max_tokens: 100,
            temperature: 0.0,
            retention: None,
            session_key: String::new(),
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: 2,
            context_marks: Default::default(),
            model_context_window: None,
            tool_profile_context:
                crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
        },
    );
    let runtime = crate::composition::bootstrap::agent_turn::build_parent_outcome();
    let registry = new_registry();
    assert!(agent.register_runtime_tool(Arc::new(
        crate::infrastructure::tools::native::declare_turn_outcome::DeclareTurnOutcomeTool::new(
            runtime.clone(),
            crate::domain::turn_declaration::LaunchAuthority::TopLevelParent,
        )
    )));
    assert!(agent.register_runtime_tool(Arc::new(LaunchTool(registry.clone()))));
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    for (sequence, agent_id) in notes.into_iter().enumerate() {
        let note = SubagentNotification::Completed {
            agent_id: agent_id.into(),
        };
        tx.try_send(SequencedSubagentNotification::new(sequence as u64, note))
            .unwrap();
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let retention = crate::composition::sessions::build_retention_handles(tmp.path());
    let mut handles = RunHandles::without_children(&retention);
    handles.parent = ParentRun::of((!legacy).then_some(runtime), Some(registry), Some(rx));
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(
        tmp.path(),
        crate::composition::sessions::build_session_handles,
        agent,
        handles,
        &flags(max_time),
        &mut out,
    );
    drop(tx);
    let requests = provider.seen.lock().unwrap().clone();
    Run {
        code,
        stdout,
        stderr,
        requests,
    }
}

fn assert_promise_is_corrected_once_then_stalls(promise: &'static str) {
    let run = run(
        vec![Reply::Text(promise), Reply::Text(promise)],
        false,
        vec![],
        None,
    );
    assert_eq!(
        run.requests.len(),
        2,
        "exactly two requests: {:?}",
        run.requests
    );
    assert_eq!(run.requests[1], CORRECTION_NOTE);
    assert_eq!(run.code, 1, "stalled is not done");
    assert_eq!(run.stderr, format!("{STALLED_REASON}\n"));
    assert_eq!(run.stdout, format!("{promise}\n"), "the final reply, once");
}

#[test]
fn issue_2708_cli_parent_original_promise_2648_is_corrected_once_then_stalls() {
    assert_promise_is_corrected_once_then_stalls(PROMISE_2648);
}

#[test]
fn issue_2708_cli_parent_original_split_reader_promise_is_corrected_once_then_stalls() {
    assert_promise_is_corrected_once_then_stalls(PROMISE_SPLIT_READER);
}

#[test]
fn issue_2708_cli_parent_declared_done_exits_zero_without_a_correction() {
    let run = run(
        vec![
            Reply::Call(
                "declare_turn_outcome",
                json!({"outcome":"done","reason":"Merged #2650"}),
            ),
            Reply::Text("All merged."),
        ],
        false,
        vec![],
        None,
    );
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert_eq!(run.requests.len(), 2);
    assert_eq!(run.stdout, "All merged.\n");
}

#[test]
fn issue_2708_cli_parent_asking_exits_one_with_the_exact_question() {
    let run = run(
        vec![
            Reply::Call(
                "declare_turn_outcome",
                json!({"outcome":"asking","reason":"Which repository?"}),
            ),
            Reply::Text("Which repository?"),
        ],
        false,
        vec![],
        None,
    );
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr, "Which repository?\n");
}

#[test]
fn issue_2708_cli_parent_live_work_waits_for_its_note_then_continues() {
    let run = run(
        vec![
            Reply::Call("launch", json!({})),
            Reply::Text("Launched the reviewer."),
            Reply::Call(
                "declare_turn_outcome",
                json!({"outcome":"done","reason":"Reviewed"}),
            ),
            Reply::Text("Review complete."),
        ],
        false,
        vec!["reviewer"],
        None,
    );
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert_eq!(run.requests.len(), 4, "{:?}", run.requests);
    assert!(
        run.requests[2].contains("reviewer"),
        "the child's note opens the next run: {:?}",
        run.requests
    );
    assert!(
        !run.requests
            .iter()
            .any(|request| request == CORRECTION_NOTE)
    );
    assert_eq!(run.stdout, "Review complete.\n", "the final reply only");
}

#[test]
fn issue_2708_cli_parent_waiting_shares_the_one_deadline() {
    let run = run(
        vec![Reply::Call("launch", json!({})), Reply::Text("Launched.")],
        false,
        vec![],
        Some("1"),
    );
    assert_eq!(run.code, 2, "max-time stops the wait: {}", run.stderr);
    assert!(run.stderr.contains("max-time exceeded"));
}

#[test]
fn issue_2708_cli_parent_an_unguarded_run_is_unchanged() {
    let run = run(vec![Reply::Text(PROMISE_2648)], true, vec![], None);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert_eq!(run.requests.len(), 1);
}
