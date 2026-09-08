use super::*;

pub(super) fn role_to_str(role: &Role) -> &str {
    role.as_str()
}

pub(super) fn str_to_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Extract the raw title datum: the session's first user message, trimmed.
/// Returns an empty string when there is none, bounded to a transport-safe
/// length (no ellipsis). Display truncation and the "(untitled)" placeholder
/// are applied by the interface/display layer, not by persistence.
pub(super) fn first_user_message(messages: &[MessageHeader<'_>]) -> String {
    const TRANSPORT_CHAR_CAP: usize = 200;
    messages
        .iter()
        .find(|m| matches!(str_to_role(&m.role), Role::User))
        .map(|m| m.content.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(TRANSPORT_CHAR_CAP).collect())
        .unwrap_or_default()
}

pub(super) fn message_to_record_ref(msg: &Message) -> MessageRecordRef<'_> {
    MessageRecordRef {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role),
        content: &msg.content,
        tool_calls: msg
            .tool_calls
            .iter()
            .map(|tc| ToolCallRecordRef {
                id: &tc.id,
                name: &tc.name,
                arguments: &tc.arguments,
            })
            .collect(),
        tool_call_id: msg.tool_call_id.as_deref(),
        turn: msg.turn,
        is_pinned: Some(msg.is_pinned),
        is_manifest: msg.is_manifest,
        is_collapsed: msg.is_collapsed,
        tool_name: msg.tool_name.as_deref(),
        input_preview: msg.input_preview.as_deref(),
        spill_id: msg.spill_id.as_deref(),
        is_error: msg.is_error,
        stop_reason: msg.stop_reason.as_ref().map(|sr| sr.to_string()),
        thinking_blocks: msg
            .thinking_blocks
            .iter()
            .map(|tb| match tb {
                ThinkingBlock::Normal {
                    thinking,
                    signature,
                } => ThinkingBlockRecordRef::Normal {
                    thinking,
                    signature,
                },
                ThinkingBlock::Redacted { data } => ThinkingBlockRecordRef::Redacted { data },
            })
            .collect(),
    }
}

pub(super) fn message_to_record(msg: &Message) -> MessageRecord {
    MessageRecord {
        ordinal: msg.ordinal,
        role: role_to_str(&msg.role).to_string(),
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

pub(super) fn record_to_message(rec: MessageRecord) -> Message {
    let role = str_to_role(&rec.role);
    let tool_calls = rec
        .tool_calls
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
        })
        .collect();
    let mut msg = match role {
        Role::System => Message::system(rec.content),
        Role::User => Message::user(rec.content),
        Role::Assistant => Message::assistant(rec.content, tool_calls),
        Role::Tool => Message::tool(rec.tool_call_id.unwrap_or_default(), rec.content),
    };
    msg.ordinal = rec.ordinal;
    msg.turn = rec.turn;
    msg.is_manifest = rec.is_manifest;
    msg.is_collapsed = rec.is_collapsed;
    msg.tool_name = rec.tool_name;
    msg.input_preview = rec.input_preview;
    msg.spill_id = rec.spill_id;
    msg.is_error = rec.is_error;
    msg.stop_reason = rec.stop_reason.as_deref().map(StopReason::parse);
    if let Some(pinned) = rec.is_pinned {
        msg.is_pinned = pinned;
    }
    msg.thinking_blocks = rec
        .thinking_blocks
        .into_iter()
        .map(|tb| match tb {
            ThinkingBlockRecord::Normal {
                thinking,
                signature,
            } => ThinkingBlock::Normal {
                thinking,
                signature,
            },
            ThinkingBlockRecord::Redacted { data } => ThinkingBlock::Redacted { data },
        })
        .collect();
    msg
}
