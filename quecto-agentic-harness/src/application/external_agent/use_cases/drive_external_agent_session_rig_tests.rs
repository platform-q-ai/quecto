//! Fakes of the member session's ports (#2287), shared by its test
//! modules: a scripted process whose user turns get ids `u1`, `u2`, …, a
//! telemetry sink and a clock on tokio's (pausable) time.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use super::*;
use crate::application::external_agent::dto::{
    AgentClockInstant, CredentialEnv, ExternalAgentExit, ExternalAgentInputError,
    ExternalAgentLaunchError, ExternalAgentLaunchSpec, MessageRole, SessionPhase, SessionRecord,
    UserTurnId,
};
use crate::application::external_agent::ports::{
    DetachedWork, ExternalAgentProcess, ExternalAgentSpawner, PortFuture,
};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, InterruptReceipt, ResultEvent, SkippedLine,
    SkippedLineReason,
};

pub(super) const BOUND: Duration = Duration::from_secs(5);
pub(super) const GRACE: Duration = Duration::from_secs(60);
pub(super) const INTERRUPT: Duration = Duration::from_secs(30);

/// What the test sees of the fake process.
#[derive(Default)]
pub(super) struct Wire {
    sent: Mutex<Vec<String>>,
    events: Mutex<VecDeque<Option<ExternalAgentEvent>>>,
    ready: tokio::sync::Notify,
    pub(super) closed: AtomicBool,
    pub(super) dropped: AtomicBool,
    pub(super) interrupts: AtomicUsize,
    /// Interrupts fail to be written, as on a closed input.
    pub(super) refuse_interrupts: AtomicBool,
    /// Closes this session while an interrupt is being written, before
    /// the write returns `Ok`: `close` racing `abort` (#2287 review 2).
    pub(super) close_on_interrupt: Mutex<Option<Weak<DriveExternalAgentSession>>>,
    /// The process never exits, even once its input is closed: an
    /// agent wedged in a tool (#2304 review round 2).
    pub(super) hang_exit: AtomicBool,
    /// Waiting for the process to exit panics: an adapter's bug (#2304
    /// review round 3).
    pub(super) panic_exit: AtomicBool,
    /// Closing the input never returns: a pipe wedged on a full buffer.
    pub(super) hang_close_input: AtomicBool,
}

impl Wire {
    /// The user turns written, in order: `u1` is the first.
    pub(super) fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }

    pub(super) fn interrupts(&self) -> usize {
        self.interrupts.load(Ordering::SeqCst)
    }

    pub(super) fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }

    /// The agent emits `event`.
    pub(super) fn emit(&self, event: ExternalAgentEvent) {
        self.events.lock().unwrap().push_back(Some(event));
        self.ready.notify_one();
    }

    /// The agent's output ends.
    pub(super) fn end_output(&self) {
        self.events.lock().unwrap().push_back(None);
        self.ready.notify_one();
    }
}

struct FakeProcess(Arc<Wire>);

impl ExternalAgentProcess for FakeProcess {
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<UserTurnId, ExternalAgentInputError>> {
        Box::pin(async move {
            match self.0.closed.load(Ordering::SeqCst) {
                true => Err(ExternalAgentInputError::Closed),
                false => {
                    let mut sent = self.0.sent.lock().unwrap();
                    sent.push(text.to_string());
                    Ok(UserTurnId(format!("u{}", sent.len())))
                }
            }
        })
    }

