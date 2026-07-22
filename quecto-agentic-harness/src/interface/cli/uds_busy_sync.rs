use super::protocol::AgentEvent;
use super::uds_multi::ConversationSnapshot;

pub(super) const SYNC_OVERSIZED_ERROR: &str =
    "sync response exceeds the protocol frame limit; retry with nextRev to continue";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedSync {
    pub request_id: Option<String>,
    pub epoch: u64,
    pub since_rev: u64,
}

pub(super) async fn intercept(
    line: &str,
    snapshot: &ConversationSnapshot,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) -> bool {
    let Some(parsed) = parse(line.trim()) else {
        return false;
    };
    service(parsed, snapshot, registry, client_id).await;
    true
}

pub(super) async fn service(
    parsed: ParsedSync,
    snapshot: &ConversationSnapshot,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let data = snapshot
        .read()
        .await
        .sync_json(parsed.epoch, parsed.since_rev);
    let event = AgentEvent::ok(parsed.request_id.as_deref(), "sync", Some(data));
    if let Some(tx) = super::uds_ext_protocol::client_writer_tx(registry, client_id) {
        let mut response = serde_json::to_string(&event).unwrap_or_default();
        if response.len() > crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET {
            response = serde_json::to_string(&AgentEvent::err(
                parsed.request_id.as_deref(),
                "sync",
                SYNC_OVERSIZED_ERROR,
            ))
            .unwrap_or_default();
        }
        response.push('\n');
        let _ = tx.send(response).await;
    }
}

pub(super) fn parse(line: &str) -> Option<ParsedSync> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "sync" || value.get("agent_id").is_some() {
        return None;
    }
    Some(ParsedSync {
        request_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        epoch: value.get("epoch")?.as_u64()?,
        since_rev: value.get("sinceRev")?.as_u64()?,
    })
}
