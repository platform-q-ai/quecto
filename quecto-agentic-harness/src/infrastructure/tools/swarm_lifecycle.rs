//! Linux process and UDS adapters for the swarm lifecycle use cases.
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::application::swarm::ports::CoordinationPort;
use crate::application::swarm::ports::PortFuture;
use crate::application::swarm::ports::{ProcessControl, ProcessObservation};
use crate::domain::error::DomainError;
use crate::domain::swarm::{Member, MemberStatus, ProcessIdentity, RunStatus};
use serde_json::{Value, json};

/// Join the container's coordination store; returns whether this container
/// hosts a created run, so the caller can tell an ordinary container from a
/// swarm (#1715). `participation` is this process's shared handle: it is set
/// from the join answer and kept current by the supervisor, so a member that
/// joined before the run was created still learns that it became a swarm
/// agent.
pub fn join_current_process(
    context: &SwarmContext,
    socket: Option<&std::path::Path>,
    participation: super::swarm_bridge::Participation,
    creator: bool,
) -> Result<bool, DomainError> {
    if !context.database().exists() && !creator {
        return Err(DomainError::Tool("swarm coordination store missing; only the container creator may initialize it; do not reset admission".into()));
    }
    prepare_checkout(context)?;
    let pid = std::process::id();
    let started = process_start(pid)
        .ok_or_else(|| DomainError::Tool("swarm requires Linux procfs process identity".into()))?;
    let snapshot = context.join(
        &ProcessIdentity { pid, started },
        socket.and_then(|s| s.to_str()),
        std::env::var("QUECTO_SWARM_RESERVATION").ok().as_deref(),
    )?;
    participation.record_creator(creator);
    let participates = crate::domain::swarm::participates(snapshot.deadline);
    participation.set(participates);
    if needs_supervision(snapshot.status) {
        supervise(context.clone(), snapshot, participation);
    }
    Ok(participates)
}

/// Whether this process is its container's creator: the one process the
/// container runtime starts with `QUECTO_SWARM_BOOTSTRAP=1` (every member it
/// adds later gets `0`).
pub fn is_creator() -> bool {
    std::env::var("QUECTO_SWARM_BOOTSTRAP").as_deref() == Ok("1")
}

/// Make the store's directory (#2145: the checkout's git directory where
/// it has one).
pub(super) fn prepare_checkout(context: &SwarmContext) -> Result<(), DomainError> {
    let database = context.database();
    let dir = database.parent().expect("the store has a directory");
    std::fs::create_dir_all(dir).map_err(|e| DomainError::Tool(format!("swarm storage: {e}")))
}

/// Every non-terminal run needs a watcher: a member joining while the run is
/// paused must still observe resume, deadline expiry, cancellation and later
/// pauses, or its local inference is never suspended nor its end settled.
pub(super) fn needs_supervision(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Setup | RunStatus::Running | RunStatus::Paused
    )
}

struct LinuxProcesses;
impl ProcessObservation for LinuxProcesses {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool {
        process_confirmed_dead(process.pid, &process.started)
    }
}

pub fn reconcile(context: &SwarmContext) -> Result<Value, DomainError> {
    context.lifecycle.reconcile(context, &LinuxProcesses)?;
    context.summary()
}

/// A settlement's closing reconcile: the harness's own, so its summary is
/// recorded as `host`, never against the member whose op settled the run
/// (#2279 S15 final review). `op=reconcile` keeps [`reconcile`], the
/// member's.
/// The coordinator's harness writes the run's summary here (#2313).
fn settled(context: &SwarmContext) -> Result<Value, DomainError> {
    let snapshot = context.lifecycle.reconcile(context, &LinuxProcesses)?;
    context.summarize_settled(&snapshot);
    context.host_summary()
}

/// The reaper of a member this harness launched observed its exit (#1961):
/// confirm the member dead (its tasks block for `recover`; an orderly exit
/// also releases its reservations) and reconcile.
pub fn member_exited(
    context: &SwarmContext,
    member: &str,
    exit: crate::domain::swarm::MemberExit,
) -> Result<Value, DomainError> {
    context
        .lifecycle
        .member_exited(context, &LinuxProcesses, member, exit)?;
    context.summary()
}

