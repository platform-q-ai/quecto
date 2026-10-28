//! Operator cold-review M1/M2: real composition, engine, effective policy,
//! file reload and supervisor-owned child monitor. No new runtime behavior.
mod owned_child_tests;
mod policy_tests;
mod run28_duplicate_tests;
use super::super::dispatch_test_env::{DispatchTestEnv, make_dispatch_test_agent_with_registry};
use super::super::{dispatch_command, drain_pending_and_nudge, protocol};
use super::issue_2708_uds_parent_tests::{Reply, of_type};
use crate::application::agent_turn::ports::{BoundTurnDeclaration, DeclareTurnOutcome};
use crate::application::agent_turn::use_cases::parent_outcome::{
    ParentOutcomeRuntime, ParentVerdict, TurnAdmission, TurnEnd,
};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::agents::value_objects::turn_outcome::{
    CORRECTION_NOTE, DECLARE_GUIDANCE, WorkLiveness,
};
use crate::domain::conversation::value_objects::message::{LlmResponse, Role, ToolCall};
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolPolicyMutation, ToolPolicyRequest};
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use crate::domain::turn_declaration::{LaunchAuthority, TurnIdentity, TurnIntent};
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};
use crate::interface::cli::controllers::active_runtime_2708_tests::{
    Composed, compose, compose_with, compose_with_config,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Each hazardous probe gets its own hard process-group KILL and core0.
/// Parent owns TMPDIR and removes it after *all* child exits, including panic.
fn isolated(test: &str) -> bool {
    if std::env::var("RUN16_LIFECYCLE_CASE").ok().as_deref() == Some(test) {
        return true;
    }
    let (_, selected) = test.split_once("::").expect("qualified test");
    let mut failures = Vec::new();
    for switch in ["unset", "1"] {
        let scratch = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "ulimit -c 0; exec timeout --signal=KILL 45s \"$@\"",
            "run16",
        ]);
        command.arg(std::env::current_exe().unwrap());
        command.args(["--exact", selected, "--nocapture", "--test-threads=1"]);
        command.env("RUN16_LIFECYCLE_CASE", test);
        command.env("RUN16_SCRATCH", scratch.path());
        command.env("TMPDIR", scratch.path());
        command.env("QUECTO_2708_TEST_ROOT", "default");
        command.env_remove("QUECTO_TURN_OUTCOMES");
        if switch == "1" {
            command.env("QUECTO_TURN_OUTCOMES", "1");
        }
        let output = command.output().expect("bounded exact lifecycle child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{switch}: {}\n{stdout}\n{stderr}", output.status);
        let selected_one = stdout.contains("running 1 test\n");
        let success = output.status.success();
        scratch
            .close()
            .expect("cleanup probe scratch after success/failure");
        assert!(selected_one, "nonzero exact selection: {selected}");
        match success {
            true => {}
            false => failures.push(format!("{switch}: {}", output.status)),
        }
    }
    assert!(
        failures.is_empty(),
        "bounded lifecycle children failed: {failures:?}"
    );
    false
}

#[derive(Clone, Debug)]
struct Request {
    last_user: String,
    tools: Vec<String>,
    guidance: bool,
    identity: Option<TurnIdentity>,
}

struct Probe {
    script: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Request>>,
    binding: Option<Arc<ParentOutcomeRuntime>>,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FiniteLifecycleProbe")
    }
}