    fn interrupt(&self) -> PortFuture<'_, Result<(), ExternalAgentInputError>> {
        Box::pin(async move {
            let refused = self.0.closed.load(Ordering::SeqCst)
                || self.0.refuse_interrupts.load(Ordering::SeqCst);
            match refused {
                true => Err(ExternalAgentInputError::Closed),
                false => {
                    self.0.interrupts.fetch_add(1, Ordering::SeqCst);
                    let racing = self.0.close_on_interrupt.lock().unwrap().clone();
                    if let Some(session) = racing.and_then(|racing| racing.upgrade()) {
                        session
                            .close()
                            .await
                            .expect("the racing close ends the member");
                    }
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
        Box::pin(async move {
            self.0.closed.store(true, Ordering::SeqCst);
            if self.0.hang_close_input.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
        })
    }

    fn exited(&self) -> PortFuture<'_, ExternalAgentExit> {
        Box::pin(async move {
            match self.0.hang_exit.load(Ordering::SeqCst) {
                true => std::future::pending().await,
                false => ExternalAgentExit::Code(0),
            }
        })
    }

    fn exited_discarding_output(&self) -> PortFuture<'_, ExternalAgentExit> {
        match self.0.panic_exit.load(Ordering::SeqCst) {
            true => Box::pin(async { panic!("the exit wait panics") }),
            false => self.exited(),
        }
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

pub(super) struct FakeLauncher {
    wire: Arc<Wire>,
    refuse: bool,
    pub(super) starts: Mutex<Vec<ExternalAgentLaunchSpec>>,
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
pub(super) struct Records(Mutex<Vec<SessionRecord>>);

impl ExternalAgentTelemetry for Records {
    fn record(&self, record: &SessionRecord) {
        self.0.lock().unwrap().push(record.clone());
    }

    fn finish(&self) -> PortFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// Runs detached work as a tokio task of the test's runtime.
struct TokioTasks;

impl ExternalAgentSpawner for TokioTasks {
    fn spawn(&self, work: DetachedWork) {
        drop(tokio::spawn(work));
    }
}

impl Records {
    pub(super) fn all(&self) -> Vec<SessionRecord> {
        self.0.lock().unwrap().clone()
    }

    pub(super) fn kinds(&self) -> Vec<&'static str> {
        self.all().iter().map(SessionRecord::kind).collect()
    }
}

/// tokio's time, in milliseconds since the clock was made: a paused test
/// runtime advances it only as its timers fire.
struct TokioTime(tokio::time::Instant);

impl ExternalAgentClock for TokioTime {
    fn now(&self) -> AgentClockInstant {
        AgentClockInstant(self.0.elapsed().as_millis() as u64)
    }

    fn sleep(&self, duration: Duration) -> PortFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

pub(super) struct Rig {
    pub(super) session: Arc<DriveExternalAgentSession>,
    pub(super) wire: Arc<Wire>,
    pub(super) records: Arc<Records>,
    pub(super) launcher: Arc<FakeLauncher>,
}

pub(super) fn settings() -> ExternalAgentSessionSettings {
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
        interrupt_grace: INTERRUPT,
    }
}

pub(super) fn rig_with(refuse: bool) -> Rig {
    rig_spawning(refuse, Arc::new(TokioTasks))
}

/// A rig whose detached work runs on `spawner`.
pub(super) fn rig_spawning(refuse: bool, spawner: Arc<dyn ExternalAgentSpawner>) -> Rig {
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
        Arc::new(TokioTime(tokio::time::Instant::now())),
        spawner,
        settings(),
    ));
    Rig {
        session,
        wire,
        records,
        launcher,
    }
}

pub(super) async fn started() -> Rig {
    let rig = rig_with(false);
    rig.session.start().await.expect("the member starts");
    rig
}

/// A started session whose agent is claude 2.1.280 (its `system/init`
/// says so, and that it withdraws queued user turns on an interrupt) and
/// has named the user turn its first result answered (`u1`, turn 1): it
/// takes steers. Its next user turn is `u2`.
pub(super) async fn named_started() -> Rig {
    let rig = started().await;
    rig.session.prompt("zero", None).await.unwrap();
    rig.feed(capable_init()).await;
    rig.feed(answered(&["u1"], "completed", "zero")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    rig
}

impl Rig {
    /// The next step, within [`BOUND`].
    pub(super) async fn step(&self) -> Option<SessionStep> {
        tokio::time::timeout(BOUND, self.session.next_step())
            .await
            .expect("a step is bounded")
    }

    /// The next step on a paused clock, where the session's own graces
    /// (minutes) must fire before the bound.
    pub(super) async fn paused_step(&self) -> Option<SessionStep> {
        tokio::time::timeout(Duration::from_secs(600), self.session.next_step())
            .await
            .expect("a step is bounded")
    }

    /// Once the ended member's end is recorded, within [`BOUND`]: its
    /// exit is recorded apart from the caller that ended it.
    pub(super) async fn end_recorded(&self) {
        tokio::time::timeout(BOUND, self.session.finish())
            .await
            .expect("the end is recorded within the bound");
    }

    /// Emit `event` and fold it.
    pub(super) async fn feed(&self, event: ExternalAgentEvent) -> SessionStep {
        self.wire.emit(event);
        self.step().await.expect("the session is live")
    }

    pub(super) fn phase(&self) -> SessionPhase {
        self.session.state().phase
    }

    /// No step comes within `quiet` (on a paused clock, at once).
    pub(super) async fn stays_quiet(&self, quiet: Duration) {
        let step = tokio::time::timeout(quiet, self.session.next_step()).await;
        assert!(step.is_err(), "no step within {quiet:?}: {step:?}");
    }
}

pub(super) fn result(
    is_error: bool,
    terminal_reason: &str,
    text: Option<&str>,
) -> ExternalAgentEvent {
    ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(is_error),
        terminal_reason: Some(terminal_reason.into()),
        result_text: text.map(str::to_string),
        duration_ms: Some(1200),
        ..ResultEvent::default()
    })
}

/// A completed turn's result that names no user turn (an older CLI).
pub(super) fn completed(text: &str) -> ExternalAgentEvent {
    result(false, "completed", Some(text))
}

/// A result naming the user turns `ids` it consumed, as claude 2.1.280
/// does (`user_message_uuids`).
pub(super) fn answered(ids: &[&str], terminal_reason: &str, text: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(terminal_reason != "completed"),
        terminal_reason: Some(terminal_reason.into()),
        result_text: Some(text.into()),
        duration_ms: Some(1200),
        user_turn_ids: ids.iter().map(|id| id.to_string()).collect(),
        ..ResultEvent::default()
    })
}

