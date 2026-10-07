//! A one-shot run's orderly end settles the children it launched (#2206).
//!
//! `quecto agent -m` without `--uds` has no dispatch loop and no harness
//! shutdown: before #2206 it simply returned, its children ran their
//! parent-loss shutdown, and a container child's box was left `stopped`
//! for `quecto container gc`. Now, once the run has ended in its own code
//! the way its owner meant — the model finished, or `--max-time` stopped
//! it — the fleet teardown asks every direct child to shut down on the
//! **run end** authority: the parent's word for the children it launched
//! (a plain container child's environment is cleaned up and forgotten),
//! but no close of any swarm (a run its owner has not closed keeps its
//! box). A run that failed (a provider error), a crash, a signal or a
//! panic runs none of this: the children then see their parent lost and
//! every box is kept, as before.
//!
//! The interface declares the inputs and the builder type; composition
//! owns the graph (`composition::subagent_teardown::build_run_end_fleet`)
//! and hands the builder in through [`crate::interface::cli::CliContext`].
use std::sync::Arc;

use crate::application::subagents::dto::{
    FleetTeardownAuthority, FleetTeardownError, TerminateAllDelegatedAgentsRequest,
};
use crate::application::subagents::use_cases::TerminateAllDelegatedAgents;
use crate::domain::agents::services::subagent_teardown::ShutdownReason;
use crate::domain::ids::AgentUuid;
use crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// What the run-end fleet of one one-shot run is built over.
pub struct RunEndFleetInputs {
    /// The run's own identity, the `owner` of its lineage.
    pub owner: AgentUuid,
    /// The registry the run's spawn tool registered its children in.
    pub registry: SubagentRegistry,
    /// The lifecycle cell the spawn tool admits registrations against.
    pub harness_lifecycle: SharedHarnessLifecycle,
}

/// Composition's builder of the run-end fleet teardown (#2206).
pub type RunEndFleetBuilder = fn(RunEndFleetInputs) -> Arc<TerminateAllDelegatedAgents>;

/// How long a one-shot exit waits for its fleet to settle. A child that
/// acknowledges the shutdown settles in milliseconds; one that does not
/// is concluded within its own ~30 s ladder (5 s acknowledgement, 25 s for
/// the exit), and children settle concurrently: one ladder plus slack.
/// Past it the run stops waiting for the fleet, and a child still
/// unsettled falls back to its parent-loss shutdown (its box then kept,
/// for `quecto container kill` or `gc`). This is not the whole exit: a
/// container script already running on a blocking worker is never
/// orphaned — the process waits for it as its runtime shuts down, within
/// that script's own bound (a kill 20 s, a cleanup 120 s).
pub(crate) const RUN_END_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// How long the run end waits before it says what it is waiting for, and
/// how often it says so again.
pub(crate) const RUN_END_NOTICE: std::time::Duration = std::time::Duration::from_secs(5);

/// A one-shot run's composed handles: its retained context, and the fleet
/// its orderly end settles.
pub(crate) struct RunHandles<'a> {
    pub retention: &'a crate::interface::cli::retention_handles::RetentionHandles,
    pub run_end: RunEnd,
}

impl<'a> RunHandles<'a> {
    /// A run that settles no children at its end (no spawn tool composed).
    #[cfg(test)]
    pub(crate) fn without_children(
        retention: &'a crate::interface::cli::retention_handles::RetentionHandles,
    ) -> Self {
        Self {
            retention,
            run_end: RunEnd::none(),
        }
    }
}

/// The fleet a one-shot run settles when it ends in an orderly way.
pub(crate) struct RunEnd {
    fleet: Option<(Arc<TerminateAllDelegatedAgents>, SubagentRegistry)>,
    /// The run's output reaches the real terminal (`quecto agent -m` from
    /// `run`), so its answer is written out before the children settle
    /// rather than held behind them; a run whose output is captured
    /// (`run_with_output`) keeps its buffers for its caller.
    live_output: bool,
}

impl RunEnd {
    /// Nothing to settle: no builder composed, or the run has no spawn
    /// tool (no registry to hold a child).
    pub(crate) fn none() -> Self {
        Self {
            fleet: None,
            live_output: false,
        }
    }

