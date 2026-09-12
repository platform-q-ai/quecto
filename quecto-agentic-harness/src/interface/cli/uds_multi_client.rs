//! Per-client connection handler of the multi-client loop: one writer task
//! multiplexing broadcast and targeted events, one reader loop feeding the
//! dispatch channel. Child module of `uds_multi`.
//!
//! Since #1935 every incoming line first passes the launch-bound parent
//! control / teardown edge (`uds_parent_control`): a presentation may bind
//! this connection as the parent, a teardown command is answered here, and
//! when the reader ends the edge decides whether this was the bound parent
//! connection — the only disconnect that runs the common shutdown.
use std::sync::Arc;

use super::super::uds::MAX_FRAME_PAYLOAD_BYTES;
use super::super::uds_cancel::CancelHandle;
use super::super::uds_parent_control::{
    ConnectionRole, ConnectionTeardown, Intercept, LineContext,
};
use super::super::uds_teardown_adapters::SharedWriter;
use super::{ClientGuard, ClientMessage, ConversationSnapshot};

/// Arguments for [`handle_client`].
pub(crate) struct ClientHandlerArgs {
    pub(crate) stream: tokio::net::UnixStream,
    pub(crate) broadcast_rx: tokio::sync::broadcast::Receiver<String>,
    /// Per-client targeted event stream — receives events addressed
    /// to this client only (currently just `execute_tool` from
    /// forwarder tasks). Writer_task selects over this AND
    /// broadcast_rx so targeted events aren't visible to other
    /// clients.
    pub(crate) targeted_rx: tokio::sync::mpsc::Receiver<String>,
    pub(crate) cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    pub(crate) cancel_handle: CancelHandle,
    /// Shared abort/steer control flags (#895/#896).
    pub(crate) turn_control: super::super::uds_cancel::TurnControlHandle,
    /// Unique client identifier (#352).
    pub(crate) client_id: u64,
    /// For in-reader handling of `tool_result` — see handle_client.
    pub(crate) client_tool_registry: super::super::uds_ext_protocol::ClientToolRegistry,
    /// Live conversation ledger. Updated as messages are appended during a turn,
    /// allowing read-only get_message lookups to bypass the blocked dispatcher.
    pub(crate) conversation_snapshot: ConversationSnapshot,
    /// Sub-agent registry, read mid-turn to serve `get_subagents` and forward
    /// child-targeted `sync` off the blocked dispatcher (spike).
    pub(crate) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// Broadcast sender for busy-path `delete_all_subagents`, which must
    /// publish the empty survivor set without waiting for the dispatcher (#1626).
    pub(crate) broadcast_tx: tokio::sync::broadcast::Sender<String>,
    /// Launch-bound parent control and teardown edge (#1935). `None` only
    /// for test rigs that exercise the plain reader path.
    pub(crate) teardown: Option<Arc<ConnectionTeardown>>,
    /// RAII guard — decrements `live_clients` and sends `Disconnected` on drop.
    pub(crate) _guard: ClientGuard,
}

