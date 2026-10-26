use super::uds_multi::{
    BusyFlag, ClientGuard, ClientHandlerArgs, ClientMessage, StateSnapshot, handle_client,
};
use crate::interface::uds::controllers::uds_cancel::CancelHandle;
use crate::interface::uds::controllers::uds_session_handles::SessionReadHandles;

use super::uds_multi::{MAX_CLIENTS, NEXT_CLIENT_ID};
use crate::interface::cli;
use crate::interface::uds::controllers;

/// Arguments for [`spawn_accept_loop`].
pub(in crate::interface) struct AcceptLoopArgs {
    pub(in crate::interface) listener: tokio::net::UnixListener,
    pub(in crate::interface) broadcast_tx: tokio::sync::broadcast::Sender<String>,
    pub(in crate::interface) cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    pub(in crate::interface) disconnect_tx:
        tokio::sync::mpsc::UnboundedSender<super::uds_multi::ClientDisconnected>,
    pub(in crate::interface) cancel_handle: CancelHandle,
    pub(in crate::interface) turn_control: controllers::uds_cancel::TurnControlHandle,
    pub(in crate::interface) live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    pub(in crate::interface) client_tool_registry:
        controllers::uds_ext_protocol::ClientToolRegistry,
    pub(in crate::interface) session: SessionReadHandles,
    pub(in crate::interface) state_snapshot: StateSnapshot,
    pub(in crate::interface) runtime_store:
        crate::application::provider_runtime::RuntimeSnapshotStore,
    pub(in crate::interface) execution_state:
        crate::interface::uds::presenters::uds_execution_state::ExecutionStateHandle,
    pub(in crate::interface) session_stats_snapshot: cli::uds_snapshots::SessionStatsSnapshot,
    pub(in crate::interface) tool_catalogue_snapshot:
        controllers::uds_extensions::ToolCatalogueSnapshot,
    pub(in crate::interface) busy: BusyFlag,
    pub(in crate::interface) subagent_registry:
        Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
    /// Live workflow engine (#914): read mid-turn so a busy `get_state` reports
    /// current step progress, not the turn-boundary frozen snapshot.
    pub(in crate::interface) workflow_state:
        Option<crate::interface::cli::controllers::shared::WorkflowStateHandle>,
    pub(in crate::interface) workspace_path: std::path::PathBuf,
    /// Launch-bound parent control / teardown edge shared by every
    /// connection (#1935).
    pub(in crate::interface) teardown:
        Option<std::sync::Arc<controllers::uds_parent_control::ConnectionTeardown>>,
    /// This agent's configured extensions, whose connections are claimed
    /// at accept (#2446).
    pub(in crate::interface) extensions: controllers::uds_extensions::Extensions,
    /// The board view's handle (#2478): every connection's
    /// `get_swarm_board`, and the `swarm_board_changed` poll.
    pub(in crate::interface) swarm_boards:
        Option<crate::interface::uds::controllers::uds_swarm_board::SwarmBoards>,
    /// A guarded harness (#2708): its connections answer a busy `get_state`.
    pub(in crate::interface) guarded: bool,
}

