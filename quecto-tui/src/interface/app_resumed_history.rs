use super::*;
use crate::application::session_payloads::ResumedChatMessage;

impl App {
    pub(super) fn resumed_chat_entries(messages: Vec<ResumedChatMessage>) -> Vec<ChatEntry> {
        let mut entries = Vec::new();
        let mut tools = Vec::<(String, usize)>::new();
        let mut suppressed_tools = std::collections::HashSet::<String>::new();
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
                    let parsed_args = serde_json::from_str(&args).ok();
                    if let Some(value) = parsed_args.as_ref()
                        && super::app_events::suppress_tool_box(&tool_name, value)
                    {
                        suppressed_tools.insert(tool_call_id);
                        continue;
                    }
                    let tool_name = crate::interface::ansi::sanitize_control(&tool_name);
                    tools.push((tool_call_id.clone(), entries.len()));
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id,
                        tool_name,
                        parsed_args,
                        args: crate::interface::ansi::sanitize_control_keep_newlines(&args),
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
                } => {
                    if !suppressed_tools.contains(&tool_call_id) {
                        Self::attach_resumed_tool_result(
                            &mut entries,
                            &tools,
                            tool_call_id,
                            tool_name,
                            content,
                            is_error,
                        );
                    }
                }
            }
        }
        entries
    }

    fn attach_resumed_tool_result(
        entries: &mut Vec<ChatEntry>,
        tools: &[(String, usize)],
        tool_call_id: String,
        tool_name: Option<String>,
        content: String,
        is_error: bool,
    ) {
        let content = crate::interface::ansi::sanitize_control_keep_newlines(&content);
        let pending_idx = tools.iter().find_map(|(id, idx)| {
            if id == &tool_call_id
                && matches!(
                    entries.get(*idx),
                    Some(ChatEntry::ToolExecution { result: None, .. })
                )
            {
                Some(*idx)
            } else {
                None
            }
        });
        if let Some(idx) = pending_idx
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
            tool_name: tool_name
                .map(|name| crate::interface::ansi::sanitize_control(&name))
                .unwrap_or_else(|| "tool".to_string()),
            parsed_args: None,
            args: String::new(),
            result: Some(content),
            is_error,
            duration_ms: None,
        });
    }
}