/// Durable messages remain authoritative when a wake hint fails.
pub async fn notify(context: &SwarmContext) -> Vec<String> {
    let ctx = context.clone();
    let Ok(Ok((members, generation))) =
        super::call_work::spawn_blocking_in_call(move || ctx.notification_batch()).await
    else {
        return vec!["swarm notification summary unavailable; inspect durable inbox".into()];
    };
    send_wake_hints(context, &members, generation).await
}

/// #1721: a resume is not an actionable board change, so the store's
/// notification fan-out never targets anyone for it. Wake every live member
/// with an endpoint (the resumer wakes itself) so a member suspended by a
/// provider failure learns the new control generation and re-arms.
pub async fn wake_after_resume(context: &SwarmContext, generation: u64) -> Vec<String> {
    let ctx = context.clone();
    let Ok(Ok(snapshot)) = super::call_work::spawn_blocking_in_call(move || ctx.snapshot()).await
    else {
        return vec![
            "swarm membership unavailable after resume; members re-arm on their next wake".into(),
        ];
    };
    send_wake_hints(context, &snapshot.members, generation).await
}

async fn send_wake_hints(
    context: &SwarmContext,
    members: &[Member],
    generation: u64,
) -> Vec<String> {
    let command = json!({"type":"swarm_control", "action":"wake", "generation":generation});
    send_hints(context, members, &command, "wake hint").await
}

/// #2390: this process changed the run's control state, so every other
/// live member with an endpoint is pushed a no-turn `watch`: its run watch
/// reads the board now rather than at its refresh. Delivered as wake hints
/// are; the warnings name the members it did not reach.
async fn announce_control_change(context: &SwarmContext, members: &[Member]) -> Vec<String> {
    let command = json!({"type":"swarm_control", "action":"watch"});
    send_hints(context, members, &command, "watch push").await
}

/// Sends `command` to every live member but this one that has an endpoint,
/// all at once, each bounded to 500 ms: best effort, as the durable board
/// is authoritative; a member that did not accept it is a warning.
async fn send_hints(
    context: &SwarmContext,
    members: &[Member],
    command: &Value,
    what: &str,
) -> Vec<String> {
    let line = command.to_string();
    let recipients: Vec<_> = members
        .iter()
        .filter(|member| pushed_to(member, &context.member))
        .filter_map(|member| Some((member, member.endpoint.as_deref()?)))
        .collect();
    let sends = recipients.iter().map(|&(member, socket)| {
        let line = &line;
        async move {
            let accepted = super::subagent_registry::send_subagent_uds_command_with_timeout(
                std::path::Path::new(socket),
                line,
                std::time::Duration::from_millis(500),
            )
            .await
            .is_ok_and(|response| {
                serde_json::from_str::<Value>(&response).is_ok_and(|value| value["success"] == true)
            });
            match accepted {
                true => None,
                false => Some(format!(
                    "{what} failed for {}; durable board is authoritative",
                    member.id
                )),
            }
        }
    });
    let started = std::time::Instant::now();
    let warnings: Vec<String> = futures::future::join_all(sends)
        .await
        .into_iter()
        .flatten()
        .collect();
    // The sends run at once, each bounded to 500 ms: one after another,
    // a swarm's members would hold the watch for seconds.
    debug_assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the hints are sent concurrently"
    );
    warnings
}

/// Whether a hint goes to `member`, seen from the harness of member `own`:
/// a live member that is another harness's (review L4: the recipients by
/// allowlist).
fn pushed_to(member: &Member, own: &str) -> bool {
    match (member.status, member.id == own) {
        (MemberStatus::Live, false) => true,
        (MemberStatus::Live, true) | (MemberStatus::Reserved | MemberStatus::Dead, _) => false,
    }
}

/// The process's local-inference suspension, as composition bound it.
type LocalSuspend = std::sync::Arc<dyn Fn(RunStatus, u64) + Send + Sync>;

