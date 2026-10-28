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
    TextAndCall(&'static str, &'static str, Value),
    Failure,
}

struct Scripted {
    replies: Mutex<std::collections::VecDeque<Reply>>,
    seen: Mutex<Vec<String>>,
    assistant_history: Mutex<Vec<String>>,
    correction_messages: Mutex<Vec<usize>>,
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
        self.assistant_history.lock().unwrap().extend(
            request
                .messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone()),
        );
        self.correction_messages.lock().unwrap().push(
            request
                .messages
                .iter()
                .filter(|m| m.role == Role::User && m.content == CORRECTION_NOTE)
                .count(),
        );
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
        let Some(reply) = reply else {
            return Box::pin(async {
                Err(DomainError::Provider("finite script exhausted".into()))
            });
        };
        let (content, tool_calls) = match reply {
            Reply::Text(text) => (Some(text.to_string()), vec![]),
            Reply::Failure => {
                return Box::pin(async { Err(DomainError::Provider("scripted failure".into())) });
            }
            Reply::TextAndCall(text, name, arguments) => (
                Some(text.to_string()),
                vec![ToolCall {
                    id: format!("call-{index}"),
                    name: name.into(),
                    arguments: arguments.to_string(),
                }],
            ),
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
    assistant_history: Vec<String>,
    correction_messages: Vec<usize>,
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
    run_quiet(replies, (legacy, notes, max_time), (None, false))
}

/// [`run`], with the quiet bound of a wait for notes set to `quiet`, and
/// the note channel `closed` before the run when asked.
fn run_quiet(
    replies: Vec<Reply>,
    (legacy, notes, max_time): (bool, Vec<&str>, Option<&str>),
    (quiet, closed): (Option<std::time::Duration>, bool),
) -> Run {
    run_composition(replies, (legacy, notes, max_time), (quiet, closed), None)
}

fn run_composition(
    replies: Vec<Reply>,
    (legacy, notes, max_time): (bool, Vec<&str>, Option<&str>),
    (quiet, closed): (Option<std::time::Duration>, bool),
    composed: Option<
        crate::interface::cli::controllers::agent::active_runtime_2708_tests::Composed,
    >,
) -> Run {
    run_composition_history(
        replies,
        (legacy, notes, max_time),
        (quiet, closed),
        composed,
        None,
    )
}

