//! What the dispatch loop needs from the subagent teardown capability
//! (#1935, #1938), as plain handles.
//!
//! The interface declares the runtime inputs one loop hands over and the
//! use-case and controller handles it holds back; composition owns the
//! concrete graph between them (`composition::subagent_teardown`) and hands
//! its builder in through [`crate::interface::cli::CliContext`] as a
//! [`crate::interface::cli::TeardownHandlesBuilder`], so no interface
//! module ever names the composition layer or assembles a graph.
use std::sync::Arc;

use tokio::sync::Notify;

use crate::application::subagents::use_cases::{
    HarnessShutdownTransaction, TerminateAllDelegatedAgents,
};
use crate::domain::ids::AgentUuid;
use crate::domain::parent_control::ParentControlBinding;
use crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle;
use crate::infrastructure::tools::subagent_registry::{NotificationTx, SubagentRegistry};
use crate::interface::uds::subagent_teardown::controller::SubagentTeardownController;

use super::uds_cancel::{CancelHandle, TurnControlHandle};
use super::uds_multi::BusyFlag;
use super::uds_parent_control::ConnectionTeardown;
use super::uds_teardown_adapters::{DeferredLoopPersistence, LoopExitReadiness};

/// The runtime inputs of one dispatch loop that the teardown handles are
/// composed over.
pub struct TeardownLoopInputs {
    /// This harness's own identity, the `owner` of its lineage.
    pub owner: AgentUuid,
    pub registry: Option<SubagentRegistry>,
    /// The lifecycle cell the spawn tool admits registrations against
    /// (#1938): the composed lifecycle repository freezes it. `None` builds
    /// a private one (a harness without a spawn tool).
    pub harness_lifecycle: Option<SharedHarnessLifecycle>,
    /// This loop's event stream, on which the fleet compensation and a
    /// selected termination's owner conclusion (#1936) broadcast the
    /// survivor roster.
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// Passive-note channel of the dispatch loop, for exits the fleet
    /// compensation joins.
    pub notify_tx: Option<NotificationTx>,
    pub cancel_handle: CancelHandle,
    pub turn_control: TurnControlHandle,
    pub busy: BusyFlag,
    /// The dispatch loop's exit notification (shared with the termination
    /// signal watcher), so every trigger converges on one exit path.
    pub exit_notify: Arc<Notify>,
    /// The binding this harness was launched with, or `unlaunched()`.
    pub binding: ParentControlBinding,
    /// The owner's exit announcement (#2070) each connection's reader task
    /// raises on the owning TUI's exit persist and withdraws on its close;
    /// the shutdown reads it.
    pub owner_exit: Arc<dyn crate::application::subagents::ports::OwnerExitAnnouncement>,
    /// The environment control slot (#2070), for the emptied `retained`
    /// environments an owner exit ends; `None` builds a teardown that ends
    /// none (unit rigs).
    pub environment_control:
        Option<crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot>,
}

/// The handles one dispatch loop holds: what the connection layer needs,
/// plus the shared pieces tests and the loop observe.
pub struct TeardownHandles {
    pub connections: Arc<ConnectionTeardown>,
    /// The one controller every trigger without a wire — termination
    /// signal, last client — drives the common shutdown through.
    pub controller: Arc<SubagentTeardownController>,
    /// The fleet teardown (#1938): delete-all and session transitions.
    pub fleet: Arc<TerminateAllDelegatedAgents>,
    pub transaction: Arc<HarnessShutdownTransaction>,
    pub exit: Arc<LoopExitReadiness>,
    pub persistence: Arc<DeferredLoopPersistence>,
}
