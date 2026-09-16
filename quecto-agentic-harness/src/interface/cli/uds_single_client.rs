//! The single-client UDS loop: one pre-connected stream (tests) served by
//! the same dispatch loop as the multi-client server, minus the accept
//! loop and broadcast. Split out of `uds_lifecycle` (#1845).

use super::uds::{DispatchCtx, run_command_loop};
use super::uds_cancel::{CancelSlot, TurnControl};
use super::uds_lifecycle::{ExtRegistry, inject_system_prompt};
use super::uds_session::AgentSession;
use super::uds_session_handles::SessionHandles;
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::sessions::dto::SaveTrigger;
use crate::domain::message::Message;

pub(super) struct SingleClientArgs<'a> {
    pub(super) agent: AgentLoopImpl,
    pub(super) workspace: &'a std::path::Path,
    pub(super) messages: Vec<Message>,
    pub(super) model: String,
    pub(super) session_key: String,
    pub(super) system_prompt: String,
    pub(super) ext_registry: Option<ExtRegistry>,
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub(super) workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    pub(super) provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub(super) provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
}

pub(super) async fn single_client_loop(
    args: SingleClientArgs<'_>,
    std_stream: std::os::unix::net::UnixStream,
    sessions: &SessionHandles,
    catalogue: &super::catalogue_handles::CatalogueHandles,
) -> i32 {
    let SingleClientArgs {
        mut agent,
        workspace,
        mut messages,
        model,
        session_key,
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

    let mut agent_session = AgentSession::new(model);
    let effort = catalogue.effort_view(agent.effort(), agent_session.model());
    let initial_state =
        agent_session.state_snapshot(&session_key, 0, None, agent.max_context_tokens(), effort);
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
            agent: &mut agent,
            messages: &mut messages,
            sessions: session_reads.clone(),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_state)),
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut agent_session,
            stdout: Some(&mut *writer),
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
            catalogue: catalogue.clone(),
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
