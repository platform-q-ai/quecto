//! A stopped run's transcript (#2173): the calls it was running when it
//! was stopped have no result. Saved like that, the next request pairs a
//! call with nothing, and the model cannot tell whether the call acted.

use std::collections::HashSet;

use crate::domain::message::{Message, Role};

/// Give every tool call of this run without a result an error result that
/// says the run stopped first, placed after the results of its own
/// assistant message. The run starts at the message with id `run_start`;
/// when that message is gone (pruned), at its first message not yet saved
/// (one with no ordinal) — never in history an earlier run saved.
/// Returns how many were added.
pub fn answer_unfinished_tool_calls(
    messages: &mut Vec<Message>,
    run_start: uuid::Uuid,
    reason: &str,
) -> usize {
    // A result anywhere answers its call: pairing is by id, as the
    // providers' orphan filter pairs them.
    let answered: HashSet<String> = messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .filter_map(|message| message.tool_call_id.clone())
        .collect();
    let mut index = messages
        .iter()
        .position(|message| message.id() == run_start)
        .or_else(|| {
            messages
                .iter()
                .position(|message| message.ordinal.is_none())
        })
        .unwrap_or(messages.len());
    let mut added = 0;
    while index < messages.len() {
        let dispatch = &messages[index];
        let unanswered: Vec<Message> = match dispatch.role {
            Role::Assistant => dispatch
                .tool_calls
                .iter()
                .filter(|call| !answered.contains(&call.id))
                .map(|call| stopped_result(call, dispatch.turn, reason))
                .collect(),
            Role::System | Role::User | Role::Tool => Vec::new(),
        };
        // The results that answer this message follow it directly.
        let mut end = index + 1;
        while end < messages.len() && messages[end].role == Role::Tool {
            end += 1;
        }
        let inserted = unanswered.len();
        messages.splice(end..end, unanswered);
        added += inserted;
        index = end + inserted;
    }
    added
}

fn stopped_result(
    call: &crate::domain::message::ToolCall,
    turn: Option<u32>,
    reason: &str,
) -> Message {
    let mut result = Message::tool(
        call.id.clone(),
        format!("{reason} before this call finished; whether it had any effect is unknown"),
    );
    result.tool_name = Some(call.name.clone());
    result.is_error = true;
    result.turn = turn;
    result
}

#[cfg(test)]
#[path = "session_stopped_tests.rs"]
mod tests;
