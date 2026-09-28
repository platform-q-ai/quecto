//! The member session state machine (#2287), over fakes of the S2 ports.
//!
//! Busy semantics mirror quecto's own UDS agent (`protocol_commands.rs`,
//! `uds_prompt_admission.rs::handle_busy_prompt`): a prompt while a turn
//! runs is refused unless it names a `streamingBehavior`; `steer` is
//! delivered into the running turn, `follow_up` once the turn ends.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::application::external_agent::dto::{
    CredentialEnv, ExecutionState, ExternalAgentExit, ExternalAgentInputError,
    ExternalAgentLaunchError, ExternalAgentLaunchSpec, FOLLOW_UP_QUEUE_CAPACITY, MessageRole,
    SessionPhase, SessionRecord,
};
use crate::application::external_agent::ports::{ExternalAgentProcess, PortFuture};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, ResultEvent, SkippedLine, SkippedLineReason,
};

const BOUND: Duration = Duration::from_secs(5);
const GRACE: Duration = Duration::from_secs(60);

/// What the test sees of the fake process.
#[derive(Default)]
struct Wire {
    sent: Mutex<Vec<String>>,
    events: Mutex<VecDeque<Option<ExternalAgentEvent>>>,
    ready: tokio::sync::Notify,
    closed: AtomicBool,
    dropped: AtomicBool,
}

impl Wire {
    fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }

    /// The agent emits `event`.
    fn emit(&self, event: ExternalAgentEvent) {
        self.events.lock().unwrap().push_back(Some(event));
        self.ready.notify_one();
    }

    /// The agent's output ends.
    fn end_output(&self) {
        self.events.lock().unwrap().push_back(None);
        self.ready.notify_one();
    }
}

struct FakeProcess(Arc<Wire>);

impl ExternalAgentProcess for FakeProcess {
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<(), ExternalAgentInputError>> {
        Box::pin(async move {
            match self.0.closed.load(Ordering::SeqCst) {
                true => Err(ExternalAgentInputError::Closed),
                false => {
                    self.0.sent.lock().unwrap().push(text.to_string());
                    Ok(())
                }
            }
        })
    }

    fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>> {
        Box::pin(async move {
            loop {
                let notified = self.0.ready.notified();
                if let Some(event) = self.0.events.lock().unwrap().pop_front() {
                    return event;
                }
                notified.await;
            }
        })
    }

    fn close_input(&self) -> PortFuture<'_, ()> {
        Box::pin(async move { self.0.closed.store(true, Ordering::SeqCst) })
    }

    fn exited(&self) -> PortFuture<'_, ExternalAgentExit> {
        Box::pin(async { ExternalAgentExit::Code(0) })
    }

    fn exited_discarding_output(&self) -> PortFuture<'_, ExternalAgentExit> {
        self.exited()
    }

    fn stderr_tail(&self) -> String {
        String::new()
    }
}

impl Drop for FakeProcess {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

struct FakeLauncher {
    wire: Arc<Wire>,
    refuse: bool,
    starts: Mutex<Vec<ExternalAgentLaunchSpec>>,
}

impl ExternalAgentLauncher for FakeLauncher {
    fn start<'a>(
        &'a self,
        spec: ExternalAgentLaunchSpec,
    ) -> PortFuture<'a, Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError>> {
        Box::pin(async move {
            self.starts.lock().unwrap().push(spec);
            match self.refuse {
                true => Err(ExternalAgentLaunchError::NotFound {
                    program: "claude".into(),
                    required_for: "claude-code members".into(),
                }),
                false => {
                    Ok(Box::new(FakeProcess(self.wire.clone())) as Box<dyn ExternalAgentProcess>)
                }
            }
        })
    }
}

#[derive(Default)]
struct Records(Mutex<Vec<SessionRecord>>);

