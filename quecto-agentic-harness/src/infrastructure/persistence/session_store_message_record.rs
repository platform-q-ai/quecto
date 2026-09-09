use super::*;
pub(super) fn message_to_record(msg: &Message) -> MessageRecord {
    MessageRecord {
        ordinal: msg.ordinal,
        role: msg.role.as_str().to_string(),
        content: msg.content.clone(),
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect(),
        tool_call_id: msg.tool_call_id.clone(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        tool_name: msg.tool_name.clone(),
        input_preview: msg.input_preview.clone(),
        spill_id: msg.spill_id.clone(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(|tb| match tb {
                ThinkingBlock::Normal {
                    thinking,
                    signature,
                } => ThinkingBlockRecord::Normal {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                },
                ThinkingBlock::Redacted { data } => {
                    ThinkingBlockRecord::Redacted { data: data.clone() }
                }
            })
            .collect(),
    }
}