/// This process's inference and its members' ends, and the suspension of
/// its local inference that composition bound, if any: nothing it does
/// needs the context since its execution jobs went (#2282).
struct RuntimeProcesses(Option<LocalSuspend>);
impl ProcessControl for RuntimeProcesses {
    fn suspend_local_inference(&self, snapshot: &crate::domain::swarm::Snapshot) {
        // The suspension verifies the run's control status on the board
        // before it acts, so it runs off the async workers (#2278 L6).
        if let Some(cancel) = &self.0 {
            let (cancel, status, generation) =
                (cancel.clone(), snapshot.status, snapshot.control_generation);
            run_off_the_workers(move || cancel(status, generation));
        }
    }
    fn abort<'a>(&'a self, member: &'a Member) -> PortFuture<'a, bool> {
        Box::pin(async move {
            let Some(socket) = &member.endpoint else {
                return false;
            };
            super::subagent_registry::send_subagent_uds_command_with_timeout(
                std::path::Path::new(socket),
                r#"{"type":"abort","ack":"accept"}"#,
                std::time::Duration::from_millis(500),
            )
            .await
            .is_ok()
        })
    }
    /// Delegated (#1939): the member's own harness ends it, through the
    /// delegated-agent graph bound by composition when this harness
    /// launched the member, over its registered endpoint otherwise. Never
    /// a pid.
    fn terminate<'a>(&'a self, member: &'a Member) -> PortFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            match MEMBER_TERMINATION.get() {
                Some(termination) => termination.terminate(member).await,
                None => {
                    super::swarm_member_termination::shutdown_member_over_endpoint(member).await
                }
            }
        })
    }
}

static MEMBER_TERMINATION: std::sync::OnceLock<
    std::sync::Arc<super::swarm_member_termination::DelegatedSwarmMemberTermination>,
> = std::sync::OnceLock::new();

/// The composition root supplies the delegated-agent termination of swarm
/// members this harness launched (#1939); without it a member is only
/// reachable over its endpoint.
pub fn bind_member_termination(
    termination: std::sync::Arc<super::swarm_member_termination::DelegatedSwarmMemberTermination>,
) {
    let _ = MEMBER_TERMINATION.set(termination);
}

pub async fn settle(context: SwarmContext) -> Result<Value, DomainError> {
    settle_suspending(context, local_suspend()).await
}

/// [`settle`], suspending this process's local inference through `suspend`.
async fn settle_suspending(
    context: SwarmContext,
    suspend: Option<LocalSuspend>,
) -> Result<Value, DomainError> {
    let ctx = context.clone();
    let snapshot = super::call_work::spawn_blocking_in_call(move || ctx.snapshot())
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))??;
    context
        .lifecycle
        .settle(
            &snapshot,
            &context.member,
            &RuntimeProcesses(suspend),
            &LinuxProcesses,
        )
        .await?;
    super::call_work::spawn_blocking_in_call(move || settled(&context))
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))?
}

/// Runs `job`, which makes board calls, off the async workers (#2278
/// review L6), and returns once it has finished (#2278 final review L1,
/// L2): the caller cannot await it (a drop, or a synchronous port method
/// on an async worker), yet what follows depends on it (a released slot,
/// a suspended turn). See [`super::call_work::block_here`].
pub(super) fn run_off_the_workers(job: impl FnOnce()) {
    super::call_work::block_here(job);
}

