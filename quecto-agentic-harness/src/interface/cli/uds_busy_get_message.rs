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

pub(super) async fn service(
    parsed: (Option<String>, String),
    snapshot: &ConversationSnapshot,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let (request_id, message_id) = parsed;
    // Resolve against the id-addressable ledger (full copies) first, falling
    // back to the live snapshot — a ref pruned/collapsed from the live
    // conversation still resolves to its full content (#1060 review 1a).
    let resolution = snapshot.read().await.resolve_for_get_message(&message_id);
    let data = resolution
        .into_message()
        .await
        .map(|msg| super::uds_session::message_to_json(&msg));
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
        response.push('\n');
        let _ = tx.send(response).await;
    }
}

pub(super) fn parse(line: &str) -> Option<(Option<String>, String)> {
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
    Some((request_id, message_id))
}