/// Spawn the accept loop that listens for new client connections.
pub(in crate::interface) fn spawn_accept_loop(args: AcceptLoopArgs) -> tokio::task::JoinHandle<()> {
    let AcceptLoopArgs {
        listener,
        broadcast_tx,
        cmd_tx,
        disconnect_tx,
        cancel_handle,
        turn_control,
        live_clients,
        client_tool_registry,
        session,
        state_snapshot,
        runtime_store,
        execution_state,
        session_stats_snapshot,
        tool_catalogue_snapshot,
        busy,
        subagent_registry,
        workflow_state,
        workspace_path,
        teardown,
        extensions,
        swarm_boards,
        guarded,
    } = args;
    let busy_state =
        guarded.then(
            || crate::interface::uds::presenters::uds_busy_state::BusyState {
                busy: busy.clone(),
                state: state_snapshot.clone(),
                runtime_store: runtime_store.clone(),
                workflow: workflow_state.clone(),
                execution: execution_state.clone(),
            },
        );
    // A recoverable failure's self-wake (#2531) rides the loop's channel.
    turn_control.bind_self_waker(cmd_tx.downgrade());
    tokio::spawn(async move {
        // The board view's change hints (#2478) live as long as the loop;
        // what they have seen is told to each new connection.
        let board_hints =
            crate::interface::uds::controllers::uds_swarm_board::BoardHints::default();
        let hints = swarm_boards.clone().map(|boards| {
            crate::interface::uds::controllers::uds_swarm_board::spawn_change_hints(
                boards,
                board_hints.clone(),
                broadcast_tx.clone(),
                live_clients.clone(),
            )
        });
        let _hints = crate::interface::uds::controllers::uds_swarm_board::AbortOnDrop(hints);
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
                        disconnect_tx: disconnect_tx.clone(),
                        client_id,
                    };
                    let (targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(64);
                    controllers::uds_ext_protocol::register_client_writer(
                        &client_tool_registry,
                        client_id,
                        targeted_tx.clone(),
                    );
                    controllers::uds_extensions::claim_connection(
                        &extensions,
                        &stream,
                        &client_tool_registry,
                        client_id,
                    );

                    // Subscribed with what the board hints have seen,
                    // under one lock (#2488 round 2 M2).
                    let (broadcast_rx, board_lines) = board_hints.subscribe(&broadcast_tx);
                    let workspace_line = super::super::wire::protocol::AgentEvent::Workspace {
                        path: workspace_path.display().to_string(),
                    }
                    .to_json_line()
                        + "\n";
                    // Connect-time events are pushed BEFORE the client has
                    // spoken, i.e. before its framing is negotiated, so they
                    // are written as legacy NDJSON for the deprecation window
                    // — framed clients sniff each incoming message (#1059 /
                    // ADR-0008 part 1).
                    {
                        use tokio::io::AsyncWriteExt;
                        if let Err(e) = stream.write_all(workspace_line.as_bytes()).await {
                            tracing::debug!("connect-time workspace event not delivered: {e}");
                            continue;
                        }
                    }
                    let is_busy = busy.load(std::sync::atomic::Ordering::SeqCst);
                    {
                        use tokio::io::AsyncWriteExt;
                        // `get_state` is a pure inspector read. Always publish the
                        // current point-in-time projection on connect so callers
                        // never depend on the serialized command loop replying.
                        let mut live = state_snapshot.read().await.clone();
                        crate::interface::uds::presenters::uds_admission_warnings::overlay_current(
                            &mut live,
                            &runtime_store,
                        );
                        let state_line = cli::uds_snapshots::build_connect_get_state_line(
                            &live,
                            &workflow_state,
                            &execution_state,
                            is_busy,
                        );
                        if let Err(e) = stream.write_all(state_line.as_bytes()).await {
                            tracing::debug!("connect-time state snapshot not delivered: {e}");
                            continue;
                        }
                        if is_busy {
                            let snapshot_lines = cli::uds_snapshots::busy_connect_snapshot_lines(
                                cli::uds_snapshots::BusySnapshotSources {
                                    state: &state_snapshot,
                                    session: &session,
                                    session_stats: &session_stats_snapshot,
                                    tool_catalogue: &tool_catalogue_snapshot,
                                    subagents: &subagent_registry,
                                    workflow: &workflow_state,
                                    execution: &execution_state,
                                },
                            )
                            .await;
                            // State was already sent above; publish the other busy
                            // inspector snapshots without duplicating it.
                            for line in snapshot_lines.into_iter().skip(1) {
                                if let Err(e) = stream.write_all(line.as_bytes()).await {
                                    tracing::debug!("connect-time snapshot not delivered: {e}");
                                }
                            }
                        }
                        // Every board already seen (#2488 review M1): the
                        // client is subscribed, so later moves follow.
                        for line in board_lines {
                            if let Err(e) = stream.write_all(line.as_bytes()).await {
                                tracing::debug!("connect-time board hint not delivered: {e}");
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
                        session: session.clone(),
                        subagent_registry: subagent_registry.clone(),
                        teardown: teardown.clone(),
                        swarm_boards: swarm_boards.clone(),
                        busy_state: busy_state.clone(),
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
