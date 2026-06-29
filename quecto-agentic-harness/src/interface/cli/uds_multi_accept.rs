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
    pub(super) session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot,
    pub(super) extension_snapshot: super::uds_extensions::ExtensionSnapshot,
    pub(super) busy: BusyFlag,
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
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
        session_stats_snapshot,
        extension_snapshot,
        busy,
        subagent_registry,
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

                    let broadcast_rx = broadcast_tx.subscribe();
                    if busy.load(std::sync::atomic::Ordering::SeqCst) {
                        use tokio::io::AsyncWriteExt;
                        let snapshot_lines = super::uds_snapshots::busy_connect_snapshot_lines(
                            &state_snapshot,
                            &conversation_snapshot,
                            &session_stats_snapshot,
                            &extension_snapshot,
                            &subagent_registry,
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
