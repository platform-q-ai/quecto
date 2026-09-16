//! The `sync` command (#1857, #1973) on both transports: the reader task's
//! fast path while the dispatch loop is busy ([`intercept`], answering on
//! the client's own writer) and the idle loop's dispatch
//! (`uds_dispatch_query`), both through [`sync_data`] — the one presenter
//! of the composed `SynchronizeTranscriptController`'s typed result. The
//! wire field names, the history frame budget a delta is cut at and the
//! frame-limit refusal are this module's; reset-or-delta and where a cut
//! delta continues are the application's.
use super::protocol::AgentEvent;
use super::uds_session::{
    HISTORY_PAGE_JSON_BUDGET, HISTORY_PAGE_SIZE, history_page_json,
    message_to_json_for_history_page,
};
use super::uds_session_handles::SessionReadHandles;
use crate::application::sessions::dto::TranscriptSync;
use crate::domain::message::Message;
use crate::interface::uds::sessions::synchronize_transcript_controller::{
    SyncFields, SynchronizeTranscriptController,
};

pub(super) const SYNC_OVERSIZED_ERROR: &str =
    "sync response exceeds the protocol frame limit; retry with nextRev to continue";

/// The bytes one message costs in a frame: its bounded history
/// representation plus the separator.
fn encoded_size(message: &Message) -> usize {
    serde_json::to_vec(&message_to_json_for_history_page(message))
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        + 1
}

/// The `sync` response data for a client at `since_rev` of `epoch`.
///
/// A reset is the newest `HISTORY_PAGE_SIZE` messages in the history page
/// shape (`messages`, `before`, `hasMoreBefore`) with `resync: true`,
/// `caughtUp: true` and a null `nextRev`. A delta is the committed
/// messages after `since_rev` that fit the history frame budget, each in
/// its bounded history representation, with `nextRev` naming the revision
/// the delta was cut at (null, `caughtUp: true`, when nothing was left
/// out). The frame predicate measures; the typed delta is the one source
/// of what is carried, encoded here.
pub(super) async fn sync_data(
    controller: &SynchronizeTranscriptController,
    epoch: u64,
    since_rev: u64,
) -> serde_json::Value {
    let mut used = 0usize;
    let carry = |message: &Message| {
        let sz = encoded_size(message);
        if used.saturating_add(sz) > HISTORY_PAGE_JSON_BUDGET {
            return false;
        }
        used = used.saturating_add(sz);
        true
    };
    let outcome = controller
        .sync(epoch, since_rev, HISTORY_PAGE_SIZE, carry)
        .await;
    match outcome {
        TranscriptSync::Reset(reset) => {
            let mut data = history_page_json(reset.page);
            if let Some(obj) = data.as_object_mut() {
                obj.insert("epoch".into(), serde_json::json!(reset.epoch));
                obj.insert("rev".into(), serde_json::json!(reset.rev));
                obj.insert("nextRev".into(), serde_json::Value::Null);
                obj.insert("caughtUp".into(), serde_json::json!(true));
                obj.insert("resync".into(), serde_json::json!(true));
            }
            data
        }
        TranscriptSync::Delta(delta) => serde_json::json!({
            "epoch": delta.epoch,
            "rev": delta.rev,
            "messages": delta
                .messages
                .iter()
                .map(message_to_json_for_history_page)
                .collect::<Vec<_>>(),
            "nextRev": delta.next_rev,
            "caughtUp": delta.caught_up(),
            "resync": false,
        }),
    }
}

/// The line the reader fast path writes for `event`, the `sync` success
/// for `request_id`: the event itself when it fits `frame_limit` bytes,
/// else the structured [`SYNC_OVERSIZED_ERROR`] refusal (never an
/// unstructured frame-limit error). Always newline-terminated.
pub(super) fn bounded_response_line(
    event: &AgentEvent,
    request_id: Option<&str>,
    frame_limit: usize,
) -> String {
    let mut response = serde_json::to_string(event).unwrap_or_default();
    if response.len() > frame_limit {
        response =
            serde_json::to_string(&AgentEvent::err(request_id, "sync", SYNC_OVERSIZED_ERROR))
                .unwrap_or_default();
    }
    response.push('\n');
    response
}

/// The reader task's fast path: answer a parent-local `sync` line on the
/// client's own writer, never queuing behind the dispatch loop. `false`
/// leaves any other line (another command, a child-addressed or malformed
/// `sync`) to its own route.
pub(super) async fn intercept(
    line: &str,
    session: &SessionReadHandles,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) -> bool {
    let Some(fields) = SyncFields::parse_line(line.trim()) else {
        return false;
    };
    let data = sync_data(
        &session.synchronize_transcript,
        fields.epoch,
        fields.since_rev,
    )
    .await;
    let event = AgentEvent::ok(fields.request_id.as_deref(), "sync", Some(data));
    if let Some(tx) = super::uds_ext_protocol::client_writer_tx(registry, client_id) {
        let response = bounded_response_line(
            &event,
            fields.request_id.as_deref(),
            crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        );
        let _ = tx.send(response).await;
    }
    true
}
