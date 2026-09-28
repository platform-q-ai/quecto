//! The stream vocabulary's own rules (#2285).

use serde_json::json;

use super::*;

fn init(statuses: &[(&str, &str)], tools: &[&str]) -> InitEvent {
    InitEvent {
        tools: tools.iter().map(|t| t.to_string()).collect(),
        mcp_servers: statuses
            .iter()
            .map(|(name, status)| McpServerStatus {
                name: name.to_string(),
                status: status.to_string(),
            })
            .collect(),
        ..InitEvent::default()
    }
}

// Mapping row: `system/init` → launch check that
// `mcp_servers[].status == "connected"` and the tools list is exact.
#[test]
fn the_launch_check_needs_every_mcp_server_connected() {
    assert!(init(&[("board", "connected")], &[]).mcp_servers_connected());
    assert!(init(&[], &[]).mcp_servers_connected());
    assert!(!init(&[("board", "connected"), ("quecto", "failed")], &[]).mcp_servers_connected());
    assert!(!init(&[("board", "pending")], &[]).mcp_servers_connected());
}

#[test]
fn the_launch_check_needs_the_exact_tool_list() {
    let session = init(&[], &["Bash", "Read", "mcp__board__board_claim"]);
    assert!(session.tools_are_exactly(&["mcp__board__board_claim", "Read", "Bash"]));
    assert!(!session.tools_are_exactly(&["Bash", "Read"]));
    assert!(!session.tools_are_exactly(&["Bash", "Read", "mcp__board__board_claim", "WebFetch"]));
}

fn tool_result(content: serde_json::Value) -> ToolResultEvent {
    ToolResultEvent {
        tool_use_id: Some("toolu_1".into()),
        content,
        is_error: false,
        permission_denied: false,
    }
}

#[test]
fn tool_result_text_reads_a_string_or_text_blocks() {
    assert_eq!(
        tool_result(json!("Hello, World!")).content_text(),
        "Hello, World!"
    );
    assert_eq!(
        tool_result(json!([
            {"type": "text", "text": "{\"sent\": "},
            {"type": "image", "source": {}},
            {"type": "text", "text": "true}"}
        ]))
        .content_text(),
        "{\"sent\": true}"
    );
    assert_eq!(tool_result(json!(null)).content_text(), "");
}

// Mapping row: `rate_limit_event.rate_limit_info` (`status`).
#[test]
fn only_an_allowed_rate_limit_status_needs_no_warning() {
    assert_eq!(RateLimitStatus::parse("allowed"), RateLimitStatus::Allowed);
    assert!(!RateLimitStatus::parse("allowed").warrants_warning());
    assert!(RateLimitStatus::parse("allowed_warning").warrants_warning());
    assert!(RateLimitStatus::parse("rejected").warrants_warning());
    assert_eq!(
        RateLimitStatus::parse("throttled"),
        RateLimitStatus::Other("throttled".into())
    );
    assert!(RateLimitStatus::parse("throttled").warrants_warning());
}
