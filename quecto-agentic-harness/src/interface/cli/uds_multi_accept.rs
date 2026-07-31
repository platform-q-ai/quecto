use super::uds_cancel::CancelHandle;
use super::uds_multi::{
    BusyFlag, ClientGuard, ClientHandlerArgs, ClientMessage, ConversationSnapshot, StateSnapshot,
    handle_client,
};

use super::uds_multi::{MAX_CLIENTS, NEXT_CLIENT_ID};

/// Arguments for [`spawn_accept_loop`].
pub(super) struct AcceptLoopArgs {
    pub(super) listener: tokio::net::UnixListener,
    pub(super) broadcast_tx: tokio::sync::broadcast::Sender<String>,
    pub(super) cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    pub(super) cancel_handle: CancelHandle,
    pub(super) turn_control: super::uds_cancel::TurnControlHandle,
    pub(super) live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    pub(super) client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
    pub(super) conversation_snapshot: ConversationSnapshot,
    pub(super) state_snapshot: StateSnapshot,
    pub(super) execution_state: super::uds_execution_state::ExecutionStateHandle,
    pub(super) session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot,
    pub(super) tool_catalogue_snapshot: super::uds_extensions::ToolCatalogueSnapshot,
    pub(super) busy: BusyFlag,
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// Live workflow engine (#914): read mid-turn so a busy `get_state` reports
    /// current step progress, not the turn-boundary frozen snapshot.
    pub(super) workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    pub(super) workspace_path: std::path::PathBuf,
}

/// Spawn the accept loop that listens for new client connections.
pub(super) fn spawn_accept_loop(args: AcceptLoopArgs) -> tokio::task::JoinHandle<()> {
    let AcceptLoopArgs {
        listener,
        broadcast_tx,
        cmd_tx,
        cancel_handle,
        turn_control,
        live_clients,
        client_tool_registry,
        conversation_snapshot,
        state_snapshot,
        execution_state,
        session_stats_snapshot,
        tool_catalogue_snapshot,
        busy,
        subagent_registry,
        workflow_state,
        workspace_path,
    } = args;
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((mut stream, _addr)) => {
                    let current = live_clients.load(std::sync::atomic::Ordering::SeqCst);
                    if current >= MAX_CLIENTS {
                        tracing::warn!(
                            current,
                            max = MAX_CLIENTS,
                            "rejecting connection: max clients reached"
                        );
                        drop(stream);
                        continue;
                    }
                    live_clients.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let client_id =
                        NEXT_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let guard = ClientGuard {
                        live_clients: live_clients.clone(),
                        cmd_tx: cmd_tx.clone(),
                        client_id,
                    };
                    let (targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(64);
                    super::uds_ext_protocol::register_client_writer(
                        &client_tool_registry,
                        client_id,
                        targeted_tx,
                    );

                    let workspace_line = super::protocol::AgentEvent::Workspace {
                        path: workspace_path.display().to_string(),
                    }
                    .to_json_line()
                        + "\n";
                    if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut stream, workspace_line.as_bytes()).await {
                        tracing::warn!(error = %e, "failed to write workspace announcement");
                        drop(stream);
                        continue;
                    }

                    let broadcast_rx = broadcast_tx.subscribe();
                    // The busy-connect snapshot is pushed BEFORE the client
                    // has spoken, i.e. before its framing is negotiated, so
                    // it is written as legacy NDJSON for the deprecation
                    // window — framed clients sniff each incoming message
                    // (#1059 / ADR-0008 part 1).
                    if busy.load(std::sync::atomic::Ordering::SeqCst) {
                        use tokio::io::AsyncWriteExt;
                        let snapshot_lines = super::uds_snapshots::busy_connect_snapshot_lines(
                            super::uds_snapshots::BusySnapshotSources {
                                state: &state_snapshot,
                                conversation: &conversation_snapshot,
                                session_stats: &session_stats_snapshot,
                                tool_catalogue: &tool_catalogue_snapshot,
                                subagents: &subagent_registry,
                                workflow: &workflow_state,
                                execution: &execution_state,
                            },
                        )
                        .await;
                        for line in snapshot_lines {
                            if let Err(e) = stream.write_all(line.as_bytes()).await {
                                tracing::debug!("connect-time snapshot not delivered: {e}");
                            }
                        }
                    }

                    let args = ClientHandlerArgs {
                        stream,
                        broadcast_rx,
                        targeted_rx,
                        cmd_tx: cmd_tx.clone(),
                        cancel_handle: cancel_handle.clone(),
                        turn_control: turn_control.clone(),
                        client_id,
                        client_tool_registry: client_tool_registry.clone(),
                        conversation_snapshot: conversation_snapshot.clone(),
                        subagent_registry: subagent_registry.clone(),
                        _guard: guard,
                    };
                    tokio::spawn(async move { handle_client(args).await });
                }
                Err(e) => {
                    tracing::warn!("accept error: {e}");
                    break;
                }
            }
        }
    })
}