impl Probe {
    fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

impl LlmProvider for Probe {
    fn name(&self) -> &str {
        "run16-finite"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        self.seen.lock().unwrap().push(Request {
            last_user: request
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            tools: request.tools.iter().map(|t| t.name.to_string()).collect(),
            guidance: request
                .messages
                .iter()
                .any(|m| m.role == Role::System && m.content.contains(DECLARE_GUIDANCE)),
            identity: self.binding.as_ref().and_then(|runtime| runtime.capture()),
        });
        let reply = self.script.lock().unwrap().pop_front();
        let result = match reply {
            Some(Reply::Text(text)) => Ok((Some(text.to_owned()), vec![])),
            Some(Reply::Call(name, args)) => Ok((
                None,
                vec![ToolCall {
                    id: format!("run16-{}", self.requests().len()),
                    name: name.into(),
                    arguments: args.to_string(),
                }],
            )),
            Some(Reply::Fail) | None => Err(DomainError::Other(
                "finite lifecycle fixture exhausted".into(),
            )),
        };
        Box::pin(async move {
            let (content, tool_calls) = result?;
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

struct Rig {
    env: DispatchTestEnv,
    provider: Arc<Probe>,
    runtime: Option<Arc<ParentOutcomeRuntime>>,
    tx: tokio::sync::broadcast::Sender<String>,
    rx: tokio::sync::broadcast::Receiver<String>,
    snippets: String,
    reload: Option<Arc<crate::application::catalogue::use_cases::ReloadRuntimeConfiguration>>,
    _home: tempfile::TempDir,
}

impl Rig {
    fn new(replies: Vec<Reply>) -> Self {
        Self::composed(compose(true), replies)
    }
    fn composed(composed: Composed, replies: Vec<Reply>) -> Self {
        let provider = Arc::new(Probe {
            script: Mutex::new(replies.into()),
            seen: Mutex::new(vec![]),
            binding: composed.runtime.clone(),
        });
        let mut env = DispatchTestEnv::with_unselected_workflow();
        if let Some(workflow) = composed.workflow {
            env.workflow = workflow;
        }
        env.set_agent(make_dispatch_test_agent_with_registry(
            provider.clone(),
            composed.registry,
        ));
        env.session.bind_parent_outcome(composed.runtime.clone());
        env.subagent_registry =
            Some(crate::infrastructure::tools::agents::subagent_registry::new_registry());
        let (tx, rx) = tokio::sync::broadcast::channel(1024);
        Self {
            env,
            provider,
            runtime: composed.runtime,
            tx,
            rx,
            snippets: composed.snippets,
            reload: None,
            _home: composed._home,
        }
    }
    async fn send(&mut self, line: &str) {
        let mut ctx = self.env.ctx();
        ctx.broadcast_tx = Some(self.tx.clone());
        ctx.stdout = None;
        ctx.system_prompt = &self.snippets;
        if let Some(reload) = &self.reload {
            ctx.catalogue.reload = reload.clone();
        }
        dispatch_command(
            protocol::parse_command_line(line).expect("real command"),
            &mut ctx,
        )
        .await;
    }
    async fn prompt(&mut self, id: &str) {
        self.send(&json!({"type":"prompt","id":id,"message":"Carry on"}).to_string())
            .await;
    }
    async fn drain(&mut self) {
        let mut ctx = self.env.ctx();
        ctx.broadcast_tx = Some(self.tx.clone());
        ctx.stdout = None;
        ctx.system_prompt = &self.snippets;
        if let Some(reload) = &self.reload {
            ctx.catalogue.reload = reload.clone();
        }
        drain_pending_and_nudge(&mut ctx).await;
    }
    fn frames(&mut self) -> Vec<Value> {
        std::iter::from_fn(|| self.rx.try_recv().ok())
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect()
    }
    async fn policy(&mut self, enabled: bool, queued: bool) {
        self.send(&json!({"type":"set_tool_policy", "id":"policy", "operation":"replace", "unlistedScope": if enabled { "both" } else { "none" }, "mutations":[], "mode":if queued {"atNextTurnBoundary"} else {"immediateIfIdle"}, "persist":false}).to_string()).await;
    }
}

fn done() -> Vec<Reply> {
    vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Initial work complete"}),
        ),
        Reply::Text("Complete."),
    ]
}

fn workflow_config() -> Config {
    use crate::domain::workflow::{WorkflowTemplate, WorkflowTemplateStep};
    let mut config = Config::default();
    config.workflow.templates = vec![WorkflowTemplate {
        id: "run16".into(),
        label: "Run16".into(),
        description: "Finite lifecycle fixture".into(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "one".into(),
            label: "One".into(),
            phase: "test".into(),
            guidance: Some("advance once".into()),
        }],
        guards: vec![],
    }];
    config
}

async fn workflow_after_terminal(completed: bool, initially_stalled: bool) {
    let composed = compose_with_config(
        true,
        |flags| {
            flags.workflow_disabled = false;
            flags.workflow = true;
        },
        workflow_config(),
    );
    assert!(composed.workflow.is_some(), "real builder-bound engine");
    let mut replies = if initially_stalled {
        vec![Reply::Text("Still working."), Reply::Text("Still working.")]
    } else {
        done()
    };
    if !initially_stalled && std::env::var("QUECTO_TURN_OUTCOMES").ok().as_deref() == Some("1") {
        replies.extend(done());
    } else {
        replies.extend([
            Reply::Text("Received fresh workflow information."),
            Reply::Text("No declaration."),
        ]);
    }
    let mut rig = Rig::composed(composed, replies);
    rig.prompt("first").await;
    let initial = rig.frames();
    assert_eq!(rig.provider.requests().len(), 2);
    assert_eq!(
        of_type(&initial, "turn_outcome")[0]["outcome"],
        if initially_stalled { "stalled" } else { "done" }
    );
    {
        let mut engine = rig.env.workflow.lock().unwrap();
        engine.select_template("run16", None).unwrap();
        if completed {
            engine.check(1).unwrap();
        }
    }
    rig.drain().await;
    let requests = rig.provider.requests();
    let frames = rig.frames();
    if !initially_stalled && std::env::var("QUECTO_TURN_OUTCOMES").ok().as_deref() == Some("1") {
        assert_eq!(
            requests.len(),
            2,
            "opt-in settled root waits for next USER, not workflow wake"
        );
        assert!(of_type(&frames, "turn_admitted").is_empty());
        for _ in 0..3 {
            rig.drain().await;
        }
        assert_eq!(
            rig.provider.requests().len(),
            2,
            "idle workflow drains remain inert in opt-in settled mode"
        );
        rig.prompt("next-user").await;
        assert_eq!(
            rig.provider.requests().len(),
            4,
            "next USER opens fresh episode and declares done"
        );
        let next = rig.frames();
        assert_eq!(of_type(&next, "turn_admitted")[0]["episodeId"], "next-user");
        assert_eq!(of_type(&next, "turn_outcome")[0]["outcome"], "done");
        return;
    }
    assert_eq!(
        requests.len(),
        4,
        "actual engine emission must wake terminal root, then one correction"
    );
    assert!(
        requests[2].last_user.contains(if completed {
            "All workflow steps complete"
        } else {
            "Workflow incomplete"
        }),
        "{:?}",
        requests[2]
    );
    assert_eq!(requests[3].last_user, CORRECTION_NOTE);
    let admitted = of_type(&frames, "turn_admitted");
    assert_eq!(admitted.len(), 2, "fresh turn plus correction");
    assert_ne!(admitted[0]["episodeId"], "first");
    assert_ne!(
        admitted[0]["turnId"],
        of_type(&initial, "turn_admitted")[0]["turnId"]
    );
    assert_eq!(
        of_type(&frames, "turn_outcome")[0]["outcome"],
        "stalled",
        "old done declaration cannot satisfy new input"
    );
    for _ in 0..3 {
        rig.drain().await;
    }
    assert_eq!(
        rig.provider.requests().len(),
        4,
        "unchanged engine/repeated drains cannot replenish correction or mint input forever"
    );
    assert!(of_type(&rig.frames(), "turn_outcome").is_empty());
}

#[tokio::test]
async fn run16_m1_real_workflow_auto_continue_wakes_done_and_dedupes() {
    if isolated(concat!(
        module_path!(),
        "::run16_m1_real_workflow_auto_continue_wakes_done_and_dedupes"
    )) {
        workflow_after_terminal(false, false).await;
    }
}
#[tokio::test]
async fn run16_m1_real_workflow_completion_wakes_done_once() {
    if isolated(concat!(
        module_path!(),
        "::run16_m1_real_workflow_completion_wakes_done_once"
    )) {
        workflow_after_terminal(true, false).await;
    }
}
#[tokio::test]
async fn run16_m1_unchanged_workflow_does_not_reset_stalled_budget_forever() {
    if isolated(concat!(
        module_path!(),
        "::run16_m1_unchanged_workflow_does_not_reset_stalled_budget_forever"
    )) {
        workflow_after_terminal(false, true).await;
    }
}

#[tokio::test]
async fn run20_refuter_new_history_tool_success_does_not_inherit_emission() {
    if run21_default_isolated(concat!(
        module_path!(),
        "::run20_refuter_new_history_tool_success_does_not_inherit_emission"
    )) {
        let composed = compose_with_config(
            true,
            |flags| {
                flags.workflow_disabled = false;
                flags.workflow = true;
            },
            workflow_config(),
        );
        let mut replies = done();
        replies.extend([
            Reply::Text("First nudge."),
            Reply::Text("First correction."),
        ]);
        replies.push(Reply::Call(
            "workflow",
            json!({"action":"select_template", "template":"run16"}),
        ));
        replies.extend(done());
        replies.extend([
            Reply::Text("Second task nudge."),
            Reply::Text("Second correction."),
        ]);
        let mut rig = Rig::composed(composed, replies);
        rig.prompt("first-task").await;
        rig.env
            .workflow
            .lock()
            .unwrap()
            .select_template("run16", None)
            .unwrap();
        rig.drain().await;
        assert_eq!(
            rig.provider.requests().len(),
            4,
            "consume first incarnation emission"
        );
        let first = rig.frames();
        assert_eq!(
            of_type(&first, "turn_outcome").last().unwrap()["outcome"],
            "stalled"
        );
        let old_epoch = rig.env.session.parent_outcome().unwrap().view().epoch;
        rig.send(r#"{"type":"clear_history","id":"clear-for-new-task"}"#)
            .await;
        let clear = rig.frames();
        assert!(
            clear.iter().any(|frame| frame["type"] == "response"
                && frame["id"] == "clear-for-new-task"
                && frame["success"] == true),
            "actual successful history replacement: {clear:?}"
        );
        assert_ne!(
            rig.env.session.parent_outcome().unwrap().view().epoch,
            old_epoch
        );
        rig.prompt("second-task").await;
        assert_eq!(
            rig.provider.requests().len(),
            9,
            "real prompt auto-drains fresh workflow input"
        );
        assert!(
            rig.env.messages.iter().any(|message| {
                message.role == crate::domain::conversation::value_objects::message::Role::Tool
                    && message
                        .content
                        .contains("Selected workflow template 'run16'.")
            }),
            "second task must successfully execute actual workflow selection tool"
        );
        assert!(rig.env.workflow.lock().unwrap().active_template().is_some());
        let second = rig.frames();
        assert_eq!(of_type(&second, "turn_outcome")[0]["outcome"], "done");
        rig.drain().await;
        let count = rig.provider.requests().len();
        // Tear down all scratch before expected behavioral assertion.
        drop(rig);
        assert_eq!(
            count, 9,
            "new history/task must receive its own workflow emission even when progress matches the previous task"
        );
    }
}

fn run21_default_isolated(test: &str) -> bool {
    if std::env::var("RUN16_LIFECYCLE_CASE").ok().as_deref() == Some(test) {
        return true;
    }
    let (_, selected) = test.split_once("::").expect("qualified test");
    let mut failures = Vec::new();
    {
        let switch = "unset";
        let scratch = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "ulimit -c 0; exec timeout --signal=KILL 45s \"$@\"",
            "run16",
        ]);
        command.arg(std::env::current_exe().unwrap());
        command.args(["--exact", selected, "--nocapture", "--test-threads=1"]);
        command.env("RUN16_LIFECYCLE_CASE", test);
        command.env("RUN16_SCRATCH", scratch.path());
        command.env("TMPDIR", scratch.path());
        command.env("QUECTO_2708_TEST_ROOT", "default");
        command.env_remove("QUECTO_TURN_OUTCOMES");
        if switch == "1" {
            command.env("QUECTO_TURN_OUTCOMES", "1");
        }
        let output = command.output().expect("bounded exact lifecycle child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{switch}: {}\n{stdout}\n{stderr}", output.status);
        let selected_one = stdout.contains("running 1 test\n");
        let success = output.status.success();
        scratch
            .close()
            .expect("cleanup probe scratch after success/failure");
        assert!(selected_one, "nonzero exact selection: {selected}");
        match success {
            true => {}
            false => failures.push(format!("{switch}: {}", output.status)),
        }
    }
    assert!(
        failures.is_empty(),
        "bounded lifecycle children failed: {failures:?}"
    );
    false
}

#[tokio::test]
async fn run21_same_history_workflow_a_b_a_is_stale() {
    let mut session = crate::composition::bootstrap::agent_turn::build_turn_session("stub".into());
    assert!(session.take_workflow_emission("A".into()));
    assert!(session.take_workflow_emission("B".into()));
    assert!(
        !session.take_workflow_emission("A".into()),
        "same-history A→B→A retry must not mint a second fresh input"
    );
}

#[tokio::test]
async fn run21_settled_outcomes_workflow_wake_obeys_two_modes() {
    if !isolated(concat!(
        module_path!(),
        "::run21_settled_outcomes_workflow_wake_obeys_two_modes"
    )) {
        return;
    }
    for intent in ["done", "asking", "blocked"] {
        let composed = compose_with_config(
            true,
            |flags| {
                flags.workflow_disabled = false;
                flags.workflow = true;
            },
            workflow_config(),
        );
        let mut rig = Rig::composed(
            composed,
            vec![
                Reply::Call(
                    "declare_turn_outcome",
                    json!({"outcome":intent,"reason":"Settled fixture"}),
                ),
                Reply::Text("Settled"),
                Reply::Text("Fresh information"),
                Reply::Text("Correction"),
            ],
        );
        rig.prompt("settled-user").await;
        let initial = rig.frames();
        assert_eq!(
            of_type(&initial, "turn_outcome")[0]["outcome"],
            if intent == "done" {
                "done"
            } else {
                "needs_user"
            }
        );
        rig.env
            .workflow
            .lock()
            .unwrap()
            .select_template("run16", None)
            .unwrap();
        rig.drain().await;
        let expected = if std::env::var("QUECTO_TURN_OUTCOMES").ok().as_deref() == Some("1") {
            2
        } else {
            4
        };
        assert_eq!(
            rig.provider.requests().len(),
            expected,
            "settled {intent} wake mode"
        );
        for _ in 0..3 {
            rig.drain().await;
        }
        assert_eq!(
            rig.provider.requests().len(),
            expected,
            "unchanged drains never loop"
        );
    }
}

#[tokio::test]
async fn run21_failed_resume_preserves_consumed_workflow_and_epoch() {
    if !isolated(concat!(
        module_path!(),
        "::run21_failed_resume_preserves_consumed_workflow_and_epoch"
    )) {
        return;
    }
    let mut rig = Rig::new(vec![]);
    assert!(rig.env.session.take_workflow_emission("A".into()));
    let epoch = rig.runtime.as_ref().unwrap().view().epoch;
    rig.send(r#"{"type":"resume_session","id":"failed-reset","session":"missing-run21-session"}"#)
        .await;
    let frames = rig.frames();
    assert!(
        frames
            .iter()
            .any(|f| f["type"] == "response" && f["id"] == "failed-reset" && f["success"] == false),
        "real failed resume response: {frames:?}"
    );
    assert_eq!(
        rig.runtime.as_ref().unwrap().view().epoch,
        epoch,
        "failed reset preserves outcome epoch"
    );
    assert!(
        !rig.env.session.take_workflow_emission("A".into()),
        "failed reset preserves consumption fence"
    );
}