impl ExternalAgentTelemetry for Records {
    fn record(&self, record: &SessionRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

impl Records {
    fn all(&self) -> Vec<SessionRecord> {
        self.0.lock().unwrap().clone()
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.all().iter().map(SessionRecord::kind).collect()
    }
}

struct Rig {
    session: Arc<DriveExternalAgentSession>,
    wire: Arc<Wire>,
    records: Arc<Records>,
    launcher: Arc<FakeLauncher>,
}

fn settings() -> ExternalAgentSessionSettings {
    ExternalAgentSessionSettings {
        launch: ExternalAgentLaunchSpec {
            model: "claude-sonnet".into(),
            tools: Vec::new(),
            mcp_config: serde_json::json!({}),
            settings: serde_json::json!({}),
            max_budget_usd: 1.0,
            checkout: "/work".into(),
            member_dir: "/members/w1".into(),
            credential: CredentialEnv {
                name: "ANTHROPIC_API_KEY".into(),
                value: "sk-ant-api03-SECRETSECRETSECRET".into(),
            },
        },
        skipped_line_grace: GRACE,
    }
}

fn rig_with(refuse: bool) -> Rig {
    let wire = Arc::new(Wire::default());
    let launcher = Arc::new(FakeLauncher {
        wire: wire.clone(),
        refuse,
        starts: Mutex::default(),
    });
    let records = Arc::new(Records::default());
    let session = Arc::new(DriveExternalAgentSession::new(
        launcher.clone(),
        records.clone(),
        settings(),
    ));
    Rig {
        session,
        wire,
        records,
        launcher,
    }
}

async fn started() -> Rig {
    let rig = rig_with(false);
    rig.session.start().await.expect("the member starts");
    rig
}

impl Rig {
    /// The next step, within [`BOUND`].
    async fn step(&self) -> Option<SessionStep> {
        tokio::time::timeout(BOUND, self.session.next_step())
            .await
            .expect("a step is bounded")
    }

    /// Emit `event` and fold it.
    async fn feed(&self, event: ExternalAgentEvent) -> SessionStep {
        self.wire.emit(event);
        self.step().await.expect("the session is live")
    }

    fn phase(&self) -> SessionPhase {
        self.session.state().phase
    }
}

fn result(is_error: bool, terminal_reason: &str, text: Option<&str>) -> ExternalAgentEvent {
    ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(is_error),
        terminal_reason: Some(terminal_reason.into()),
        result_text: text.map(str::to_string),
        duration_ms: Some(1200),
        ..ResultEvent::default()
    })
}

fn completed(text: &str) -> ExternalAgentEvent {
    result(false, "completed", Some(text))
}

fn text_block(id: &str, text: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: id.into(),
        block: AssistantContent::Text(text.into()),
    }
}

fn user_messages(session: &DriveExternalAgentSession) -> Vec<String> {
    session
        .messages(0, 100)
        .into_iter()
        .filter(|m| m.role == MessageRole::User)
        .map(|m| m.content)
        .collect()
}

// quecto: `prompt` while running without `streamingBehavior` is rejected
// with "agent is running; provide streamingBehavior" (handle_busy_prompt).
#[tokio::test]
async fn a_prompt_while_busy_is_refused_without_steer_or_follow_up() {
    let rig = started().await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(
        rig.session.prompt("one", None).await,
        Ok(PromptAccepted::Started { turn: 1 })
    );
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    let refusal = rig.session.prompt("two", None).await.unwrap_err();
    assert_eq!(refusal, SessionRefusal::Busy);
    assert_eq!(
        refusal.to_string(),
        "agent is running; provide streamingBehavior"
    );
    assert_eq!(
        rig.wire.sent(),
        ["one"],
        "a refused prompt is never written"
    );
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
}

// quecto: `steer` interrupts after the current tool and is delivered next;
// a claude process reads stdin mid-turn and folds the text into the
// running turn (spike #2264), so it is written at once and the turn's one
// `result` ends both.
#[tokio::test]
async fn steer_writes_immediately_and_folds_into_the_running_turn() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(text_block("m1", "working")).await;
    assert_eq!(
        rig.session.steer("two").await,
        Ok(PromptAccepted::Steered { turn: 1 })
    );
    assert_eq!(rig.wire.sent(), ["one", "two"], "written at once");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    let SessionStep::Folded(step) = rig.feed(completed("done")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some(), "one result ends the steered turn");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().totals.turns, 1);
    assert_eq!(user_messages(&rig.session), ["one", "two"]);
    assert_eq!(rig.session.report().unwrap().content, "done");
}

// quecto: `follow_up` is delivered when the agent finishes.
#[tokio::test]
async fn follow_up_waits_for_turn_end() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    assert_eq!(
        rig.session.follow_up("two").await,
        Ok(PromptAccepted::Queued { position: 1 })
    );
    assert_eq!(
        rig.session
            .prompt("three", Some(StreamingBehavior::FollowUp))
            .await,
        Ok(PromptAccepted::Queued { position: 2 })
    );
    assert_eq!(rig.wire.sent(), ["one"], "follow-ups wait");
    assert_eq!(rig.session.state().queued_follow_ups, 2);
    rig.feed(text_block("m1", "still working")).await;
    assert_eq!(rig.wire.sent(), ["one"], "still waiting mid-turn");

    rig.feed(completed("first")).await;
    assert_eq!(rig.wire.sent(), ["one", "two"], "sent at the turn's end");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert_eq!(rig.session.state().queued_follow_ups, 1);

    rig.feed(completed("second")).await;
    assert_eq!(rig.wire.sent(), ["one", "two", "three"]);
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 });
    rig.feed(completed("third")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().totals.turns, 3);
}

