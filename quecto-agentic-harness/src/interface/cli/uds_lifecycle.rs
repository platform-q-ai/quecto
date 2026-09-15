use super::uds::{DispatchCtx, run_command_loop};
use super::uds_cancel::{CancelSlot, TurnControl};
use super::uds_multi::MultiClientArgs;
#[path = "uds/uds_session_load.rs"]
mod uds_session_load;
use super::uds_session::AgentSession;
use super::uds_session_handles::{SessionHandles, SessionLoopInputs};
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::sessions::dto::SaveTrigger;
use crate::application::sessions::ports::SessionStore;
pub(crate) use crate::domain::conversation_view::inject_system_prompt;
#[cfg(test)]
pub(crate) use crate::domain::conversation_view::remove_injected_system_prompt;
use crate::domain::message::Message;
#[cfg(test)]
use crate::domain::message::Role;
use crate::domain::session_identity::SessionIdentity;
use uds_session_load::load_session;

#[cfg(test)]
#[path = "uds_lifecycle_cov2_tests.rs"]
mod cov2_tests;
#[cfg(test)]
#[path = "uds_lifecycle_cov_tests.rs"]
mod cov_tests;

type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;

pub struct UdsLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub workspace: &'a std::path::Path,
    pub session_key: String,
    pub model: String,
    pub ephemeral: bool,
    pub system_prompt: String,
    pub socket_path: std::path::PathBuf,
    /// `None` = multi-client mode. `Some` = single-client mode (tests).
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    /// A store the loop is handed instead of the composed file store
    /// (tests); threaded into the sessions builder's inputs.
    pub session_store_override: Option<std::sync::Arc<dyn SessionStore>>,
    /// Composition's sessions handles builder (#1970): the loop hands over
    /// its base directory (and any override) and holds the handles back.
    pub sessions: super::SessionHandlesBuilder,
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
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
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
        base_dir,
        workspace,
        session_key,
        model,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override,
        session_store_override,
        sessions,
        ext_registry,
        lifetime,
        notification_rx,
        subagent_registry,
        harness_lifecycle,
        workflow_state,
        workflow_config,
        broadcast_tx,
        provider_reload,
        provider_reload_inputs,
        parent_control,
        teardown_graph,
    } = args;
    let sessions = sessions(SessionLoopInputs {
        base_dir: base_dir.to_path_buf(),
        store: session_store_override,
        session_key: session_key.clone(),
        ephemeral,
        system_prompt: system_prompt.clone(),
        spill_store: agent.spill_store().cloned(),
        durable_prefix: agent.durable_prefix_latch(),
        workflow_state: workflow_state.clone(),
        subagent_registry: subagent_registry.clone(),
    });
    let session_store: &dyn SessionStore = sessions.store.as_ref();
    let identity = sessions.active_session.read().await.identity().clone();
    // Refuse at open, not at first save (#1460): a key owned by another
    // live process must fail before any turn runs against it.
    if !ephemeral
        && !session_key.is_empty()
        && let Err(err) = session_store.claim(&identity)
    {
        eprintln!("{err}");
        return 1;
    }
    let loaded_session = match load_session(session_store, &identity, ephemeral).await {
        Ok(m) => m,
        Err(err) => {
            eprintln!("failed to load session: {err}");
            return 1;
        }
    };
    // What the store holds is the durable prefix the first delta may trust.
    sessions
        .active_session
        .write()
        .await
        .set_persisted_watermark(loaded_session.messages.len());
    let messages = loaded_session.messages;
    if let (Some(ws), Some(persisted)) = (&workflow_state, loaded_session.workflow_run) {
        if let Ok(mut engine) = ws.lock() {
            engine.restore_run(persisted);
        }
    }

    if let Some(std_stream) = socket_override {
        // Single-client path: backward-compatible with existing tests.
        single_client_loop(
            SingleClientArgs {
                agent,
                base_dir,
                workspace,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                subagent_registry,
                workflow_state,
                provider_reload,
                provider_reload_inputs,
            },
            std_stream,
            &sessions,
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
        super::uds_multi::multi_client_loop(
            MultiClientArgs {
                agent,
                base_dir,
                workspace,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                lifetime,
                notification_rx,
                subagent_registry,
                harness_lifecycle,
                workflow_state,
                workflow_config,
                broadcast_tx,
                provider_reload,
                provider_reload_inputs,
                parent_control,
                teardown_graph,
            },
            listener,
            &sessions,
        )
        .await
    }
}

struct SingleClientArgs<'a> {
    agent: AgentLoopImpl,
    base_dir: &'a std::path::Path,
    workspace: &'a std::path::Path,
    messages: Vec<Message>,
    model: String,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
    ext_registry: Option<ExtRegistry>,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
}

async fn single_client_loop(
    args: SingleClientArgs<'_>,
    std_stream: std::os::unix::net::UnixStream,
    sessions: &SessionHandles,
) -> i32 {
    let session_store: &dyn SessionStore = sessions.store.as_ref();
    let SingleClientArgs {
        mut agent,
        base_dir,
        workspace,
        mut messages,
        model,
        session_key,
        ephemeral,
        system_prompt,
        ext_registry,
        subagent_registry,
        workflow_state,
        provider_reload,
        provider_reload_inputs,
    } = args;
    std_stream
        .set_nonblocking(true)
        .expect("set_nonblocking failed for test socket");
    let tokio_stream = tokio::net::UnixStream::from_std(std_stream).expect("std→tokio UnixStream");
    let (r, w) = tokio::io::split(tokio_stream);
    let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
    let mut writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);

    let wire_mode = super::uds_wire::ConnectionWireMode::legacy();
    let workspace_event = super::protocol::AgentEvent::Workspace {
        path: workspace.display().to_string(),
    };
    let line = workspace_event.to_json_line() + "\n";
    let _ = super::uds_wire::write_event_line(&mut writer, &line, &wire_mode).await;

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());
    let max_context_tokens = agent.max_context_tokens();
    let initial_effort = agent.effort().map(|l| l.as_str().to_string());
    let initial_stats = super::uds_session::compute_session_stats(&session_key, &messages);
    let session_reads = sessions.read_handles();
    let _ = session_reads
        .active_session
        .write()
        .await
        .publish(&messages);

    run_command_loop(
        reader,
        &mut DispatchCtx {
            wire_mode,
            base_dir,
            agent: &mut agent,
            messages: &mut messages,
            sessions: session_reads.clone(),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                agent_session.state_snapshot(0, None, max_context_tokens, initial_effort),
            )),
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut agent_session,
            stdout: Some(&mut *writer),
            session_store,
            ephemeral,
            system_prompt: &system_prompt,
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::<TurnControl>::default(),
            broadcast_tx: None,
            _ext_registry: ext_registry,
            client_tool_registry: super::uds_ext_protocol::new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: subagent_registry.clone(),
            notification_rx: None,
            workflow_state: workflow_state.clone(),
            workflow_config: None,
            provider_reload,
            provider_reload_inputs,
            fleet_teardown: None,
            list_sessions: sessions.list_sessions.clone(),
            save_session: sessions.save_session.clone(),
            rewrite: sessions.rewrite.clone(),
            switch: sessions.switch.clone(),
        },
    )
    .await;

    // The final save of an ordinary exit (#1860): the transaction decides
    // whether there is anything to save.
    if let Err(err) = sessions
        .save_session
        .save(&mut messages, SaveTrigger::OrdinaryExit)
        .await
    {
        tracing::warn!("failed to persist session on exit: {err}");
    }
    0
}
