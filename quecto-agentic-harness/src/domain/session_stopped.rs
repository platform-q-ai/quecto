//! A stopped run's transcript (#2173): the calls it was running when it
//! was stopped have no result. Saved like that, the next request pairs a
//! call with nothing, and the model cannot tell whether the call acted.

use super::message::{Message, Role};

/// Give every tool call without a result an error result that says the
/// run stopped first, placed after the results of its own assistant
/// message. Returns how many were added.
pub fn answer_unfinished_tool_calls(messages: &mut Vec<Message>, reason: &str) -> usize {
    let mut added = 0;
    let mut index = 0;
    while index < messages.len() {
        let is_dispatch =
            messages[index].role == Role::Assistant && !messages[index].tool_calls.is_empty();
        if !is_dispatch {
            index += 1;
            continue;
        }
        // The results that answer this message follow it directly.
        let mut end = index + 1;
        while end < messages.len() && messages[end].role == Role::Tool {
            end += 1;
        }
        let answered: Vec<&str> = messages[index + 1..end]
            .iter()
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect();
        let unanswered: Vec<Message> = messages[index]
            .tool_calls
            .iter()
            .filter(|call| !answered.contains(&call.id.as_str()))
            .map(|call| {
                let mut result = Message::tool(
                    call.id.clone(),
                    format!(
                        "{reason} before this call finished; whether it had any effect is unknown"
                    ),
                );
                result.tool_name = Some(call.name.clone());
                result.is_error = true;
                result.turn = messages[index].turn;
                result
            })
            .collect();
        added += unanswered.len();
        let inserted = unanswered.len();
        messages.splice(end..end, unanswered);
        index = end + inserted;
    }
    added
}

#[cfg(test)]
#[path = "session_stopped_tests.rs"]
mod tests;