#[tokio::test]
async fn queue_overflow_is_refused() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    for position in 1..=FOLLOW_UP_QUEUE_CAPACITY {
        assert_eq!(
            rig.session.follow_up(&format!("f{position}")).await,
            Ok(PromptAccepted::Queued { position })
        );
    }
    let refusal = rig.session.follow_up("one too many").await.unwrap_err();
    assert_eq!(refusal, SessionRefusal::QueueFull);
    assert_eq!(
        refusal.to_string(),
        "the follow-up queue is full (16); wait for the running turn to end"
    );
    assert_eq!(
        rig.session.state().queued_follow_ups,
        FOLLOW_UP_QUEUE_CAPACITY
    );
    // A steer is written at once, so a full queue does not refuse it.
    assert_eq!(
        rig.session.steer("steer").await,
        Ok(PromptAccepted::Steered { turn: 1 })
    );
}

#[tokio::test]
async fn a_failed_turn_returns_to_idle_and_reports_the_failure() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let SessionStep::Folded(step) = rig.feed(result(true, "api_error", None)).await else {
        panic!("the result is folded")
    };
    let outcome = step.turn_end.expect("the failed result ends the turn");
    assert!(!outcome.end.is_completed());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().execution, ExecutionState::Idle);
    let report = rig.session.report().expect("a failed turn reports");
    assert!(report.failure.is_some(), "{report:?}");
    assert_eq!(
        report.content,
        "the turn failed (terminal_reason: api_error)"
    );
    // The member takes the next prompt.
    assert_eq!(
        rig.session.prompt("again", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
    );
}

// Owner decision (#2287 risks): claude has no abort message on stdin, so
// `abort` ends the member: its turn, its follow-ups and its process.
#[tokio::test]
async fn abort_ends_the_turn() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    // A reader is waiting on the stream when the abort lands.
    let reader = tokio::spawn({
        let session = rig.session.clone();
        async move { session.next_step().await }
    });
    tokio::task::yield_now().await;
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 1,
        })
    );
    let step = tokio::time::timeout(BOUND, reader)
        .await
        .expect("the waiting reader is released")
        .unwrap();
    assert_eq!(step, None, "the aborted member has no more steps");
    assert!(
        rig.wire.dropped.load(Ordering::SeqCst),
        "the agent process is ended"
    );
    assert_eq!(rig.wire.sent(), ["one"], "the follow-up is dropped");
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert_eq!(rig.session.state().queued_follow_ups, 0);
    assert_eq!(
        rig.session.prompt("three", None).await,
        Err(SessionRefusal::Ended)
    );
    assert_eq!(rig.session.abort().await, Err(SessionRefusal::Ended));
    assert_eq!(rig.session.next_step().await, None);
}

#[tokio::test]
async fn an_idle_abort_ends_the_member_without_a_turn() {
    let rig = started().await;
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: None,
            dropped_follow_ups: 0,
        })
    );
    assert!(rig.wire.dropped.load(Ordering::SeqCst));
    assert_eq!(rig.phase(), SessionPhase::Ended);
}

// S2 review: a skipped line may have been the turn's `result`. The turn is
// given up once the stream then stays quiet for the grace period; the
// next follow-up starts.
#[tokio::test(start_paused = true)]
async fn a_skipped_line_then_silence_ends_the_turn_as_lost() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    let skipped = ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::OverCap,
        bytes: 17 * 1024 * 1024,
    });
    assert!(matches!(rig.feed(skipped).await, SessionStep::Folded(_)));
    let started = tokio::time::Instant::now();
    let step = rig.session.next_step().await;
    assert_eq!(step, Some(SessionStep::TurnLost { turn: 1 }));
    assert!(started.elapsed() >= GRACE, "only after the grace period");
    assert_eq!(rig.wire.sent(), ["one", "two"], "the follow-up starts");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
}

#[tokio::test(start_paused = true)]
async fn a_skipped_line_followed_by_the_result_ends_the_turn_as_usual() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::NotUtf8,
        bytes: 40,
    }))
    .await;
    let SessionStep::Folded(step) = rig.feed(completed("done")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    // Idle, the stream may stay quiet for ever: no turn is lost.
    let quiet = tokio::time::timeout(GRACE * 3, rig.session.next_step()).await;
    assert!(quiet.is_err(), "an idle member waits: {quiet:?}");
}