fn run_composition_history(
    replies: Vec<Reply>,
    (legacy, notes, max_time): (bool, Vec<&str>, Option<&str>),
    (quiet, closed): (Option<std::time::Duration>, bool),
    composed: Option<
        crate::interface::cli::controllers::agent::active_runtime_2708_tests::Composed,
    >,
    old_answer: Option<&str>,
) -> Run {
    let mut session_flags = flags(max_time);
    if composed.is_some() {
        match std::env::var("QUECTO_2708_TEST_ROOT")
            .expect("isolated root")
            .as_str()
        {
            "default" => session_flags.no_session = false,
            "named" => {
                session_flags.no_session = false;
                session_flags.session_name = Some("activation-root".into());
            }
            "ephemeral" => {}
            root => panic!("unexpected root {root}"),
        }
    }
    let (tools, composed_runtime) = match composed {
        Some(composed) => (composed.registry, Some(composed.runtime)),
        None => (
            crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
            None,
        ),
    };
    let provider = Arc::new(Scripted {
        replies: Mutex::new(replies.into()),
        seen: Mutex::new(Vec::new()),
        assistant_history: Mutex::new(Vec::new()),
        correction_messages: Mutex::new(Vec::new()),
    });
    let mut agent = crate::application::agent_turn::agent_loop::AgentLoopImpl::new(
        crate::application::agent_turn::agent_loop::AgentLoopConfig {
            provider: provider.clone(),
            tool_registry: Box::new(tools),
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
    let registry = new_registry();
    let runtime = match composed_runtime {
        Some(runtime) => runtime,
        None => {
            let runtime = crate::composition::bootstrap::agent_turn::build_parent_outcome();
            assert!(agent.register_runtime_tool(Arc::new(
                crate::infrastructure::tools::native::declare_turn_outcome::DeclareTurnOutcomeTool::new(
                    runtime.clone(),
                    crate::domain::turn_declaration::LaunchAuthority::TopLevelParent,
                )
            )));
            (!legacy).then_some(runtime)
        }
    };
    assert!(agent.register_runtime_tool(Arc::new(LaunchTool(registry.clone()))));
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    for (sequence, agent_id) in notes.into_iter().enumerate() {
        let note = SubagentNotification::Completed {
            agent_id: agent_id.into(),
        };
        tx.try_send(SequencedSubagentNotification::new(sequence as u64, note))
            .unwrap();
    }
    let tx = (!closed).then_some(tx);
    let tmp = tempfile::TempDir::new().unwrap();
    if let Some(old_answer) = old_answer {
        use crate::domain::conversation::value_objects::message::Message;
        use crate::domain::sessions::entities::session_identity::SessionIdentity;
        let identity =
            SessionIdentity::named_cli(session_flags.session_name.as_deref().unwrap_or("default"))
                .unwrap();
        let sessions = crate::composition::sessions::build_session_handles(
            crate::composition::sessions::SessionLoopInputs {
                base_dir: tmp.path().to_path_buf(),
                identity: identity.clone(),
                ephemeral: false,
                system_prompt: String::new(),
                spill_store: None,
                durable_prefix: crate::application::agent_turn::agent_loop::durable_prefix::DurablePrefixLatch::shared(),
                workflow_state: None,
                subagent_registry: None,
            },
        );
        let rt = crate::infrastructure::processes::tokio_runtime::build_tokio_runtime().unwrap();
        rt.block_on(async {
            sessions.switch.resume.open_at_startup().await.unwrap();
            let mut history = vec![
                Message::user("old question"),
                Message::assistant(old_answer, vec![]),
            ];
            sessions
                .save_session
                .save(
                    &mut history,
                    crate::application::sessions::dto::SaveTrigger::OrdinaryExit,
                )
                .await
                .unwrap();
        });
        sessions.store.release(&identity);
    }
    let retention = crate::composition::sessions::build_retention_handles(tmp.path());
    let mut handles = RunHandles::without_children(&retention);
    handles.parent = ParentRun::of(runtime, Some(registry), Some(rx)).map(|parent| match quiet {
        Some(quiet) => parent.quiet_for(quiet),
        None => parent,
    });
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
        &session_flags,
        &mut out,
    );
    drop(tx);
    let requests = provider.seen.lock().unwrap().clone();
    Run {
        code,
        stdout,
        stderr,
        requests,
        assistant_history: provider.assistant_history.lock().unwrap().clone(),
        correction_messages: provider.correction_messages.lock().unwrap().clone(),
    }
}

fn assert_promise_is_corrected_once_then_stalls(promise: &'static str) {
    let run = run(
        vec![Reply::Text(promise), Reply::Text(promise)],
        false,
        vec![],
        None,
    );
    assert_stalled(run, promise);
}

fn assert_stalled(run: Run, promise: &str) {
    assert_eq!(
        run.requests.len(),
        2,
        "exactly two requests: {:?}",
        run.requests
    );
    assert_eq!(run.requests[1], CORRECTION_NOTE);
    assert_eq!(
        run.requests
            .iter()
            .filter(|note| note.as_str() == CORRECTION_NOTE)
            .count(),
        1
    );
    assert_eq!(run.code, 1, "stalled is not done");
    assert_eq!(
        run.stderr,
        format!("turn outcome: stalled: {STALLED_REASON}\n")
    );
    assert_eq!(run.stdout, format!("{promise}\n"), "the final reply, once");
}

#[test]
fn issue_2708_cli_parent_original_promise_2648_is_corrected_once_then_stalls() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_original_promise_2648_is_corrected_once_then_stalls"
    )) {
        assert_promise_is_corrected_once_then_stalls(PROMISE_2648);
    }
}

#[test]
fn issue_2708_cli_parent_original_split_reader_promise_is_corrected_once_then_stalls() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_original_split_reader_promise_is_corrected_once_then_stalls"
    )) {
        assert_promise_is_corrected_once_then_stalls(PROMISE_SPLIT_READER);
    }
}

#[test]
fn issue_2708_cli_parent_declared_done_exits_zero_without_a_correction() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_declared_done_exits_zero_without_a_correction"
    )) {
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
        assert_eq!(
            (run.code, run.stderr.as_str()),
            (0, "turn outcome: done: Merged #2650\n")
        );
        assert_eq!(run.requests.len(), 2);
        assert_eq!(run.stdout, "All merged.\n");
    }
}

#[test]
fn issue_2708_cli_parent_asking_exits_zero_with_the_exact_question() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_asking_exits_zero_with_the_exact_question"
    )) {
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
        assert_eq!(run.code, 0);
        assert_eq!(run.stderr, "turn outcome: needs_user: Which repository?\n");
    }
}

#[test]
fn issue_2708_cli_parent_live_work_waits_for_its_note_then_continues() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_live_work_waits_for_its_note_then_continues"
    )) {
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
        assert_eq!(
            (run.code, run.stderr.as_str()),
            (0, "turn outcome: done: Reviewed\n")
        );
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
}

#[test]
fn issue_2708_cli_parent_waiting_shares_the_one_deadline() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_waiting_shares_the_one_deadline"
    )) {
        let run = run(
            vec![Reply::Call("launch", json!({})), Reply::Text("Launched.")],
            false,
            vec![],
            Some("1"),
        );
        assert_eq!(run.code, 2, "max-time stops the wait: {}", run.stderr);
        assert!(run.stderr.contains("max-time exceeded"));
    }
}

