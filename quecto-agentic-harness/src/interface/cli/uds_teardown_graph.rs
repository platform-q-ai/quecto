//! Inputs and outputs of the per-harness subagent teardown graph (#1935).
//!
//! The interface owns the shape of what it needs; composition owns the
//! concrete graph (`composition::subagent_teardown::build_teardown_graph`)
//! and hands the builder in through [`crate::interface::cli::CliContext`],
//! so no interface module ever names the composition layer.
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::application::subagents::use_cases::HarnessShutdownTransaction;
use crate::domain::ids::AgentUuid;
use crate::domain::parent_control::ParentControlBinding;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

use super::uds_cancel::{CancelHandle, TurnControlHandle};
use super::uds_multi::BusyFlag;
use super::uds_parent_control::ConnectionTeardown;
use super::uds_teardown_adapters::{DeferredLoopPersistence, LoopExitReadiness};

/// Default time a launched harness waits for its parent to bind before it
/// presumes the launcher gone (#1935 review: a parent that dies between
/// spawning and presenting must not leave an unbound orphan: a launch-bound
/// child ignores client churn, #1937).
pub const DEFAULT_BIND_DEADLINE: Duration = Duration::from_secs(30);

/// Test-only override of the bind deadline, in milliseconds. Read once at
/// startup by the child.
pub const BIND_DEADLINE_ENV: &str = "QUECTO_PARENT_BIND_DEADLINE_MS";

/// When an `Unbound` launched harness gives up waiting for its parent.
#[derive(Debug, Clone)]
pub enum BindDeadline {
    /// After this much wall-clock time (production).
    After(Duration),
    /// When this notification fires (tests drive the deadline explicitly
    /// instead of racing a timer).
    Triggered(Arc<Notify>),
}

/// How a launched harness was bound to its launcher.
#[derive(Debug, Clone)]
pub struct ParentControlLaunch {
    pub binding: ParentControlBinding,
    /// When the harness stops waiting for the presentation.
    pub bind_deadline: BindDeadline,
}

pub struct TeardownGraphInputs {
    /// This harness's own identity, the `owner` of its lineage.
    pub owner: AgentUuid,
    pub registry: Option<SubagentRegistry>,
    pub cancel_handle: CancelHandle,
    pub turn_control: TurnControlHandle,
    pub busy: BusyFlag,
    /// The dispatch loop's exit notification (shared with the termination
    /// signal watcher), so every trigger converges on one exit path.
    pub exit_notify: Arc<Notify>,
    /// The binding this harness was launched with, or `unlaunched()`.
    pub binding: ParentControlBinding,
}

/// The built graph: what the connection layer holds, plus the shared pieces
/// tests and the loop observe.
pub struct TeardownGraph {
    pub connections: Arc<ConnectionTeardown>,
    pub transaction: Arc<HarnessShutdownTransaction>,
    pub exit: Arc<LoopExitReadiness>,
    pub persistence: Arc<DeferredLoopPersistence>,
}

/// Composition's graph builder, injected through the CLI context.
pub type TeardownGraphBuilder = fn(TeardownGraphInputs) -> TeardownGraph;
