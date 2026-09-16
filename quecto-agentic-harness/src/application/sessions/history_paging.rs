//! The paging rule of the user-visible transcript (#1856, #1973): which
//! messages a window holds, the cursor a client continues from and
//! whether older history remains. Pure over a conversation slice — no
//! state, no ports — so the history and sync use cases share one rule and
//! a unit rig needs no store to exercise either.
use crate::application::sessions::dto::{HistoryError, HistoryPage, HistoryQuery};
use crate::domain::conversation_view::{position_by_id, user_visible_messages};
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

/// The window `query` selects from `conversation`, whose injected system
/// prompt (`injected_prompt`, empty when none) is not part of the visible
/// transcript.
///
/// A cursor must name a message of the conversation, else it is refused
/// as unknown. A cursor naming a message outside the visible transcript
/// selects the newest window. `count: 0` is the empty page and reports no
/// cursor. An explicit `count` above one page keeps the "last N" contract.
pub fn page_of(
    conversation: &[Message],
    injected_prompt: &str,
    query: &HistoryQuery,
) -> Result<HistoryPage, HistoryError> {
    if let Some(cursor) = &query.before
        && position_by_id(conversation, cursor).is_none()
    {
        return Err(HistoryError::UnknownCursor(cursor.clone()));
    }
    Ok(window(
        &user_visible_messages(conversation, injected_prompt),
        query,
    ))
}

/// The newest `count` messages of the visible transcript: the cursorless
/// query, which no cursor can make unknown, so it cannot be refused.
pub fn newest_window(conversation: &[Message], injected_prompt: &str, count: usize) -> HistoryPage {
    window(
        &user_visible_messages(conversation, injected_prompt),
        &HistoryQuery::newest(count),
    )
}

fn window(visible: &[Message], query: &HistoryQuery) -> HistoryPage {
    let end = query
        .before
        .as_ref()
        .and_then(|cursor| position_by_id(visible, cursor))
        .unwrap_or(visible.len());
    let start = end.saturating_sub(query.count);
    let has_more_before = query.count > 0 && start > 0;
    HistoryPage {
        messages: visible[start..end].to_vec(),
        before: has_more_before.then(|| MessageId::from(visible[start].id().to_string())),
        has_more_before,
    }
}

#[cfg(test)]
#[path = "history_paging_tests.rs"]
mod tests;
