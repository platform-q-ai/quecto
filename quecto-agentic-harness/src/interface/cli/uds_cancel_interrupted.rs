//! The end of a turn that did not complete (#2218): its interrupted
//! history is finalized, saved by the loop's routine save and published,
//! and the routine save a completed turn runs before its `agent_end`. The
//! ledger's full copies of a turn's messages are taken before the save
//! stamps the live ones, so every turn end restamps them (#2235 review).
use super::{EventSink, Message, update_execution, user_visible_messages};

/// An interrupted turn's history, finalized, saved (#2218) and published.
pub(super) struct InterruptedTurn<'a, 's> {
    pub(super) messages: &'a mut Vec<Message>,
    pub(super) prompt_id: uuid::Uuid,
    pub(super) turn_save: Option<&'a crate::interface::cli::uds::TurnSave>,
    pub(super) active_session:
        Option<&'a crate::application::sessions::active_session::ActiveSessionHandle>,
    pub(super) execution_state:
        &'a Option<crate::interface::cli::uds_execution_state::ExecutionStateHandle>,
    pub(super) system_prompt: &'a str,
    pub(super) sink: &'a mut EventSink<'s>,
}

pub(super) async fn settle_interrupted_turn(turn: InterruptedTurn<'_, '_>) {
    let InterruptedTurn {
        messages,
        prompt_id,
        turn_save,
        active_session,
        execution_state,
        system_prompt,
        sink,
    } = turn;
    let finalized =
        crate::interface::cli::uds_cancel_history::finalize_interrupted_turn(messages, prompt_id);
    crate::domain::turn_origin::stamp_turn(messages, prompt_id); // #2226
    save_turn(turn_save, messages).await;
    if let Some(session) = active_session {
        let visible = user_visible_messages(messages, system_prompt);
        let mut state = session.write().await;
        let publish = state.publish(&visible);
        let full = state.record_full(&finalized.recordable_messages());
        restamp_ledger(&mut state, messages);
        drop(state);
        sink.emit_ledger_advanced(publish).await;
        sink.emit_ledger_advanced(full).await;
    }
    update_execution(execution_state, |state| {
        state.set_message_count(user_visible_messages(messages, system_prompt).len());
    });
}

/// Run the loop's routine save for a turn that ran (#2218).
pub(super) async fn save_turn(
    turn_save: Option<&crate::interface::cli::uds::TurnSave>,
    messages: &mut Vec<Message>,
) {
    if let Some(save) = turn_save {
        save.persist(messages).await;
    }
}

/// Give every full copy the ledger holds of a live message the durable
/// ordinal the save stamped on it and its turn origin (#2226), so
/// `get_message` by id serves what `get_messages` does (#2235 review). The
/// copies stay full; only their stamps change.
pub(super) fn restamp_ledger(
    state: &mut crate::application::sessions::active_session::ActiveSessionState,
    live: &[Message],
) {
    let restamped: Vec<Message> = live
        .iter()
        .filter_map(|message| {
            let ordinal = message.ordinal?;
            let copy = state.conversation().lookup(&message.id().to_string())?;
            let stale = (copy.ordinal, copy.turn_origin) != (Some(ordinal), message.turn_origin);
            stale.then(|| {
                let mut copy = copy.clone();
                copy.ordinal = Some(ordinal);
                copy.turn_origin = message.turn_origin;
                copy
            })
        })
        .collect();
    let _ = state.record_full(&restamped);
}
