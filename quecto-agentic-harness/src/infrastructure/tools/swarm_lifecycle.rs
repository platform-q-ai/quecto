//! Linux process and UDS adapters for the swarm lifecycle use cases.
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::domain::error::DomainError;
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::{
    CoordinationPort, Member, MemberStatus, ProcessControl, ProcessIdentity, ProcessObservation,
    RunStatus,
};
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
) -> Result<bool, DomainError> {
    if !context.database().exists() && std::env::var("QUECTO_SWARM_BOOTSTRAP").as_deref() != Ok("1")
    {
        return Err(DomainError::Tool("swarm coordination store missing; only the container creator may initialize it; do not reset admission".into()));
    }
    std::fs::create_dir_all(context.checkout.join(".quecto"))
        .map_err(|e| DomainError::Tool(format!("swarm storage: {e}")))?;
    let pid = std::process::id();
    let started = process_start(pid)
        .ok_or_else(|| DomainError::Tool("swarm requires Linux procfs process identity".into()))?;
    let snapshot = context.join(
        &ProcessIdentity { pid, started },
        socket.and_then(|s| s.to_str()),
        std::env::var("QUECTO_SWARM_RESERVATION").ok().as_deref(),
    )?;
    let participates = crate::domain::swarm::participates(snapshot.deadline);
    participation.set(participates);
    if needs_supervision(snapshot.status) {
        supervise(context.clone(), snapshot, participation);
    }
    Ok(participates)
}

/// Every non-terminal run needs a watcher: a member joining while the run is
/// paused must still observe resume, deadline expiry, cancellation and later
/// pauses, or its local jobs and inference are never settled.
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

/// Durable messages remain authoritative when a wake hint fails.
pub async fn notify(context: &SwarmContext) -> Vec<String> {
    let ctx = context.clone();
    let Ok(Ok((members, generation))) =
        tokio::task::spawn_blocking(move || ctx.notification_batch()).await
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
    let Ok(Ok(snapshot)) = tokio::task::spawn_blocking(move || ctx.snapshot()).await else {
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
    let mut warnings = Vec::new();
    for member in members {
        let Some(socket) = &member.endpoint else {
            continue;
        };
        if member.id == context.member || member.status != MemberStatus::Live {
            continue;
        }
        let command = json!({"type":"swarm_control", "action":"wake", "generation":generation});
        let accepted = super::subagent_registry::send_subagent_uds_command_with_timeout(
            std::path::Path::new(socket),
            &command.to_string(),
            std::time::Duration::from_millis(500),
        )
        .await
        .is_ok_and(|response| {
            serde_json::from_str::<Value>(&response).is_ok_and(|value| value["success"] == true)
        });
        if !accepted {
            warnings.push(format!(
                "wake hint failed for {}; durable board is authoritative",
                member.id
            ));
        }
    }
    warnings
}

struct RuntimeProcesses<'a>(&'a SwarmContext);
impl ProcessControl for RuntimeProcesses<'_> {
    fn suspend_local_executions(&self, snapshot: &crate::domain::swarm::Snapshot) {
        super::swarm::suspend_context_jobs(self.0, snapshot.control_generation);
    }
    fn suspend_local_inference(&self, snapshot: &crate::domain::swarm::Snapshot) {
        if let Some(cancel) = LOCAL_SUSPEND.get() {
            cancel(snapshot.status, snapshot.control_generation);
        }
    }
    fn cancel_local_executions(&self) {
        super::swarm::cancel_context_jobs(self.0);
    }
    fn abort<'a>(&'a self, member: &'a Member) -> LaunchFuture<'a, bool> {
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
    fn terminate<'a>(
        &'a self,
        process: &'a ProcessIdentity,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            let process = process.clone();
            tokio::task::spawn_blocking(move || {
                if process_start(process.pid).as_deref() == Some(&process.started) {
                    super::swarm::terminate_member(process.pid);
                }
            })
            .await
            .map_err(|e| DomainError::Tool(e.to_string()))
        })
    }
}

pub async fn settle(context: SwarmContext) -> Result<Value, DomainError> {
    let ctx = context.clone();
    let snapshot = tokio::task::spawn_blocking(move || ctx.snapshot())
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))??;
    context
        .lifecycle
        .settle(&snapshot, &context.member, &RuntimeProcesses(&context))
        .await?;
    tokio::task::spawn_blocking(move || reconcile(&context))
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))?
}