#[tokio::test(start_paused = true)]
async fn silence_without_a_skipped_line_never_ends_a_turn() {
    // A long tool call is quiet: only a skipped line arms the grace.
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let quiet = tokio::time::timeout(GRACE * 3, rig.session.next_step()).await;
    assert!(quiet.is_err(), "{quiet:?}");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
}

#[tokio::test]
async fn the_agent_exiting_mid_turn_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.wire.end_output();
    assert_eq!(
        rig.step().await,
        Some(SessionStep::Ended {
            turn: Some(1),
            exit: ExternalAgentExit::Code(0),
        })
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert_eq!(
        rig.wire.sent(),
        ["one"],
        "nothing is written to an exited agent"
    );
    assert_eq!(rig.session.next_step().await, None);
    assert_eq!(
        rig.session.prompt("three", None).await,
        Err(SessionRefusal::Ended)
    );
}

#[tokio::test]
async fn a_session_starts_its_agent_once_with_its_settings() {
    let rig = rig_with(false);
    assert_eq!(rig.phase(), SessionPhase::NotStarted);
    assert_eq!(
        rig.session.prompt("early", None).await,
        Err(SessionRefusal::NotStarted)
    );
    assert_eq!(rig.session.next_step().await, None);
    rig.session.start().await.unwrap();
    assert_eq!(
        rig.session.start().await,
        Err(SessionRefusal::AlreadyStarted)
    );
    assert_eq!(*rig.launcher.starts.lock().unwrap(), [settings().launch]);
    assert_eq!(rig.phase(), SessionPhase::Idle);

    let refused = rig_with(true);
    let error = refused.session.start().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "claude not found on PATH (required for claude-code members)"
    );
    assert_eq!(refused.phase(), SessionPhase::Ended);
}

#[tokio::test]
async fn turn_ordinals_strictly_increase() {
    let rig = started().await;
    let mut last = 0;
    for round in 0..3 {
        let Ok(PromptAccepted::Started { turn }) = rig.session.prompt("go", None).await else {
            panic!("round {round} starts a turn")
        };
        assert!(turn > last, "{turn} after {last}");
        last = turn;
        rig.feed(completed("ok")).await;
    }
}

// Telemetry (#2284): every op records ids, kinds and sizes, and never the
// text it carried.
#[tokio::test]
async fn every_decision_and_effect_is_recorded_without_its_text() {
    let secret = "sk-ant-api03-PROMPTSECRETPROMPTSECRET";
    let rig = started().await;
    rig.session.prompt(secret, None).await.unwrap();
    rig.session.prompt(secret, None).await.unwrap_err();
    rig.session.follow_up(secret).await.unwrap();
    rig.feed(ExternalAgentEvent::AssistantBlock {
        message_id: "m1".into(),
        block: AssistantContent::ToolUse {
            id: "t1".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": secret}),
        },
    })
    .await;
    rig.feed(ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::NotUtf8,
        bytes: 9,
    }))
    .await;
    rig.feed(completed(secret)).await;
    rig.session.abort().await.unwrap();
    let bytes = secret.len();
    assert_eq!(
        rig.records.all(),
        [
            SessionRecord::Started,
            SessionRecord::PromptAccepted {
                accepted: PromptAccepted::Started { turn: 1 },
                bytes,
            },
            SessionRecord::PromptRefused {
                refusal: "busy",
                bytes,
            },
            SessionRecord::PromptAccepted {
                accepted: PromptAccepted::Queued { position: 1 },
                bytes,
            },
            SessionRecord::ToolCalled {
                turn: Some(1),
                tool: "Bash".into(),
            },
            SessionRecord::LineSkipped {
                turn: Some(1),
                bytes: 9,
            },
            SessionRecord::TurnEnded {
                turn: 1,
                outcome: "completed",
                duration_ms: Some(1200),
                cost_micro_usd: 0,
            },
            SessionRecord::FollowUpStarted { turn: 2, bytes },
            SessionRecord::Aborted {
                turn: Some(2),
                dropped_follow_ups: 0,
            },
        ]
    );
    let logged = format!("{:?}", rig.records.all());
    assert!(!logged.contains("SECRET"), "{logged}");
}

#[tokio::test]
async fn a_refused_start_and_an_exit_are_recorded() {
    let refused = rig_with(true);
    refused.session.start().await.unwrap_err();
    assert_eq!(
        refused.records.all(),
        [SessionRecord::StartRefused { kind: "not_found" }]
    );

    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.end_output();
    rig.step().await;
    assert_eq!(
        rig.records.kinds(),
        ["started", "prompt_accepted", "turn_ended", "ended"]
    );
    assert!(rig.records.all().contains(&SessionRecord::TurnEnded {
        turn: 1,
        outcome: "exited",
        duration_ms: None,
        cost_micro_usd: 0,
    }));
}
