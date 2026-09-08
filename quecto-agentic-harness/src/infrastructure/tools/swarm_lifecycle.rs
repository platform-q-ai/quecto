//! Linux process and UDS adapters for the swarm lifecycle use cases.
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::domain::error::DomainError;
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::{
    CoordinationPort, Member, MemberStatus, ProcessControl, ProcessIdentity, ProcessObservation,
    RunStatus,
};
use serde_json::{Value, json};

pub fn join_current_process(
    context: &SwarmContext,
    socket: Option<&std::path::Path>,
) -> Result<(), DomainError> {
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
    if snapshot.status == RunStatus::Running {
        supervise(context.clone(), snapshot);
    }
    Ok(())
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
    let Ok(Ok(members)) = tokio::task::spawn_blocking(move || ctx.notifications()).await else {
        return vec!["swarm notification summary unavailable; inspect durable inbox".into()];
    };
    let mut warnings = Vec::new();
    for member in &members {
        let Some(socket) = &member.endpoint else {
            continue;
        };
        if member.id == context.member || member.status != MemberStatus::Live {
            continue;
        }
        let command = json!({"type":"prompt", "message":"Swarm work may be available. First call swarm with op=summary. If the run is running, inspect your durable inbox and ready tasks; acknowledge messages after reading. If it is terminal, do not run Python or attempt inbox acknowledgment: report the final summary and remain available for supervisor requests, including artifact export. This hint may have been queued before completion.", "streamingBehavior":"followUp", "ack":"accept"});
        if super::subagent_registry::send_subagent_uds_command_with_timeout(
            std::path::Path::new(socket),
            &command.to_string(),
            std::time::Duration::from_millis(500),
        )
        .await
        .is_err()
        {
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

/// A per-process watcher also observes outcomes set by other members. This
/// cancels detached local jobs even if a remote turn-abort leaves them alive.
pub fn supervise(context: SwarmContext, mut snapshot: crate::domain::swarm::Snapshot) {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            match context.snapshot() {
                Ok(current) => snapshot = current,
                Err(error) => {
                    super::swarm::cancel_context_jobs(&context);
                    tracing::error!(%error, "swarm supervisor lost coordination; retaining ownership");
                }
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