/// A per-process watcher also observes outcomes set by other members: it
/// suspends this process's inference on each pause, and settles the run's
/// terminal outcome for this member even if a remote turn-abort never
/// reaches it.
pub fn supervise(
    context: SwarmContext,
    snapshot: crate::domain::swarm::Snapshot,
    participation: super::swarm_bridge::Participation,
) {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    // The latch exists before the watch starts (review round 2): a local
    // change made from here on nudges it.
    let _latch = context.board.watch_nudges(&context.database());
    // The watcher's own thread, never an async worker: it blocks on the
    // board's latch between ticks and drives a runtime of its own only for
    // its settlement, so its board calls are marked as blocking work (#2278).
    std::thread::spawn(super::call_work::blocking(move || {
        let mut watched = watch::watch_until_terminal(
            &context,
            snapshot,
            &participation,
            &mut Supervisor(&context),
        );
        let snapshot = watched.snapshot.clone();
        // The polls the watch still holds are written before it settles.
        context.flush_watch_polls();
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => {
                if let Err(error) = runtime.block_on(context.lifecycle.settle(
                    &snapshot,
                    &context.member,
                    &RuntimeProcesses(local_suspend()),
                    &LinuxProcesses,
                )) {
                    tracing::error!(%error, "swarm terminal settlement failed");
                }
                context.summarize_settled(&snapshot);
                // Settled first (review round 2, L2), then a change this
                // process made whose nudge came late is pushed.
                watch::announce_late(&context, &mut watched, &mut Supervisor(&context));
                watch_until_ended(&context, &runtime);
            }
            Err(error) => {
                tracing::error!(%error, "swarm settlement runtime failed");
                watch::announce_late(&context, &mut watched, &mut Supervisor(&context));
            }
        }
    }));
}

/// The supervisor's side of the watch: it suspends this process's
/// inference on a pause, and tells the other members of a control change
/// this process made (#2390).
struct Supervisor<'a>(&'a SwarmContext);
impl watch::WatchObserver for Supervisor<'_> {
    fn paused(&mut self, snapshot: &crate::domain::swarm::Snapshot) -> bool {
        settle_observed_snapshot(self.0, snapshot)
    }
    fn announce(&mut self, snapshot: &crate::domain::swarm::Snapshot) {
        // The watch's own thread, outside any runtime: a runtime of its
        // own for the pushes, as for its settlement.
        let warnings = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(announce_control_change(self.0, &snapshot.members)),
            Err(error) => vec![format!("swarm watch push runtime failed: {error}")],
        };
        for warning in warnings {
            tracing::warn!(%warning, "swarm watch push not delivered");
        }
    }
}

/// How long a member waits after settling for its launcher to end it: the
/// launcher's teardown may take one conclusion bound per level of nesting
/// (a launcher ends its own launchees before it exits), with room for three.
const SELF_END_GRACE: std::time::Duration = std::time::Duration::from_secs(
    crate::infrastructure::processes::direct_child_routing::PER_HOP_CONCLUSION_BOUND.as_secs() * 3,
);
/// Attempts after which a member stops trying to end itself: it cannot reach
/// its own endpoint, or its harness accepted the shutdown but does not exit.
const SELF_END_ATTEMPTS: u32 = 5;
/// Consecutive unreadable snapshots (a contended store at close) tolerated
/// before the watch gives up.
const SNAPSHOT_ATTEMPTS: u32 = 10;

/// How long the settlement watch keeps trying before it gives up.
#[derive(Debug, Default)]
struct WatchBudget {
    unreadable: u32,
    self_end_attempts: u32,
}

impl WatchBudget {
    fn readable(&mut self) {
        self.unreadable = 0;
    }
    /// Records an unreadable snapshot; true once too many came in a row.
    fn unreadable_exhausted(&mut self) -> bool {
        self.unreadable += 1;
        self.unreadable >= SNAPSHOT_ATTEMPTS
    }
    /// Records an attempt to end itself; true once none is left.
    fn self_end_exhausted(&mut self) -> bool {
        self.self_end_attempts += 1;
        self.self_end_attempts >= SELF_END_ATTEMPTS
    }
}

