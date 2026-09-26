//! Deadline-bounded agent run (`--max-time`), split from `agent.rs`
//! (750-line cap).

use crate::application::agent_loop::AgentLoopImpl;
use crate::application::agent_turn::ports::AgentLoop;
use crate::domain::message::Message;

/// Outcome of a deadline-bounded agent run.
pub(crate) enum DeadlineResult {
    /// Agent completed (successfully or with error) within the deadline.
    Completed(Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>),
    /// The deadline expired before the agent finished.
    TimedOut,
}

/// Run the agent until it finishes or `timeout_secs` pass, whichever comes
/// first. At the deadline the run is dropped where it stands (#2168): an
/// in-flight model request is abandoned and a running tool is stopped (the
/// bash tool kills its command's process group when its call is dropped), so
/// nothing the run started goes on acting after it. The earlier thread-based
/// wait noticed the deadline but still joined the worker, so the run went on
/// to its natural end.
pub(crate) fn run_with_deadline(
    rt: &tokio::runtime::Runtime,
    agent: &mut AgentLoopImpl,
    messages: &mut Vec<Message>,
    timeout_secs: u64,
) -> DeadlineResult {
    let deadline = std::time::Duration::from_secs(timeout_secs);
    // The timer is created inside the runtime it runs on.
    match rt.block_on(async { tokio::time::timeout(deadline, agent.process(messages)).await }) {
        Ok(result) => DeadlineResult::Completed(result),
        Err(_) => DeadlineResult::TimedOut,
    }
}

/// How long settling a stopped run may take before the process exits.
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// After the deadline stopped the run: record its unfinished request and why
/// it stopped, and flush pending request accounting (#2172 review), within
/// [`SETTLE_LIMIT`] so settling can never hold the exit.
pub(crate) fn settle_stopped_run(
    rt: &tokio::runtime::Runtime,
    agent: &AgentLoopImpl,
    timeout_secs: u64,
) {
    let reason = format!("max-time {timeout_secs}s exceeded: the run was stopped");
    let settled = rt.block_on(async {
        tokio::time::timeout(SETTLE_LIMIT, agent.settle_stopped_run(&reason)).await
    });
    match settled {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "a stopped run's accounting remains pending"),
        Err(_) => tracing::warn!("settling a stopped run outlasted its limit"),
    }
}

#[cfg(test)]
#[path = "agent_deadline_tests.rs"]
mod tests;
