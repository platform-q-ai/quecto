use super::*;
use crate::application::session_payloads::ResumedChatMessage;

impl App {
    pub(super) fn resumed_chat_entries(messages: Vec<ResumedChatMessage>) -> Vec<ChatEntry> {
        let mut entries = Vec::new();
        let mut tools = std::collections::HashMap::<String, usize>::new();
        for message in messages {
            match message {
                ResumedChatMessage::User { text, id, stub } => entries.push(Self::history_entry(
                    crate::interface::ansi::sanitize_control_keep_newlines(&text),
                    id,
                    stub,
                    true,
                )),
                ResumedChatMessage::Assistant { text, id, stub } => {
                    entries.push(Self::history_entry(
                        crate::interface::ansi::sanitize_control_keep_newlines(&text),
                        id,
                        stub,
                        false,
                    ))
                }
                ResumedChatMessage::ToolCall {
                    tool_call_id,
                    tool_name,
                    args,
                } => {
                    tools.insert(tool_call_id.clone(), entries.len());
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id,
                        tool_name,
                        parsed_args: serde_json::from_str(&args).ok(),
                        args,
                        result: None,
                        is_error: false,
                        duration_ms: None,
                    });
                }
                ResumedChatMessage::ToolResult {
                    tool_call_id,
                    tool_name,
                    content,
                    is_error,
                } => Self::attach_resumed_tool_result(
                    &mut entries,
                    &tools,
                    tool_call_id,
                    tool_name,
                    content,
                    is_error,
                ),
            }
        }
        entries
    }

    fn attach_resumed_tool_result(
        entries: &mut Vec<ChatEntry>,
        tools: &std::collections::HashMap<String, usize>,
        tool_call_id: String,
        tool_name: Option<String>,
        content: String,
        is_error: bool,
    ) {
        let content = crate::interface::ansi::sanitize_control_keep_newlines(&content);
        if let Some(idx) = tools.get(&tool_call_id).copied()
            && let Some(ChatEntry::ToolExecution {
                result,
                is_error: err,
                ..
            }) = entries.get_mut(idx)
        {
            *result = Some(content);
            *err = is_error;
            return;
        }
        entries.push(ChatEntry::ToolExecution {
            tool_call_id,
            tool_name: tool_name.unwrap_or_else(|| "tool".to_string()),
            parsed_args: None,
            args: String::new(),
            result: Some(content),
            is_error,
            duration_ms: None,
        });
    }
}