/// After settling, a member waits to be ended by its launcher (#2121). Each
/// tick settles again on a fresh snapshot, so a coordinator lost meanwhile
/// hands the ending to the members; a member still alive after the grace
/// ends itself instead of keeping the environment alive. What to do each
/// tick is the application's decision (`settlement_step`).
fn watch_until_ended(context: &SwarmContext, runtime: &tokio::runtime::Runtime) {
    use crate::application::swarm::ports::SettlementStep;
    let started = std::time::Instant::now();
    let mut retry_logged = false;
    let mut budget = WatchBudget::default();
    loop {
        let snapshot = match context.snapshot() {
            Ok(snapshot) => {
                budget.readable();
                snapshot
            }
            Err(error) => {
                if budget.unreadable_exhausted() {
                    tracing::error!(%error, "swarm settlement watch lost coordination");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };
        let step = context.lifecycle.settlement_step(
            &snapshot,
            &context.member,
            started.elapsed(),
            SELF_END_GRACE,
        );
        let outcome = match step {
            SettlementStep::Done => return,
            SettlementStep::Wait => runtime.block_on(context.lifecycle.settle(
                &snapshot,
                &context.member,
                &RuntimeProcesses(local_suspend()),
                &LinuxProcesses,
            )),
            SettlementStep::EndSelf => runtime.block_on(context.lifecycle.settle_overdue(
                &snapshot,
                &context.member,
                &RuntimeProcesses(local_suspend()),
            )),
        };
        match (outcome, step) {
            (outcome, SettlementStep::EndSelf) => {
                // Every attempt counts, accepted or not: a harness that took
                // the shutdown but is still here is as stuck as one that
                // refused it.
                if budget.self_end_exhausted() {
                    let reason = outcome.err().map_or_else(
                        || "shutdown accepted but still running".into(),
                        |e| e.to_string(),
                    );
                    tracing::error!(%reason, "swarm member could not end itself; giving up");
                    return;
                }
            }
            (Ok(()), _) => {}
            (Err(error), _) => {
                if !retry_logged {
                    tracing::warn!(%error, "swarm settlement retry failed; still waiting");
                    retry_logged = true;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// The wall clock: Unix seconds, as Python's `time.time()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;
impl crate::application::swarm::ports::Clock for SystemClock {
    fn now_seconds(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

/// The model gates (#2339): a request's first send reads the admission as
/// `model`, each reattempt as `retry`.
impl crate::application::providers::ports::RequestAdmission for SwarmContext {
    fn check(
        &self,
        attempt: crate::domain::inference::value_objects::provider::RequestAttempt,
    ) -> PortFuture<'_, Result<(), DomainError>> {
        let context = self.clone();
        let actor = self.member.clone();
        let gate = crate::domain::swarm::AdmissionGate::for_attempt(attempt);
        Box::pin(async move {
            let snapshot =
                super::call_work::spawn_blocking_in_call(move || context.inference_snapshot(gate))
                    .await
                    .map_err(|error| DomainError::Tool(error.to_string()))??;
            if snapshot.admits_inference(&actor) {
                Ok(())
            } else {
                Err(DomainError::Tool(format!(
                    "swarm {:?}: model execution suspended; use supervisor controls",
                    snapshot.status
                )))
            }
        })
    }
}

static LOCAL_SUSPEND: std::sync::OnceLock<LocalSuspend> = std::sync::OnceLock::new();

/// The local-inference suspension composition bound, if it bound one.
fn local_suspend() -> Option<LocalSuspend> {
    LOCAL_SUSPEND.get().cloned()
}

/// The CLI composition root supplies active-turn cancellation, without coupling
/// process lifecycle adapters to the UDS cancellation representation.
pub fn bind_local_suspension(cancel: LocalSuspend) {
    let _ = LOCAL_SUSPEND.set(cancel);
}

impl crate::application::swarm::ports::SwarmRunControl for SwarmContext {
    fn apply(
        &self,
        action: crate::domain::swarm::RunControlAction,
    ) -> PortFuture<'_, Result<crate::domain::swarm::RunControlReceipt, DomainError>> {
        let context = self.clone();
        Box::pin(async move {
            let resuming = matches!(action, crate::domain::swarm::RunControlAction::Resume);
            let fan_out = context.clone();
            let mut receipt = super::call_work::spawn_blocking_in_call(move || {
                use crate::domain::swarm::RunControlAction;
                let mut wake_allowed = false;
                let value = match action {
                    RunControlAction::UsageBudget {
                        token_limit,
                        strict_unknown,
                    } => {
                        context.usage_budget(token_limit, strict_unknown)?;
                        context.control_status()?
                    }
                    RunControlAction::Wake { generation } => {
                        wake_allowed = context.accept_wake(generation)?;
                        context.control_status()?
                    }
                    RunControlAction::Pause { reason } => context.pause(&reason)?,
                    RunControlAction::Resume => context.resume_external()?,
                    RunControlAction::Close => context.close()?,
                    RunControlAction::ExtendDeadline { seconds } => {
                        context.extend_deadline(seconds)?
                    }
                    RunControlAction::Status => context.control_status()?,
                };
                SwarmContext::decode_control_receipt(value, wake_allowed)
            })
            .await
            .map_err(|error| DomainError::Tool(error.to_string()))??;
            if resuming {
                receipt.wake_warnings = wake_after_resume(&fan_out, receipt.generation).await;
            }
            Ok(receipt)
        })
    }
    fn nudge_watch(&self) {
        self.board
            .watch_nudges(&self.database())
            .nudge(crate::domain::swarm::watch::Nudge::Remote);
    }
    /// Two harness reads, off the async workers: `_status` (whose
    /// coordinator, which status, free workers) and `_run_totals` (tasks).
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        let context = self.clone();
        Box::pin(async move {
            super::call_work::spawn_blocking_in_call(move || context.coordinator_board_now())
                .await
                .map_err(|error| DomainError::Tool(error.to_string()))?
        })
    }
}

impl crate::application::swarm::ports::WorkerBoardRead for SwarmContext {
    /// Three harness reads, off the async workers (#2471).
    fn worker_board(
        &self,
    ) -> PortFuture<'_, Result<Option<crate::domain::swarm::worker_wake::WorkerBoard>, DomainError>>
    {
        let context = self.clone();
        Box::pin(async move {
            super::call_work::spawn_blocking_in_call(move || context.worker_board_now())
                .await
                .map_err(|error| DomainError::Tool(error.to_string()))?
        })
    }
}

/// The tool gate (#2339): each tool call reads the admission afresh, as
/// `tool`, because the reply that asked for it (and each earlier call)
/// took its own time, and the reply's usage may just have spent the budget.
impl crate::application::tools::ports::ToolExecutionAdmission for SwarmContext {
    fn check<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a str,
    ) -> PortFuture<'a, Result<(), DomainError>> {
        let context = self.clone();
        Box::pin(async move {
            let snapshot = super::call_work::spawn_blocking_in_call(move || {
                context.inference_snapshot(crate::domain::swarm::AdmissionGate::Tool)
            })
            .await
            .map_err(|error| DomainError::Tool(error.to_string()))??;
            use crate::domain::swarm::RunStatus;
            // A reporting coordinator (terminal run, or a run it ended and
            // that now waits for the supervisor, #1729) keeps native reads.
            let reporting_only = || {
                self.member == snapshot.coordinator
                    && name == "swarm"
                    && serde_json::from_str::<serde_json::Value>(arguments)
                        .ok()
                        .is_some_and(|input| {
                            matches!(input["op"].as_str(), Some("summary" | "events" | "usage"))
                        })
            };
            let admitted = match snapshot.status {
                RunStatus::Setup | RunStatus::Running => true,
                RunStatus::Paused => snapshot.ended() && reporting_only(),
                RunStatus::Succeeded
                | RunStatus::Blocked
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::BudgetExhausted => reporting_only(),
            };
            if admitted {
                Ok(())
            } else {
                Err(DomainError::Tool("swarm lifecycle prohibits this tool execution; terminal coordinator is report only".into()))
            }
        })
    }
}

fn settle_observed_snapshot(
    context: &SwarmContext,
    snapshot: &crate::domain::swarm::Snapshot,
) -> bool {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => {
            if let Err(error) = runtime.block_on(context.lifecycle.settle(
                snapshot,
                &context.member,
                &RuntimeProcesses(local_suspend()),
                &LinuxProcesses,
            )) {
                tracing::error!(%error, "swarm suspension failed");
            }
            true
        }
        Err(error) => {
            tracing::error!(%error, "swarm suspension runtime failed");
            false
        }
    }
}

#[path = "swarm_watch.rs"]
mod watch;

#[cfg(test)]
#[path = "swarm_pause_generation_tests.rs"]
mod pause_generation_tests;

#[cfg(test)]
#[path = "swarm_lifecycle_tests.rs"]
mod tests;
