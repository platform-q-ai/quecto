use super::uds_multi::MultiClientArgs;
use super::uds_session_handles::SessionLoopInputs;
use super::uds_single_client::{SingleClientArgs, single_client_loop};
use crate::application::agent_loop::AgentLoopImpl;
pub(crate) use crate::domain::conversation_view::inject_system_prompt;
#[cfg(test)]
pub(crate) use crate::domain::conversation_view::remove_injected_system_prompt;
#[cfg(test)]
use crate::domain::message::Role;

#[cfg(test)]
#[path = "uds_lifecycle_cov2_tests.rs"]
mod cov2_tests;
#[cfg(test)]
#[path = "uds_lifecycle_cov_tests.rs"]
mod cov_tests;

pub(super) type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;

pub struct UdsLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    /// The run's retained-context handles (D9 #1978): the one store the
    /// agent's pruning writer and the active session's recovery backstop
    /// share, derived here so the two cannot diverge; `None` for unit rigs.
    pub retention: Option<super::retention_handles::RetentionHandles>,
    pub base_dir: &'a std::path::Path,
    pub workspace: &'a std::path::Path,
    /// The typed identity the loop opens (D10 #1979).
    pub identity: crate::domain::session_identity::SessionIdentity,
    pub model: String,
    pub ephemeral: bool,
    pub system_prompt: String,
    pub socket_path: std::path::PathBuf,
    /// `None` = multi-client mode. `Some` = single-client mode (tests).
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    /// Composition's sessions handles builder (#1970): the loop hands over
    /// its base directory (and any override) and holds the handles back.
    pub sessions: super::SessionHandlesBuilder,
    /// The run's catalogue handles (#1845, #1848), built once by the agent
    /// startup and shared with the spawn tool.
    pub catalogue: super::catalogue_handles::CatalogueHandles,
    pub ext_registry: Option<ExtRegistry>,
    /// How long this harness lives (#1937): decided once at startup.
    pub lifetime: crate::domain::harness_lifetime::HarnessLifetime,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// The lifecycle cell the spawn tool admits against (#1938); the
    /// teardown graph freezes it. `None` builds a private one.
    pub harness_lifecycle:
        Option<crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle>,
    /// Blocking durable-environment inspection, launched only after the
    /// listener is bound and its readiness announcement has been emitted.
    pub environment_reconciliation:
        Option<crate::application::environments::use_cases::ReconcileRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// The launch-bound parent control binding (#1935); `None` for a
    /// top-level harness.
    pub parent_control: Option<super::uds_parent_control::ParentControlLaunch>,
    /// Composition's teardown handles builder; `None` runs the loop without
    /// the teardown edge (unit rigs) and is refused for a launched child.
    pub teardown_graph: Option<super::TeardownHandlesBuilder>,
}
pub fn run_uds_loop(args: UdsLoopArgs<'_>) -> i32 {
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to create runtime: {e}");
            return 1;
        }
    };
    rt.block_on(uds_loop_async(args))
}
use super::uds_socket::{SocketGuard, bind_secure_socket};

async fn uds_loop_async(args: UdsLoopArgs<'_>) -> i32 {
    let UdsLoopArgs {
        agent,
        retention,
        base_dir,
        workspace,
        identity,
        model,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override,
        sessions,
        catalogue,
        ext_registry,
        lifetime,
        notification_rx,
        subagent_registry,
        harness_lifecycle,
        environment_reconciliation,
        workflow_state,
        workflow_config,
        broadcast_tx,
        parent_control,
        teardown_graph,
    } = args;
    let session_key = identity.runtime_key().to_string(); // presenters, owner uuid
    let sessions = sessions(SessionLoopInputs {
        base_dir: base_dir.to_path_buf(),
        identity,
        ephemeral,
        system_prompt: system_prompt.clone(),
        spill_store: retention.as_ref().map(|handles| handles.store.clone()),
        durable_prefix: agent.durable_prefix_latch(),
        workflow_state: workflow_state.clone(),
        subagent_registry: subagent_registry.clone(),
    });
    // Open the loop's session (#1863, D8 #1977): the transaction claims it
    // (a key owned by another live process is refused at open, #1460),
    // loads it and lets the watermark stand for what the store holds.
    let opened = match sessions.switch.resume.open_at_startup().await {
        Ok(opened) => opened,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let messages = opened.messages;
    if let (Some(ws), Some(persisted)) = (&workflow_state, opened.workflow_run) {
        if let Ok(mut engine) = ws.lock() {
            engine.restore_run(persisted);
        }
    }

    if let Some(std_stream) = socket_override {
        // Single-client path: backward-compatible with existing tests.
        single_client_loop(
            SingleClientArgs {
                agent,
                workspace,
                messages,
                model,
                session_key,
                system_prompt,
                ext_registry,
                subagent_registry,
                workflow_state,
            },
            std_stream,
            &sessions,
            &catalogue,
        )
        .await
    } else {
        // Multi-client path: bind, accept loop, broadcast events.
        let listener = match bind_secure_socket(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("failed to bind socket {}: {e}", socket_path.display());
                return 1;
            }
        };
        eprint!("{}", super::uds_wire::socket_announcement(&socket_path));
        let _guard = SocketGuard(socket_path);
        if let Some(reconciliation) = environment_reconciliation {
            tokio::task::spawn_blocking(move || {
                let report = reconciliation.execute();
                crate::interface::cli::container::report_environment_reconciliation(&report);
            });
        }
        super::uds_multi::multi_client_loop(
            MultiClientArgs {
                agent,
                workspace,
                messages,
                model,
                session_key,
                system_prompt,
                ext_registry,
                lifetime,
                notification_rx,
                subagent_registry,
                harness_lifecycle,
                workflow_state,
                workflow_config,
                broadcast_tx,
                parent_control,
                teardown_graph,
            },
            listener,
            &sessions,
            &catalogue,
        )
        .await
    }
}
