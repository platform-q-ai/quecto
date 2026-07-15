use super::protocol::AgentEvent;
use super::uds_multi::ConversationSnapshot;

pub(super) struct BusyCommandCtx<'a> {
    pub line: &'a str,
    pub snapshot: &'a ConversationSnapshot,
    pub registry: &'a super::uds_ext_protocol::ClientToolRegistry,
    pub client_id: u64,
}

/// Handle commands that must bypass the dispatch loop while a prompt is active.
pub(super) async fn intercept(ctx: BusyCommandCtx<'_>) -> bool {
    if let Some(result) = super::uds_tool_intercept::try_intercept_tool_result(ctx.line) {
        super::uds_ext_protocol::handle_tool_result(super::uds_ext_protocol::ToolResultArgs {
            client_id: ctx.client_id,
            tool_call_id: &result.tool_call_id,
            content: &result.content,
            is_error: result.is_error,
            registry: ctx.registry,
        });
        return true;
    }
    if let Some(parsed) = parse(ctx.line.trim()) {
        service(parsed, ctx.snapshot, ctx.registry, ctx.client_id).await;
        return true;
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedGetMessage {
    pub(super) request_id: Option<String>,
    pub(super) message_id: String,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

pub(super) async fn service(
    parsed: ParsedGetMessage,
    snapshot: &ConversationSnapshot,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let ParsedGetMessage {
        request_id,
        message_id,
        offset,
        limit,
    } = parsed;
    // Resolve against the id-addressable ledger (full copies) first, falling
    // back to the live snapshot — a ref pruned/collapsed from the live
    // conversation still resolves to its full content (#1060 review 1a).
    let data = super::uds_snapshots::resolve_get_message(snapshot, &message_id)
        .await
        .map(|msg| super::uds_session::message_to_json_range(&msg, offset, limit));
    let event = match data {
        Some(data) => AgentEvent::ok(request_id.as_deref(), "get_message", Some(data)),
        None => AgentEvent::err(
            request_id.as_deref(),
            "get_message",
            format!("message not found: {message_id}"),
        ),
    };
    if let Some(tx) = super::uds_ext_protocol::client_writer_tx(registry, client_id) {
        let mut response = serde_json::to_string(&event).unwrap_or_default();
        // Same frame-cap discipline as the dispatch-loop path (#1062): an
        // over-budget line would be dropped unread by the client's bounded
        // reader, so replace it with a small correlated error instead.
        if response.len() > crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET {
            tracing::warn!(
                len = response.len(),
                cap = crate::infrastructure::line_cap::EVENT_LINE_CAP_BYTES,
                "rejecting oversized busy get_message response"
            );
            response = serde_json::to_string(&AgentEvent::err(
                request_id.as_deref(),
                "get_message",
                "message exceeds the protocol frame limit and cannot be returned whole",
            ))
            .unwrap_or_default();
        }
        response.push('\n');
        let _ = tx.send(response).await;
    }
}

pub(super) fn parse(line: &str) -> Option<ParsedGetMessage> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    // An agent-targeted lookup (`agent_id` present) must fall through to the
    // dispatch loop, which forwards it to the child. The wire key is snake_case
    // `agent_id` (Command::GetMessage has no rename); matching `agentId` here
    // never fired, so the master wrongly served child ids from its own snapshot
    // (#1060 review).
    if value.get("type")?.as_str()? != "get_message" || value.get("agent_id").is_some() {
        return None;
    }
    let message_id = value.get("messageId")?.as_str()?.to_string();
    let request_id = value.get("id").and_then(|v| v.as_str()).map(str::to_string);
    let offset = value
        .get("offset")
        .and_then(|v| v.as_u64())
        .and_then(|n| usize::try_from(n).ok());
    let limit = value
        .get("limit")
        .and_then(|v| v.as_u64())
        .and_then(|n| usize::try_from(n).ok());
    Some(ParsedGetMessage {
        request_id,
        message_id,
        offset,
        limit,
    })
}
