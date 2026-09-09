//! Correlated subagent UDS transport and whole-operation inspection deadlines.

use super::subagent_snapshot;

/// Maximum wall-clock time to wait for a forwarded sub-agent UDS response on the
/// `agent_cmd` path (a tool call that may legitimately wait on a long operation).
const SUBAGENT_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Short, interactive-scale timeout for forwards driven by the TUI inspector's
/// 1s poll loop (#795). A `get_messages_tail` query answers from history almost
/// instantly; capping at a few seconds keeps a slow/hung sub-agent from
/// head-of-line-blocking the parent's shared dispatch loop (review: DoS / perf).
pub const INSPECTOR_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-line cap on a sub-agent's UDS reply (#795 security review). Mirrors the
/// inbound client cap (`uds::MAX_FRAME_PAYLOAD_BYTES`) so a misbehaving/compromised
/// sub-agent cannot return an unbounded line and exhaust the parent's memory.
const SUBAGENT_RESPONSE_MAX_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

/// Send a framed JSON command to a sub-agent's UDS socket and read back the
/// `response` message that matches the command we sent.
///
/// Each call opens a new connection, stamps a unique `id` on the command, writes
/// one frame, and reads messages until the `{"type":"response","id":<that id>,...}`
/// event arrives (skipping tokens, agent_start, and unsolicited responses such as
/// the connect-time `get_messages` snapshot — built with no id — that the
/// broadcast delivers to all clients).
/// Shared by
/// `agent_cmd` and the TUI agent-targeted tail forwarder so the framing rule
/// lives in one place (#795).
///
/// Uses the long [`SUBAGENT_RESPONSE_TIMEOUT`]; interactive callers that must
/// not block the shared dispatch loop should use
/// [`send_subagent_uds_command_with_timeout`] with [`INSPECTOR_RESPONSE_TIMEOUT`].
pub async fn send_subagent_uds_command(
    socket_path: &std::path::Path,
    command: &str,
) -> Result<String, crate::domain::error::DomainError> {
    send_subagent_uds_command_inner(socket_path, command, SUBAGENT_RESPONSE_TIMEOUT).await
}

/// Like [`send_subagent_uds_command`] but with an explicit whole-operation deadline
/// covering connection, request writing, and response reading, so
/// interactive callers (the TUI inspector poll, #795) can cap head-of-line
/// blocking on the parent's shared command loop.
pub async fn send_subagent_uds_command_with_timeout(
    socket_path: &std::path::Path,
    command: &str,
    response_timeout: std::time::Duration,
) -> Result<String, crate::domain::error::DomainError> {
    with_operation_deadline(
        response_timeout,
        send_subagent_uds_command_inner(socket_path, command, response_timeout),
    )
    .await
}

pub(super) async fn with_operation_deadline<T>(
    timeout: std::time::Duration,
    operation: impl std::future::Future<Output = Result<T, crate::domain::error::DomainError>>,
) -> Result<T, crate::domain::error::DomainError> {
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| {
            crate::domain::error::DomainError::Tool(format!(
                "subagent operation timed out after {}ms (connect/write/read deadline)",
                timeout.as_millis()
            ))
        })?
}