pub(crate) async fn handle_client(args: ClientHandlerArgs) {
    let ClientHandlerArgs {
        stream,
        mut broadcast_rx,
        mut targeted_rx,
        cmd_tx,
        cancel_handle,
        turn_control,
        client_id,
        client_tool_registry,
        conversation_snapshot,
        subagent_registry,
        broadcast_tx,
        teardown,
        _guard,
    } = args;
    use tokio::io::BufReader;

    let (reader, writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    // The write half is shared with the teardown edge so an ACK is written
    // and flushed in order with the event stream (#1935).
    let writer: SharedWriter = Arc::new(tokio::sync::Mutex::new(writer));

    // Negotiated per-connection framing (#1059): the reader loop below
    // records the client's detected framing; the writer task replies in it.
    // Until the client has spoken, writes fall back to legacy NDJSON — safe
    // because framed clients sniff each incoming message during the
    // deprecation window (see `uds_wire` module docs).
    let wire_mode = super::super::uds_wire::ConnectionWireMode::default();
    let wire_mode_for_writer = wire_mode.clone();
    let writer_for_task = Arc::clone(&writer);

    // Writer task: multiplex shared broadcast events AND per-client
    // targeted events (currently just `execute_tool` from forwarder
    // tasks) onto the client's socket. Targeted events never fan out
    // to other clients.
    let writer_task = tokio::spawn(async move {
        let mode = wire_mode_for_writer;
        loop {
            tokio::select! {
                b = broadcast_rx.recv() => match b {
                    Ok(line) => {
                        if write_line(&writer_for_task, &line, &mode).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("client lagged by {n} events");
                        let msg = format!(
                            "{{\"type\":\"error\",\"message\":\"dropped {} events — use get_messages to re-sync\"}}\n",
                            n
                        );
                        if write_line(&writer_for_task, &msg, &mode).await.is_err() {
                            break;
                        }
                    }
                },
                t = targeted_rx.recv() => match t {
                    Some(line) if write_line(&writer_for_task, &line, &mode).await.is_err() => {
                        break;
                    }
                    Some(_) => {}
                    None => {
                        // Sender side closed (client registry
                        // entry dropped). Fall through — broadcast
                        // may still be delivering.
                    }
                },
            }
        }
    });

    // Reader loop: commands → dispatch mpsc.
    // Each message is sniffed as a length-prefixed frame or a legacy NDJSON
    // line (#1059, deprecation window). Over-cap messages are rejected while
    // reading (bounded memory, #1003) with the connection kept usable; a peer
    // speaking neither framing is an explicit version mismatch and the
    // connection closes — never a silent misparse or a hang.
    let mut role = ConnectionRole::default();
    loop {
        let line = match read_line(&mut reader, &wire_mode, client_id).await {
            Read::Line(line) => line,
            Read::Skip => continue,
            Read::Closed => break,
        };
        if let Some(teardown) = teardown.as_deref() {
            let intercept = super::super::uds_parent_control::intercept_line(
                LineContext {
                    teardown,
                    role: &mut role,
                    writer: &writer,
                    wire_mode: &wire_mode,
                    client_id,
                },
                &line,
            )
            .await;
            match intercept {
                Intercept::NotClaimed => {}
                Intercept::Handled => continue,
                Intercept::Close => break,
            }
        }
        if !super::super::uds_reader_dispatch::dispatch(
            super::super::uds_reader_dispatch::ReaderDispatchCtx {
                line,
                snapshot: &conversation_snapshot,
                registry: &client_tool_registry,
                subagent_registry: &subagent_registry,
                broadcast_tx: &broadcast_tx,
                client_id,
                cmd_tx: &cmd_tx,
                cancel_handle: &cancel_handle,
                turn_control: &turn_control,
            },
        )
        .await
        {
            break;
        }
    }

    writer_task.abort();
    // Only the bound parent's loss runs the common shutdown; every other
    // disconnect — including the last client's — is nothing to the
    // harness's lifetime (#1935).
    if let Some(teardown) = teardown.as_deref() {
        if let Some(outcome) =
            super::super::uds_parent_control::connection_closed(teardown, &role, client_id).await
        {
            tracing::info!(client_id, ?outcome, "parent-loss shutdown finished");
        }
    }
    // `_guard` is dropped here (or on panic), which decrements live_clients
    // and sends the Disconnected sentinel.
}

enum Read {
    Line(String),
    /// Over-cap message rejected, or an empty hello frame: keep reading.
    Skip,
    Closed,
}

/// Read one message in the deprecation-window framing, recording the
/// client's framing so replies use it too.
async fn read_line<R>(
    reader: &mut R,
    wire_mode: &super::super::uds_wire::ConnectionWireMode,
    client_id: u64,
) -> Read
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let incoming =
        match quecto_line_io::read_frame_or_legacy_line(reader, MAX_FRAME_PAYLOAD_BYTES).await {
            Ok(Some(incoming)) => incoming,
            Ok(None) => return Read::Closed,
            Err(e @ quecto_line_io::FrameError::Oversized { .. }) => {
                tracing::warn!(client_id, "dropping over-cap message from client: {e}");
                return Read::Skip;
            }
            Err(e @ quecto_line_io::FrameError::VersionMismatch { .. }) => {
                tracing::warn!(client_id, "closing client connection: {e}");
                return Read::Closed;
            }
            Err(e) => {
                tracing::warn!(client_id, error = %e, "client reader loop exiting on I/O error");
                return Read::Closed;
            }
        };
    let (mode, bytes) = match incoming {
        quecto_line_io::Incoming::Frame(b) => (quecto_line_io::WireMode::Framed, b),
        quecto_line_io::Incoming::LegacyLine(b) => (quecto_line_io::WireMode::LegacyLine, b),
    };
    wire_mode.record(mode);
    // Reuse the payload `Vec`'s allocation on the common valid-UTF-8 path;
    // only pay a copy for the lossy fallback on malformed input.
    let line = String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    // The monitor's framed hello is an empty frame: not a command.
    if line.is_empty() {
        return Read::Skip;
    }
    Read::Line(line)
}

async fn write_line(
    writer: &SharedWriter,
    line: &str,
    mode: &super::super::uds_wire::ConnectionWireMode,
) -> Result<(), quecto_line_io::FrameError> {
    let mut writer = writer.lock().await;
    super::super::uds_wire::write_event_line(&mut *writer, line, mode).await
}