/// Review #2762 (5): with no --max-time, a wait in which no note arrives
/// for the quiet bound ends needs_user with the diagnostic, not forever.
#[test]
fn review_2762_a_wait_with_no_deadline_ends_after_the_quiet_bound() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::review_2762_a_wait_with_no_deadline_ends_after_the_quiet_bound"
    )) {
        let started = std::time::Instant::now();
        let run = run_quiet(
            vec![Reply::Call("launch", json!({})), Reply::Text("Launched.")],
            (false, vec![], None),
            (Some(std::time::Duration::from_millis(300)), false),
        );
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_eq!(
            run.stderr,
            format!(
                "turn outcome: needs_user: {}\n",
                crate::domain::agents::value_objects::turn_outcome::UNKNOWN_REASON
            )
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }
}

/// Review #2762 round 2: a wait whose note channel closes cannot tell
/// whether work still runs: it ends needs_user with the unknown line, not
/// "Work is already running.".
#[test]
fn review_2762_a_wait_whose_notes_close_ends_needs_user_unknown() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::review_2762_a_wait_whose_notes_close_ends_needs_user_unknown"
    )) {
        let run = run_quiet(
            vec![Reply::Call("launch", json!({})), Reply::Text("Launched.")],
            (false, vec![], None),
            (None, true),
        );
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_eq!(
            run.stderr,
            format!(
                "turn outcome: needs_user: {}\n",
                crate::domain::agents::value_objects::turn_outcome::UNKNOWN_REASON
            )
        );
    }
}

#[test]
fn issue_2708_cli_parent_an_unguarded_run_is_unchanged() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::issue_2708_cli_parent_an_unguarded_run_is_unchanged"
    )) {
        let run = run(vec![Reply::Text(PROMISE_2648)], true, vec![], None);
        assert_eq!((run.code, run.stderr.as_str()), (0, ""));
        assert_eq!(run.requests.len(), 1);
    }
}

fn production_promise(test: &str, promise: &'static str, switches: &[&str]) {
    use crate::interface::cli::controllers::agent::active_runtime_2708_tests::{compose, isolated};
    if isolated(test, switches) {
        let run = run_composition(
            vec![Reply::Text(promise), Reply::Text(promise)],
            (false, vec![], None),
            (None, true),
            Some(compose(false)),
        );
        assert_stalled(run, promise);
    }
}

#[test]
fn active_runtime_2708_cli_original_promise_2648_default_activation() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::active_runtime_2708_cli_original_promise_2648_default_activation"
    )) {
        production_promise(
            concat!(
                module_path!(),
                "::active_runtime_2708_cli_original_promise_2648_default_activation"
            ),
            PROMISE_2648,
            &["unset", "other"],
        );
    }
}

#[test]
fn active_runtime_2708_cli_original_split_reader_default_activation() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::active_runtime_2708_cli_original_split_reader_default_activation"
    )) {
        production_promise(
            concat!(
                module_path!(),
                "::active_runtime_2708_cli_original_split_reader_default_activation"
            ),
            PROMISE_SPLIT_READER,
            &["unset", "other"],
        );
    }
}

#[test]
fn active_runtime_2708_characterization_cli_opt_in() {
    if cold_review::bounded(concat!(
        module_path!(),
        "::active_runtime_2708_characterization_cli_opt_in"
    )) {
        let test = concat!(
            module_path!(),
            "::active_runtime_2708_characterization_cli_opt_in"
        );
        use crate::interface::cli::controllers::agent::active_runtime_2708_tests::{
            compose, isolated,
        };
        if isolated(test, &["1"]) {
            for promise in [PROMISE_2648, PROMISE_SPLIT_READER] {
                assert_stalled(
                    run_composition(
                        vec![Reply::Text(promise), Reply::Text(promise)],
                        (false, vec![], None),
                        (None, true),
                        Some(compose(false)),
                    ),
                    promise,
                );
            }
            for (outcome, reason, code) in [
                ("done", "Merged #2650", 0),
                ("asking", "Which repository?", 0),
                ("blocked", "Need the deploy key", 0),
            ] {
                let run = run_composition(
                    vec![
                        Reply::Call(
                            "declare_turn_outcome",
                            json!({"outcome": outcome, "reason": reason}),
                        ),
                        Reply::Text("Final."),
                    ],
                    (false, vec![], None),
                    (None, true),
                    Some(compose(false)),
                );
                assert_eq!(run.code, code);
                assert_eq!(run.requests.len(), 2);
                assert_eq!(
                    run.requests
                        .iter()
                        .filter(|note| note.as_str() == CORRECTION_NOTE)
                        .count(),
                    0
                );
                assert_eq!(
                    run.stderr,
                    format!(
                        "turn outcome: {}: {reason}\n",
                        if outcome == "done" {
                            "done"
                        } else {
                            "needs_user"
                        }
                    )
                );
            }
        }
    }
}

#[path = "run_session_parent_cold_review_tests.rs"]
mod cold_review;
