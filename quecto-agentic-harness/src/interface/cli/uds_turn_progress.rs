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
    session: Option<&crate::application::sessions::active_session::ActiveSessionHandle>,
    sink: &mut EventSink<'_>,
) {
    let (Some(session), AgentProgressEvent::TurnCompleted { messages }) = (session, event) else {
        return;
    };
    let mut state = session.write().await;
    let mut live = state.conversation().live_messages().to_vec();
    for message in messages.iter() {
        if !live.iter().any(|existing| existing.id() == message.id()) {
            live.push(message.clone());
        }
    }
    let publish = state.publish(&live);
    let full = state.record_full(messages);
    drop(state);
    sink.emit_ledger_advanced(publish).await;
    sink.emit_ledger_advanced(full).await;
}