/// The answer to an interrupt that withdrew the queued user turns `ids`.
pub(super) fn interrupt_answered(ids: &[&str]) -> ExternalAgentEvent {
    ExternalAgentEvent::InterruptAnswered(InterruptReceipt {
        accepted: true,
        cancelled: ids.iter().map(|id| id.to_string()).collect(),
    })
}

pub(super) fn skipped(bytes: usize) -> ExternalAgentEvent {
    ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::OverCap,
        bytes,
    })
}

pub(super) fn text_block(id: &str, text: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: id.into(),
        block: AssistantContent::Text(text.into()),
    }
}

pub(super) fn user_messages(session: &DriveExternalAgentSession) -> Vec<String> {
    session
        .messages(0, 100)
        .into_iter()
        .filter(|m| m.role == MessageRole::User)
        .map(|m| m.content)
        .collect()
}

/// A `system/init` naming the CLI's `version` and `capabilities`.
pub(super) fn init_event(version: Option<&str>, capabilities: &[&str]) -> ExternalAgentEvent {
    ExternalAgentEvent::Init(InitEvent {
        cli_version: version.map(str::to_string),
        capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        ..InitEvent::default()
    })
}

/// claude 2.1.280's `system/init`, as captured: it names turns (by its
/// version) and withdraws queued user turns on an interrupt.
pub(super) fn capable_init() -> ExternalAgentEvent {
    init_event(
        Some("2.1.280"),
        &[
            "interrupt_receipt_v1",
            "interrupt_cancel_queued_v1",
            "msg_lifecycle_v1",
        ],
    )
}