async fn send_subagent_uds_command_inner(
    socket_path: &std::path::Path,
    command: &str,
    response_timeout: std::time::Duration,
) -> Result<String, crate::domain::error::DomainError> {
    send_subagent_uds_command_connected(
        socket_path,
        command,
        response_timeout,
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
}

pub(super) async fn send_subagent_uds_command_connected(
    socket_path: &std::path::Path,
    command: &str,
    response_timeout: std::time::Duration,
    connect: impl std::future::Future<Output = std::io::Result<tokio::net::UnixStream>>,
) -> Result<String, crate::domain::error::DomainError> {
    use crate::domain::error::DomainError;
    use tokio::io::BufReader;

    let stream = connect.await.map_err(|e| {
        DomainError::Tool(format!(
            "connect to subagent at {} failed: {e}",
            socket_path.display()
        ))
    })?;

    let (reader, mut writer) = tokio::io::split(stream);

    // Correlate the reply with the command we SENT via a unique request `id`
    // (the protocol's correlation field): we stamp a fresh id and accept the
    // `response` whose `id` echoes it (`AgentEvent::ok(id, ..)`). Unsolicited
    // responses — notably the connect-time `get_messages` SNAPSHOT a BUSY child
    // pushes on every new connection (#828, `id: None`) — carry no id and are
    // skipped here, so a parent no longer consumes that snapshot's FIRST message
    // instead of the real reply (#831). id-matching also disambiguates two
    // responses sharing a command (a `get_messages` request vs. the snapshot).
    // A non-object command can't be stamped, so we fall back to first-response.
    let (outbound, expected_id) = stamp_request_id(command);

    // Parent and child are the same binary, so outbound commands always use
    // ADR-0008 framing. Compatibility remains reader-side and is sniffed for
    // every incoming message; it does not pin or downgrade this writer.
    quecto_line_io::write_frame(
        &mut writer,
        outbound.as_bytes(),
        quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
    )
    .await
    .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    // Do NOT shutdown or drop the write half (#557): in multi-client mode the
    // server's reader loop exits on EOF and aborts the broadcast writer, so the
    // response would never arrive. Keep the write half alive until we're done.
    let _keep_alive = writer;

    let mut reader = BufReader::new(reader);
    let deadline = tokio::time::Instant::now() + response_timeout;
    let timeout_msg = || {
        format!(
            "subagent response timed out ({}s)",
            response_timeout.as_secs()
        )
    };
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(DomainError::Tool(timeout_msg()));
        }
        let line = tokio::time::timeout(
            remaining,
            read_response_capped(&mut reader, SUBAGENT_RESPONSE_MAX_BYTES),
        )
        .await
        .map_err(|_| DomainError::Tool(timeout_msg()))??;
        match line {
            Some(l) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&l) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("response") {
                        match &expected_id {
                            // We stamped an id on the request: only accept the
                            // response that echoes it, skipping unsolicited
                            // responses (the connect-time snapshot, which carries
                            // no id, and any other interleaved reply).
                            Some(expected) => {
                                if json.get("id").and_then(|v| v.as_str()) == Some(expected) {
                                    return Ok(l);
                                }
                                if subagent_snapshot::response_is_valid_answer(&json, command) {
                                    // Accept the id-less snapshot, applying the
                                    // request's `count` locally (last-N tail, #842).
                                    return Ok(subagent_snapshot::finalize_snapshot_answer(
                                        l, json, command,
                                    ));
                                }
                                // Unsolicited / mismatched response — skip.
                            }
                            // Command wasn't a JSON object we could stamp: fall
                            // back to historical behaviour (first response).
                            None => return Ok(l),
                        }
                    }
                }
                // Not a response event — skip.
            }
            None => {
                return Err(DomainError::Tool(
                    "subagent closed connection without sending a response".into(),
                ));
            }
        }
    }
}

/// Stamp a unique correlation `id` onto an outbound UDS command so the read loop
/// can match the reply by its echoed `id` field, skipping unsolicited responses
/// such as the connect-time `get_messages` snapshot (which carries no id) (#831).
///
/// Returns the (possibly rewritten) command line to send and the id to match on.
/// When the command is not a JSON object we cannot stamp an id, so the original
/// command is returned with `None`, signalling the caller to fall back to
/// first-response behaviour. Any pre-existing `id` is overwritten so two callers
/// reusing the same literal command can never collide.
pub(super) fn stamp_request_id(command: &str) -> (String, Option<String>) {
    match serde_json::from_str::<serde_json::Value>(command) {
        Ok(serde_json::Value::Object(mut map)) => {
            let id = uuid::Uuid::new_v4().to_string();
            map.insert("id".to_owned(), serde_json::Value::String(id.clone()));
            (serde_json::Value::Object(map).to_string(), Some(id))
        }
        _ => (command.to_owned(), None),
    }
}

/// Read the next sub-agent message, capping (rather than buffering) any message
/// that exceeds `max_bytes` (#795 security review) so a sub-agent cannot OOM the
/// parent with one giant message, while still allowing an unbounded number of
/// normal-sized messages to be skipped before the `response` event arrives.
///
/// Delegates the sniff-and-cap framing to the shared
/// [`quecto_line_io::read_frame_or_legacy_line`] helper (#1059) so this
/// parent→child consumer shares the same length-prefixed-frame / legacy-NDJSON
/// deprecation-window handling as the other four UDS consumers; over-cap
/// messages are skipped (not hard-errored) here.
pub(super) async fn read_response_capped<R>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, crate::domain::error::DomainError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use crate::domain::error::DomainError;
    use quecto_line_io::{FrameError, Incoming};

    // Deprecation-window reader (#1059): each reply is sniffed as a
    // length-prefixed frame or a legacy NDJSON line. An over-cap interleaved
    // message is skipped (the declared/until-newline bytes were consumed, so
    // the stream stays framed) rather than hard-erroring the whole query — the
    // same skip-and-continue the other four consumers use, replacing the old
    // "response line exceeded size limit" abort. An unknown first byte is an
    // explicit `VersionMismatch`, never a silent misparse.
    loop {
        match quecto_line_io::read_frame_or_legacy_line(reader, max_bytes).await {
            Ok(None) => return Ok(None),
            Ok(Some(incoming)) => {
                let bytes = match incoming {
                    Incoming::Frame(b) | Incoming::LegacyLine(b) => b,
                };
                return Ok(Some(String::from_utf8(bytes).unwrap_or_else(|e| {
                    String::from_utf8_lossy(e.as_bytes()).into_owned()
                })));
            }
            Err(FrameError::Oversized { .. }) => continue,
            Err(e) => {
                return Err(DomainError::Tool(format!("read from subagent failed: {e}")));
            }
        }
    }
}