    pub(crate) fn compose(
        builder: Option<RunEndFleetBuilder>,
        registry: Option<SubagentRegistry>,
        harness_lifecycle: Option<SharedHarnessLifecycle>,
        live_output: bool,
    ) -> Self {
        match (builder, registry, harness_lifecycle) {
            (Some(build), Some(registry), Some(harness_lifecycle)) => Self {
                fleet: Some((
                    build(RunEndFleetInputs {
                        owner: AgentUuid::new("harness"),
                        registry: registry.clone(),
                        harness_lifecycle,
                    }),
                    registry,
                )),
                live_output,
            },
            _ => Self {
                live_output,
                ..Self::none()
            },
        }
    }

    /// The run returned in its own code with `code`, its transcript saved:
    /// it finished, or its `--max-time` deadline — the operator's own bound
    /// on the run — stopped it. Those are the parent's word for the
    /// children it launched, so they settle on the run-end authority. A run
    /// that failed (a provider error) never comes here: that is closer to a
    /// crash than to the owner's word, so its children fall back to their
    /// parent-loss shutdown and every box is kept (#2206 round 1). The
    /// answer is written out first, so nothing waits on the children.
    pub(crate) fn after(
        self,
        rt: &tokio::runtime::Runtime,
        out: &mut super::agent::AgentOutput<'_>,
        code: i32,
    ) -> i32 {
        if self.live_output {
            use std::io::Write;
            print!("{}", std::mem::take(out.stdout));
            eprint!("{}", std::mem::take(out.stderr));
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
        }
        self.settle(rt);
        code
    }

    /// Settle every direct child on the run-end authority, saying which
    /// children it is waiting for once the wait passes [`RUN_END_NOTICE`].
    /// Called only from the run's own orderly return paths, never from a
    /// drop guard: an unwinding panic is a crash, not the parent's word.
    pub(crate) fn settle(self, rt: &tokio::runtime::Runtime) {
        let Some((fleet, registry)) = self.fleet else {
            return;
        };
        let live_output = self.live_output;
        let request = TerminateAllDelegatedAgentsRequest {
            reason: ShutdownReason::ParentShutdown,
            authority: FleetTeardownAuthority::RunEnd,
        };
        let settled = rt.block_on(async {
            let run = fleet.execute(request);
            tokio::pin!(run);
            let limit = tokio::time::sleep(RUN_END_LIMIT);
            tokio::pin!(limit);
            let start = tokio::time::Instant::now();
            let mut notices = tokio::time::interval_at(start + RUN_END_NOTICE, RUN_END_NOTICE);
            loop {
                tokio::select! {
                    outcome = &mut run => break Some(outcome),
                    () = &mut limit => break None,
                    _ = notices.tick() => {
                        notice_waiting(&registry, start.elapsed(), live_output);
                    }
                }
            }
        });
        match settled {
            Some(Ok(outcome)) if outcome.is_settled() => {}
            Some(Ok(outcome)) => tracing::warn!(
                unsettled = ?outcome.unsettled,
                "a child of the one-shot run did not settle at its end; `quecto container ls --all` lists any environment it left"
            ),
            Some(Err(FleetTeardownError::Interrupted)) => {
                tracing::warn!("the one-shot run's fleet teardown was interrupted")
            }
            None => tracing::warn!(
                "the one-shot run's children outlasted the run-end limit; their parent-loss shutdown ends them"
            ),
        }
    }
}

/// The children the run end is still waiting for: every row not yet
/// compensated out of the registry.
fn waiting_for(registry: &SubagentRegistry) -> Vec<String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let mut names: Vec<String> = entries
        .iter()
        .map(|(key, entry)| entry.effective_display_name(key).to_string())
        .collect();
    names.sort();
    names
}

fn notice_waiting(registry: &SubagentRegistry, elapsed: std::time::Duration, live_output: bool) {
    let waiting = waiting_for(registry);
    let line = format!(
        "quecto: still shutting down {} sub-agent(s) after {}s ({}); stops waiting for them after {}s, though a container cleanup already running is let finish (up to 120s)",
        waiting.len(),
        elapsed.as_secs(),
        waiting.join(", "),
        RUN_END_LIMIT.as_secs()
    );
    tracing::warn!("{line}");
    if live_output {
        eprintln!("{line}");
    }
}

#[cfg(test)]
#[path = "run_end_fleet_tests.rs"]
mod tests;