/// One supervisor tick: refresh the snapshot and record swarm participation
/// from it (#1715), so a member that joined an ordinary container becomes a
/// swarm agent the moment the run is created by someone else.
pub(super) fn observe(
    context: &SwarmContext,
    snapshot: &mut crate::domain::swarm::Snapshot,
    participation: &super::swarm_bridge::Participation,
) {
    match context.snapshot() {
        Ok(current) => *snapshot = current,
        Err(error) => {
            super::swarm::cancel_context_jobs(context);
            tracing::error!(%error, "swarm supervisor lost coordination; retaining ownership");
        }
    }
    participation.set(crate::domain::swarm::participates(snapshot.deadline));
}

/// A per-process watcher also observes outcomes set by other members. This
/// cancels detached local jobs even if a remote turn-abort leaves them alive.
pub fn supervise(
    context: SwarmContext,
    mut snapshot: crate::domain::swarm::Snapshot,
    participation: super::swarm_bridge::Participation,
) {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let mut suspended = None;
        loop {
            observe(&context, &mut snapshot, &participation);
            if snapshot.status == RunStatus::Paused
                && suspended != Some(snapshot.control_generation)
            {
                if settle_observed_snapshot(&context, &snapshot) {
                    suspended = Some(snapshot.control_generation);
                }
            } else if snapshot.status == RunStatus::Running {
                suspended = None;
            }
            if context
                .lifecycle
                .observed_outcome(&snapshot, &SystemClock)
                .terminal()
            {
                snapshot.status = context.lifecycle.observed_outcome(&snapshot, &SystemClock);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => {
                if let Err(error) = runtime.block_on(context.lifecycle.settle(
                    &snapshot,
                    &context.member,
                    &RuntimeProcesses(&context),
                )) {
                    tracing::error!(%error, "swarm terminal settlement failed");
                }
            }
            Err(error) => tracing::error!(%error, "swarm settlement runtime failed"),
        }
    });
}

struct SystemClock;
impl crate::domain::swarm::Clock for SystemClock {
    fn now_seconds(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

impl crate::domain::provider::RequestAdmission for SwarmContext {
    fn check(&self) -> LaunchFuture<'_, Result<(), DomainError>> {
        let context = self.clone();
        let actor = self.member.clone();
        Box::pin(async move {
            let snapshot = tokio::task::spawn_blocking(move || context.inference_snapshot())
                .await
                .map_err(|error| DomainError::Tool(error.to_string()))??;
            if snapshot
                .status
                .admits_inference(actor == snapshot.coordinator)
            {
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

static LOCAL_SUSPEND: std::sync::OnceLock<std::sync::Arc<dyn Fn(RunStatus, u64) + Send + Sync>> =
    std::sync::OnceLock::new();

/// The CLI composition root supplies active-turn cancellation, without coupling
/// process lifecycle adapters to the UDS cancellation representation.
pub fn bind_local_suspension(cancel: std::sync::Arc<dyn Fn(RunStatus, u64) + Send + Sync>) {
    let _ = LOCAL_SUSPEND.set(cancel);
}

impl crate::domain::swarm::SwarmRunControl for SwarmContext {
    fn apply(
        &self,
        action: crate::domain::swarm::RunControlAction,
    ) -> LaunchFuture<'_, Result<crate::domain::swarm::RunControlReceipt, DomainError>> {
        let context = self.clone();
        Box::pin(async move {
            let resuming = matches!(action, crate::domain::swarm::RunControlAction::Resume);
            let fan_out = context.clone();
            let mut receipt = tokio::task::spawn_blocking(move || {
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
                    RunControlAction::Resume => context.resume()?,
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
}

impl crate::domain::tool::ToolExecutionAdmission for SwarmContext {
    fn check<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a str,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        let context = self.clone();
        Box::pin(async move {
            let snapshot = tokio::task::spawn_blocking(move || context.inference_snapshot())
                .await
                .map_err(|error| DomainError::Tool(error.to_string()))??;
            use crate::domain::swarm::RunStatus;
            let admitted = match snapshot.status {
                RunStatus::Setup | RunStatus::Running => true,
                RunStatus::Paused => false,
                RunStatus::Succeeded
                | RunStatus::Blocked
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::BudgetExhausted => {
                    self.member == snapshot.coordinator
                        && name == "swarm"
                        && serde_json::from_str::<serde_json::Value>(arguments)
                            .ok()
                            .is_some_and(|input| {
                                matches!(input["op"].as_str(), Some("summary" | "events" | "usage"))
                            })
                }
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
                &RuntimeProcesses(context),
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

#[cfg(test)]
#[path = "swarm_pause_generation_tests.rs"]
mod pause_generation_tests;

#[cfg(test)]
#[path = "swarm_lifecycle_tests.rs"]
mod tests;
