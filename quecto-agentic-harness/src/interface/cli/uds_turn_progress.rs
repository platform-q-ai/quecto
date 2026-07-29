//! Mid-turn conversation publishing (split from `uds_cancel.rs`, 750-line
//! baseline).

use super::EventSink;
use crate::domain::agent::AgentProgressEvent;

/// Publish the messages of a just-completed INNER turn into the shared
/// conversation snapshot, and emit the resulting `ledger_advanced` hints.
///
/// Emitting is not optional: the TUI child feed re-syncs only on
/// `ledger_advanced`, so a silent snapshot advance freezes a running agent's
/// transcript until the whole prompt finishes (the child-progress-freeze bug;
/// #1283 introduced this publish path without the emissions).
/// `emit_ledger_advanced` no-ops when the advance carries no change, so an
/// unchanged republish emits nothing.
pub(crate) async fn publish_turn_progress(
    event: &AgentProgressEvent,
    snapshot: Option<&super::super::uds_multi::ConversationSnapshot>,
    sink: &mut EventSink<'_>,
) {
    let (Some(snapshot), AgentProgressEvent::TurnCompleted { messages }) = (snapshot, event) else {
        return;
    };
    let mut snap = snapshot.write().await;
    let mut live = snap.messages.clone();
    for message in messages.iter() {
        if !live.iter().any(|existing| existing.id() == message.id()) {
            live.push(message.clone());
        }
    }
    let publish = snap.publish(&live);
    let full = snap.record_full(messages);
    drop(snap);
    sink.emit_ledger_advanced(publish).await;
    sink.emit_ledger_advanced(full).await;
}
