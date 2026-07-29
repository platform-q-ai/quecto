use std::collections::HashSet;

use crate::domain::message::{Message, Role};

fn prompt_position(messages: &[Message], prompt_id: uuid::Uuid) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.id() == prompt_id)
}

pub(super) struct FinalizedInterruptedTurn {
    pub retained_tail: Vec<Message>,
    pub synthetic_results: Vec<Message>,
}

impl FinalizedInterruptedTurn {
    pub fn recordable_messages(&self) -> Vec<Message> {
        self.retained_tail
            .iter()
            .chain(self.synthetic_results.iter())
            .cloned()
            .collect()
    }
}

pub(super) fn finalize_interrupted_turn(
    messages: &mut Vec<Message>,
    prompt_id: uuid::Uuid,
) -> FinalizedInterruptedTurn {
    let Some(index) = prompt_position(messages, prompt_id) else {
        return FinalizedInterruptedTurn {
            retained_tail: Vec::new(),
            synthetic_results: Vec::new(),
        };
    };

    let interrupted_tail = messages.split_off(index + 1);
    let mut retained_tail = Vec::new();
    for mut message in interrupted_tail {
        match message.role {
            Role::Assistant if !message.tool_calls.is_empty() => {
                message.content.clear();
                message.thinking_blocks.clear();
                message.invalidate_token_cache();
                retained_tail.push(message);
            }
            Role::Tool => retained_tail.push(message),
            _ => {}
        }
    }

    let mut answered = HashSet::new();
    for message in &retained_tail {
        if matches!(message.role, Role::Tool) {
            if let Some(id) = &message.tool_call_id {
                answered.insert(id.clone());
            }
        }
    }

    let mut synthetic_results = Vec::new();
    for message in &retained_tail {
        if matches!(message.role, Role::Assistant) {
            for call in &message.tool_calls {
                if !answered.contains(&call.id) {
                    let mut result = Message::tool(call.id.clone(), "aborted by user");
                    result.is_error = true;
                    result.tool_name = Some(call.name.clone());
                    synthetic_results.push(result);
                    answered.insert(call.id.clone());
                }
            }
        }
    }

    let finalized = FinalizedInterruptedTurn {
        retained_tail,
        synthetic_results,
    };
    messages.extend(finalized.recordable_messages());
    finalized
}
